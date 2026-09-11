use super::*;

#[test]
fn trusted_context_binds_session_key_method_path_body_and_expiry() {
    let session = uuid::Uuid::new_v4().to_string();
    let path = format!("{PREFIX}operations/verify");
    let assertion = sign("key-a", &session, "POST", &path, b"proof", 100).unwrap();
    let expected = binding("key-a", Some(&assertion), "POST", &path, b"proof", 105).unwrap();
    for (key, method, route, body, now) in [
        ("key-b", "POST", path.as_str(), b"proof".as_slice(), 105),
        ("key-a", "GET", path.as_str(), b"proof".as_slice(), 105),
        ("key-a", "POST", "/other", b"proof".as_slice(), 105),
        ("key-a", "POST", path.as_str(), b"changed".as_slice(), 105),
        ("key-a", "POST", path.as_str(), b"proof".as_slice(), 116),
    ] {
        assert!(binding(key, Some(&assertion), method, route, body, now).is_none());
    }
    let later = sign("key-a", &session, "POST", &path, b"new proof", 200).unwrap();
    assert_eq!(
        binding("key-a", Some(&later), "POST", &path, b"new proof", 200),
        Some(expected.clone())
    );
    let new_session = sign(
        "key-a",
        &uuid::Uuid::new_v4().to_string(),
        "POST",
        &path,
        b"proof",
        100,
    )
    .unwrap();
    assert_ne!(
        binding("key-a", Some(&new_session), "POST", &path, b"proof", 100),
        Some(expected.clone())
    );
    assert_ne!(
        binding("key-a", None, "POST", &path, b"proof", 100),
        Some(expected)
    );
    assert!(binding("key-a", Some("forged"), "POST", &path, b"proof", 100).is_none());
}
