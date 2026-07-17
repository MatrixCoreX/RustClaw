//! Per-provider circuit breaker (Phase 2.1)
//!
//! 目标：在 fallback 循环里避免对一个已知"持续失败"的 provider 反复施压。
//!
//! 状态机：
//! ```text
//!     Closed (正常)
//!       │  连续失败 >= FAILURE_THRESHOLD
//!       ▼
//!     Open (拒绝调用，等待 cooldown)
//!       │  cooldown 到期
//!       ▼
//!     HalfOpen (放行 1 次试探)
//!       │ 成功 → Closed
//!       │ 失败 → Open + cooldown × 2（封顶 MAX_COOLDOWN_MS）
//! ```
//!
//! - 只对**真正访问 provider 的失败**计数（即 [`call_provider_with_retry`] 内部
//!   重试耗尽后仍 Err 的情况）。retry 内部的瞬时失败不算独立失败。
//! - 没用 `tokio::sync::RwLock`：状态切换很轻（4 字段），用 `parking_lot::Mutex`
//!   级别的 `std::sync::Mutex` 即可，且能在非 async 上下文里同步检查。
//! - 默认对所有 provider 适用，**不可关闭**。如未来需要按 provider 关闭，加
//!   `breaker_disabled: bool` 字段即可。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// 连续 N 次失败触发 Open。
const FAILURE_THRESHOLD: u32 = 3;

/// 首次进入 Open 的 cooldown（毫秒）。
const INITIAL_COOLDOWN_MS: u64 = 60_000;

/// cooldown 上限（毫秒）。指数退避封顶，避免坏 provider 永远被屏蔽。
const MAX_COOLDOWN_MS: u64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    Open,
    HalfOpen,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct CircuitBreakerSnapshot {
    pub(crate) state: String,
    pub(crate) consecutive_failures: u32,
    pub(crate) current_cooldown_ms: u64,
    pub(crate) remaining_cooldown_ms: u64,
}

#[derive(Debug)]
struct Inner {
    state: State,
    consecutive_failures: u32,
    /// 当前应该等待的 cooldown 长度（指数退避后的值）。
    current_cooldown_ms: u64,
    /// Open 状态进入时刻；HalfOpen / Closed 时为 None。
    opened_at: Option<Instant>,
}

#[derive(Debug)]
pub(crate) struct CircuitBreaker {
    inner: Mutex<Inner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptDecision {
    /// 允许调用 provider。
    Allow,
    /// 允许调用 provider，但当前是试探调用（HalfOpen）。
    /// 可用于在日志里标记。
    AllowTrial,
    /// 当前 provider 处于 Open 状态且 cooldown 未到期，跳过。
    /// `remaining_ms` 是剩余冷却时间，便于 caller 打日志。
    SkipCooldown { remaining_ms: u64 },
}

impl CircuitBreaker {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: State::Closed,
                consecutive_failures: 0,
                current_cooldown_ms: INITIAL_COOLDOWN_MS,
                opened_at: None,
            }),
        }
    }

    /// 在打 provider 之前调用：决定是否放行。
    /// 这一步可能有副作用：Open 且 cooldown 到期时会切到 HalfOpen。
    pub(crate) fn before_attempt(&self) -> AttemptDecision {
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("circuit breaker mutex poisoned");
        match inner.state {
            State::Closed => AttemptDecision::Allow,
            State::HalfOpen => AttemptDecision::AllowTrial,
            State::Open => {
                let cooldown = Duration::from_millis(inner.current_cooldown_ms);
                let elapsed = inner
                    .opened_at
                    .map(|t| now.saturating_duration_since(t))
                    .unwrap_or(cooldown);
                if elapsed >= cooldown {
                    inner.state = State::HalfOpen;
                    inner.opened_at = None;
                    AttemptDecision::AllowTrial
                } else {
                    let remaining_ms = (cooldown - elapsed).as_millis() as u64;
                    AttemptDecision::SkipCooldown { remaining_ms }
                }
            }
        }
    }

    /// provider 调用成功（或被业务视为 OK，比如内容被清洗后非空）后调用。
    /// 立刻清零失败计数 + 回到 Closed + 重置 cooldown。
    pub(crate) fn note_success(&self) {
        let mut inner = self.inner.lock().expect("circuit breaker mutex poisoned");
        inner.state = State::Closed;
        inner.consecutive_failures = 0;
        inner.current_cooldown_ms = INITIAL_COOLDOWN_MS;
        inner.opened_at = None;
    }

    /// provider 调用失败后调用。
    /// 1) 累加 consecutive_failures；
    /// 2) 若达阈值或处于 HalfOpen 失败，进入 Open；HalfOpen → Open 时 cooldown × 2。
    pub(crate) fn note_failure(&self) {
        let mut inner = self.inner.lock().expect("circuit breaker mutex poisoned");
        inner.consecutive_failures = inner.consecutive_failures.saturating_add(1);
        let was_half_open = inner.state == State::HalfOpen;
        let should_open = was_half_open || inner.consecutive_failures >= FAILURE_THRESHOLD;
        if should_open {
            if was_half_open {
                inner.current_cooldown_ms =
                    (inner.current_cooldown_ms.saturating_mul(2)).min(MAX_COOLDOWN_MS);
            } else if inner.state == State::Closed {
                // 首次 Closed → Open，使用初始 cooldown。
                inner.current_cooldown_ms = INITIAL_COOLDOWN_MS;
            }
            inner.state = State::Open;
            inner.opened_at = Some(Instant::now());
        }
    }

    /// Machine-only runtime snapshot for task telemetry and tests.
    pub(crate) fn snapshot(&self) -> CircuitBreakerSnapshot {
        let now = Instant::now();
        let inner = self.inner.lock().unwrap();
        let remaining_cooldown_ms = if inner.state == State::Open {
            let cooldown = Duration::from_millis(inner.current_cooldown_ms);
            let elapsed = inner
                .opened_at
                .map(|opened_at| now.saturating_duration_since(opened_at))
                .unwrap_or(cooldown);
            cooldown.saturating_sub(elapsed).as_millis() as u64
        } else {
            0
        };
        CircuitBreakerSnapshot {
            state: inner.state.as_str().to_string(),
            consecutive_failures: inner.consecutive_failures,
            current_cooldown_ms: inner.current_cooldown_ms,
            remaining_cooldown_ms,
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "circuit_tests.rs"]
mod tests;
