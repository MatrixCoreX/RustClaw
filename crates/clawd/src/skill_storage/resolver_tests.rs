use super::*;

#[test]
fn resolves_each_skill_to_an_isolated_database() {
    let resolver = SkillStorageResolver::test_default();
    let crypto = resolver.database_path("crypto").expect("crypto path");
    let kb = resolver.database_path("kb").expect("kb path");
    assert_eq!(
        crypto.file_name().and_then(|value| value.to_str()),
        Some("state.db")
    );
    assert_eq!(
        kb.file_name().and_then(|value| value.to_str()),
        Some("state.db")
    );
    assert_ne!(crypto.parent(), kb.parent());
    assert!(crypto.starts_with(resolver.root()));
    assert!(kb.starts_with(resolver.root()));
}

#[test]
fn rejects_traversal_and_non_machine_names() {
    let resolver = SkillStorageResolver::test_default();
    for value in ["../crypto", "kb/other", "KB", "知识库", ".hidden", "a..b"] {
        assert!(resolver.database_path(value).is_err(), "{value}");
    }
}
