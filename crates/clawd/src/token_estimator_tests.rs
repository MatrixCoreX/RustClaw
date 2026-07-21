use super::*;

#[test]
fn minimax_estimate_uses_documented_cjk_ratio_with_conservative_admission() {
    let text = "测".repeat(1_600);

    let estimate =
        estimate_provider_tokens("vendor-minimax", "openai_compat", "MiniMax-M2.7", &text);

    assert_eq!(estimate.estimator, TokenEstimatorKind::MiniMaxM2);
    assert_eq!(estimate.provider_tokens, 1_000);
    assert_eq!(estimate.safety_tokens, 1_600);
    assert_eq!(estimate.char_count, 1_600);
    assert_eq!(estimate.byte_count, 4_800);
}

#[test]
fn mixed_language_estimates_are_provider_specific_and_never_zero() {
    let text = "fn main() { println!(\"你好世界持续执行\"); }";
    let minimax = estimate_provider_tokens("vendor-minimax", "openai_compat", "MiniMax-M2.7", text);
    let openai = estimate_provider_tokens("vendor-openai", "openai_compat", "gpt-5", text);

    assert!(minimax.provider_tokens < openai.provider_tokens);
    assert!(minimax.safety_tokens >= minimax.provider_tokens);
    assert!(openai.safety_tokens >= openai.provider_tokens);
    assert_eq!(estimate_generic_tokens("").safety_tokens, 0);
}

#[test]
fn unicode_estimator_does_not_split_or_underflow_multibyte_text() {
    let estimate = estimate_generic_tokens("日本語 한국어 café");

    assert_eq!(estimate.byte_count, "日本語 한국어 café".len());
    assert_eq!(estimate.char_count, "日本語 한국어 café".chars().count());
    assert!(estimate.provider_tokens > 0);
    assert!(estimate.safety_tokens >= estimate.provider_tokens);
}
