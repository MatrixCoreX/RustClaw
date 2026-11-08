use super::cutoff_timestamp;

#[test]
fn ordinary_and_zero_days_preserve_exact_cutoffs() {
    for now in [i64::MIN, -100_000, 0, 2_000_000_000, i64::MAX] {
        assert_eq!(cutoff_timestamp(now, 0), now);
    }
    assert_eq!(cutoff_timestamp(2_000_000_000, 1), 1_999_913_600);
    assert_eq!(cutoff_timestamp(0, 1), -86_400);
    assert_eq!(cutoff_timestamp(-100_000, 1), -186_400);
}

#[test]
fn all_u64_durations_match_wide_reference_without_future_cutoffs() {
    let days = [
        0,
        1,
        7,
        3650,
        i64::MAX as u64 / 86_400,
        i64::MAX as u64 / 86_400 + 1,
        i64::MAX as u64,
        i64::MAX as u64 + 1,
        u64::MAX,
    ];
    for now in [
        i64::MIN,
        i64::MIN + 1,
        -86_400,
        -1,
        0,
        1,
        2_000_000_000,
        i64::MAX,
    ] {
        for days in days {
            let exact = i128::from(now) - i128::from(days) * 86_400;
            let expected = i64::try_from(exact).unwrap_or(i64::MIN);
            let actual = cutoff_timestamp(now, days);
            assert_eq!(actual, expected, "now={now}, days={days}");
            assert!(
                actual <= now,
                "a retention cutoff must never expire future rows"
            );
        }
    }
}

#[test]
fn increasing_retention_never_moves_cutoff_forward() {
    for now in [i64::MIN, -1, 0, 2_000_000_000, i64::MAX] {
        let mut previous = now;
        for days in [
            0,
            1,
            3650,
            i64::MAX as u64 / 86_400,
            i64::MAX as u64,
            u64::MAX,
        ] {
            let cutoff = cutoff_timestamp(now, days);
            assert!(cutoff <= previous, "now={now}, days={days}");
            previous = cutoff;
        }
    }
}

#[test]
fn duration_beyond_timestamp_range_saturates_to_oldest_timestamp() {
    for now in [i64::MIN, -1, 0, 2_000_000_000, i64::MAX] {
        assert_eq!(cutoff_timestamp(now, u64::MAX), i64::MIN);
        assert_eq!(cutoff_timestamp(now, i64::MAX as u64), i64::MIN);
    }
    assert_eq!(cutoff_timestamp(i64::MIN + 86_400, 1), i64::MIN);
    assert_eq!(cutoff_timestamp(i64::MIN + 86_401, 1), i64::MIN + 1);
}
