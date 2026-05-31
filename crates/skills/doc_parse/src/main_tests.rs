use super::normalize_action;

#[test]
fn normalize_action_accepts_parse_alias() {
    assert_eq!(normalize_action("parse_doc"), Some("parse_doc"));
    assert_eq!(normalize_action("parse"), Some("parse_doc"));
    assert_eq!(normalize_action("unknown"), None);
}
