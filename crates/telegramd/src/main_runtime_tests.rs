use super::*;

#[test]
fn telegram_api_retry_delay_uses_bounded_exponential_backoff() {
    let seconds = (0..=6)
        .map(|attempt| telegram_api_retry_delay(attempt).as_secs())
        .collect::<Vec<_>>();
    assert_eq!(seconds, vec![5, 10, 20, 40, 60, 60, 60]);
}
