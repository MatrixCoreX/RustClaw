use super::web_search_candidate_title_sources_from_output;

#[test]
fn web_search_candidate_sources_ignore_visible_text_payload() {
    let output = r#"{"extra":{"candidates":[{"title":"Observed","source":"example.com"}]},"text":"{\"candidates\":[{\"title\":\"must_not_parse_text\",\"source\":\"bad.example\"}]}"}"#;

    let pairs = web_search_candidate_title_sources_from_output(output);

    assert!(pairs.iter().any(|(title, _)| title == "Observed"));
    assert!(!pairs
        .iter()
        .any(|(title, _)| title == "must_not_parse_text"));
}
