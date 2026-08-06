use super::*;

fn profile(id: &str) -> GitConnectionProfile {
    GitConnectionProfile {
        id: id.to_string(),
        forge_kind: "github".to_string(),
        git_host: "github.com".to_string(),
        api_host: "api.github.com".to_string(),
        allowed_owners: vec!["ExampleOwner".to_string()],
        allowed_repositories: vec!["example-repository".to_string()],
        git_username: "x-access-token".to_string(),
        auth_scheme: "token".to_string(),
        git_credential_ref: GITHUB_GIT_CREDENTIAL_REF.to_string(),
        api_credential_ref: GITHUB_API_CREDENTIAL_REF.to_string(),
    }
}

#[test]
fn connection_store_uses_revision_cas_and_normalized_allowlists() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-git-connections-test-{}",
        Uuid::new_v4().simple()
    ));
    let path = root.join("connections.json");
    let saved = upsert_git_connection(&path, 0, profile("Primary")).expect("upsert");
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.profiles[0].id, "primary");
    assert_eq!(saved.profiles[0].allowed_owners, vec!["exampleowner"]);
    assert!(upsert_git_connection(&path, 0, profile("other")).is_err());
    let deleted = delete_git_connection(&path, 1, "PRIMARY").expect("delete");
    assert_eq!(deleted.revision, 2);
    assert!(deleted.profiles.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn v1_rejects_unapproved_hosts_and_credential_refs() {
    let mut candidate = profile("primary");
    candidate.git_host = "example.com".to_string();
    assert!(normalize_and_validate_profile(&mut candidate).is_err());

    let mut candidate = profile("primary");
    candidate.git_credential_ref = "other_token".to_string();
    assert!(normalize_and_validate_profile(&mut candidate).is_err());
}

#[test]
fn canonical_github_remote_is_credential_free_and_digest_stable() {
    let first = canonical_github_remote_url("https://github.com/ExampleOwner/Runtime.git")
        .expect("canonical GitHub remote");
    let second = canonical_github_remote_url("https://github.com/exampleowner/runtime")
        .expect("same canonical GitHub remote");
    assert_eq!(first, second);
    assert_eq!(first.owner, "exampleowner");
    assert_eq!(first.repository, "runtime");
    assert!(first.url_digest.starts_with("sha256:"));

    for value in [
        "https://token@github.com/ExampleOwner/Runtime.git",
        "https://github.com:443/ExampleOwner/Runtime.git",
        "https://api.github.com/ExampleOwner/Runtime.git",
        "https://127.0.0.1/ExampleOwner/Runtime.git",
        "file:///tmp/repository",
        "ssh://git@github.com/ExampleOwner/Runtime.git",
    ] {
        assert!(canonical_github_remote_url(value).is_err(), "{value}");
    }
}

#[test]
fn concurrent_connection_revision_allows_only_one_writer() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-git-connections-race-{}",
        Uuid::new_v4().simple()
    ));
    let path = root.join("connections.json");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = ["first", "second"].map(|id| {
        let path = path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            upsert_git_connection(&path, 0, profile(id)).is_ok()
        })
    });
    let successes = handles
        .into_iter()
        .map(|handle| handle.join().expect("writer thread"))
        .filter(|succeeded| *succeeded)
        .count();
    assert_eq!(successes, 1);
    let saved = load_git_connections(&path).expect("load winner");
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.profiles.len(), 1);
    let _ = fs::remove_dir_all(root);
}
