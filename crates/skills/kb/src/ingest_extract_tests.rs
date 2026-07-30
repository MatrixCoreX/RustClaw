use super::*;

#[test]
fn extracts_structured_text_without_lossy_binary_fallback() {
    let json = extract_document(br#"{"name":"Agent Runtime","enabled":true}"#, "json")
        .expect("extract JSON");
    match json {
        ExtractOutcome::Text {
            text,
            parser_version,
        } => {
            assert!(text.contains("Agent Runtime"));
            assert_eq!(parser_version, "json-structured-v1");
        }
        ExtractOutcome::Skip { .. } => panic!("JSON should be extracted"),
    }

    let html = extract_document(b"<h1>Title</h1><p>Body</p>", "html").expect("extract HTML");
    assert!(matches!(html, ExtractOutcome::Text { text, .. } if text == "Title Body"));

    let binary = extract_document(&[0xff, 0x00, 0x10], "bin").expect("classify binary");
    assert!(matches!(binary, ExtractOutcome::Skip { .. }));
}
