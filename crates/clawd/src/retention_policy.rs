/// A retention duration may span more seconds than an i64 timestamp can hold.
/// Calculate before clamping: casting days or saturating the subtraction alone
/// can overflow first and accidentally move a cleanup cutoff into the future.
pub(crate) fn cutoff_timestamp(now: i64, days: u64) -> i64 {
    let cutoff = i128::from(now) - i128::from(days) * 86_400;
    cutoff.max(i128::from(i64::MIN)) as i64
}

#[cfg(test)]
#[path = "retention_policy_tests.rs"]
mod tests;
