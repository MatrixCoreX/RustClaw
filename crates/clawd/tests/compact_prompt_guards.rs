#[test]
fn compact_intent_normalizer_source_keeps_scalar_count_root_exclusion_guard() {
    let source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/intent_router.rs"))
            .expect("read intent_router source");

    assert!(source.contains("SCALAR_COUNT_GUARD"));
    assert!(source.contains("root-excluding directory counts must not use recursive=false"));
    assert!(source.contains("Root-excluding directory counts mean recursive=true"));
}
