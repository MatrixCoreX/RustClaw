use super::*;
#[test]
fn bullets_non_empty_from_sample() {
    let sample =
        "本公司2024年一季度营收同比上升12%。毛利率改善。\n风险提示：海外市场波动可能影响出口业务。";
    let b = summarize_bullets(sample, 5);
    assert!(!b.is_empty());
}
