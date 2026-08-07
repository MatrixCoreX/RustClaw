#![allow(dead_code)]

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SqliteBusyRetryPolicy {
    pub(crate) max_attempts: u32,
    pub(crate) base_delay: Duration,
    pub(crate) max_delay: Duration,
}

impl Default for SqliteBusyRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(8),
            max_delay: Duration::from_millis(160),
        }
    }
}

pub(crate) fn with_sqlite_busy_retry<T>(
    policy: SqliteBusyRetryPolicy,
    mut operation: impl FnMut() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let attempts = policy.max_attempts.max(1);
    for attempt in 0..attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_sqlite_busy(&error) && attempt + 1 < attempts => {
                // The operation closure has returned, so any transaction it
                // owned has already dropped. Never sleep while holding a DB
                // transaction or while waiting on provider/file/network I/O.
                std::thread::sleep(backoff(policy, attempt));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded retry loop always returns")
}

pub(crate) fn is_sqlite_busy(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<rusqlite::Error>()
            .is_some_and(|sqlite| match sqlite {
                rusqlite::Error::SqliteFailure(code, _) => matches!(
                    code.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ),
                _ => false,
            })
    })
}

fn backoff(policy: SqliteBusyRetryPolicy, attempt: u32) -> Duration {
    let multiplier = 1_u32.checked_shl(attempt.min(8)).unwrap_or(u32::MAX);
    let delay = policy.base_delay.saturating_mul(multiplier);
    let jitter_ms = ((attempt as u64 * 17) + 11) % 13;
    delay
        .saturating_add(Duration::from_millis(jitter_ms))
        .min(policy.max_delay)
}

#[cfg(test)]
#[path = "sqlite_busy_retry_tests.rs"]
mod tests;
