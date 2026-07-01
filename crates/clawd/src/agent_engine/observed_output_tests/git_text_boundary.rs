#[test]
fn git_current_branch_accepts_machine_extra_output() {
    let output = serde_json::json!({
        "status": "ok",
        "text": "git status completed",
        "extra": {
            "output": "exit=0\n## main...origin/main\n M Cargo.toml\n"
        }
    })
    .to_string();

    assert_eq!(
        super::output_git::git_current_branch_from_output(&output).as_deref(),
        Some("main")
    );
}

#[test]
fn git_current_branch_ignores_visible_text_output() {
    let output = serde_json::json!({
        "status": "ok",
        "text": "exit=0\n## main...origin/main\n M Cargo.toml\n"
    })
    .to_string();

    assert_eq!(
        super::output_git::git_current_branch_from_output(&output),
        None
    );
}

#[test]
fn git_current_branch_ignores_json_hidden_in_string_output() {
    let hidden_payload = serde_json::json!({
        "current_branch": "main"
    })
    .to_string();
    let output = serde_json::json!({
        "status": "ok",
        "output": hidden_payload
    })
    .to_string();

    assert_eq!(
        super::output_git::git_current_branch_from_output(&output),
        None
    );
}
