#[test]
fn compact_intent_normalizer_source_keeps_boundary_only_scalar_count_contract() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/intent_router_prompt_render.rs"
    ))
    .expect("read intent normalizer prompt source");

    assert!(source.contains("This stage extracts boundaries only"));
    assert!(source.contains("Do not classify ordinary capability families"));
    assert!(source.contains("scalar_count_filter"));
    assert!(source.contains("Do not put localized prose in machine fields"));
    assert!(!source.contains("SCALAR_COUNT_GUARD"));
}
