use super::*;
use std::thread;

#[test]
fn closed_allows_until_threshold() {
    let cb = CircuitBreaker::new();
    for _ in 0..(FAILURE_THRESHOLD - 1) {
        assert!(matches!(cb.before_attempt(), AttemptDecision::Allow));
        cb.note_failure();
    }
    // 第 FAILURE_THRESHOLD-1 次失败后，仍处于 Closed
    assert_eq!(cb.snapshot().state, "closed");

    // 第 FAILURE_THRESHOLD 次：触发 Open
    assert!(matches!(cb.before_attempt(), AttemptDecision::Allow));
    cb.note_failure();
    assert_eq!(cb.snapshot().state, "open");
}

#[test]
fn open_skips_until_cooldown_elapses() {
    let cb = CircuitBreaker::new();
    for _ in 0..FAILURE_THRESHOLD {
        cb.note_failure();
    }
    let decision = cb.before_attempt();
    match decision {
        AttemptDecision::SkipCooldown { remaining_ms } => {
            assert!(remaining_ms <= INITIAL_COOLDOWN_MS);
            assert!(remaining_ms > 0);
        }
        other => panic!("expected SkipCooldown, got {other:?}"),
    }
}

#[test]
fn success_resets_counter_and_state() {
    let cb = CircuitBreaker::new();
    cb.note_failure();
    cb.note_failure();
    assert_eq!(cb.snapshot().consecutive_failures, 2);
    cb.note_success();
    let snapshot = cb.snapshot();
    assert_eq!(snapshot.state, "closed");
    assert_eq!(snapshot.consecutive_failures, 0);
    assert_eq!(snapshot.current_cooldown_ms, INITIAL_COOLDOWN_MS);
    assert_eq!(snapshot.remaining_cooldown_ms, 0);
}

#[test]
fn half_open_failure_doubles_cooldown_capped_at_max() {
    let cb = CircuitBreaker::new();
    // 强制把 inner state 调成 HalfOpen，模拟 cooldown 到期后的 before_attempt
    {
        let mut inner = cb.inner.lock().unwrap();
        inner.state = State::HalfOpen;
        inner.consecutive_failures = FAILURE_THRESHOLD;
        inner.current_cooldown_ms = INITIAL_COOLDOWN_MS;
    }
    cb.note_failure();
    let snapshot = cb.snapshot();
    assert_eq!(snapshot.state, "open");
    assert_eq!(snapshot.current_cooldown_ms, INITIAL_COOLDOWN_MS * 2);

    // 反复 HalfOpen→Open，cooldown 翻倍直到封顶
    for _ in 0..20 {
        {
            let mut inner = cb.inner.lock().unwrap();
            inner.state = State::HalfOpen;
        }
        cb.note_failure();
    }
    assert_eq!(cb.snapshot().current_cooldown_ms, MAX_COOLDOWN_MS);
}

#[test]
fn open_transitions_to_half_open_after_cooldown_zero() {
    // 用 0 cooldown 模拟"已到期"
    let cb = CircuitBreaker::new();
    {
        let mut inner = cb.inner.lock().unwrap();
        inner.state = State::Open;
        inner.current_cooldown_ms = 0;
        inner.opened_at = Some(Instant::now());
    }
    // sleep 1ms 确保 elapsed > 0
    thread::sleep(Duration::from_millis(1));
    match cb.before_attempt() {
        AttemptDecision::AllowTrial => {}
        other => panic!("expected AllowTrial after cooldown, got {other:?}"),
    }
    assert_eq!(cb.snapshot().state, "half_open");
}
