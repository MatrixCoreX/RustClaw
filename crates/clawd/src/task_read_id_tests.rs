use super::*;

#[test]
fn accepts_parent_child_and_nested_host_ids_without_accepting_paths() {
    let parent = uuid::Uuid::new_v4();
    let child = format!("{parent}:child:review:0123456789abcdef0123456789abcdef");
    let nested = format!("{child}:child:entry:0123456789abcdef0123456789abcdef");
    for id in [parent.to_string(), child, nested] {
        assert_eq!(TaskReadId::try_from(id.clone()).unwrap().to_string(), id);
    }
    for id in [
        "../private",
        "not-a-task",
        "",
        "00000000-0000-0000-0000-000000000000:child:/etc:0123456789abcdef0123456789abcdef",
    ] {
        assert!(TaskReadId::try_from(id.to_string()).is_err());
    }
    assert!(TaskReadId::try_from(format!(
        "{parent}:child:{}:0123456789abcdef0123456789abcdef",
        "a".repeat(1024)
    ))
    .is_err());
}
