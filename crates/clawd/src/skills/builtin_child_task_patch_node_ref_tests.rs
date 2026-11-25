use super::*;

fn bind_node(fixture: &ChildPatchFixture, node_id: &str) {
    let db = fixture.state.core.db.get().unwrap();
    db.execute(
        "UPDATE tasks SET payload_json = json_set(payload_json,
         '$.child_task_contract.scope.node_id', ?2) WHERE task_id = ?1",
        rusqlite::params![fixture.child_task_id, node_id],
    )
    .unwrap();
}

fn invoke(
    fixture: &ChildPatchFixture,
    parent: &ClaimedTask,
    reference: &str,
    patch: &str,
    action: &str,
) -> Result<Value, String> {
    let args = json!({"action":action,"child_task_id":reference,"patch_ref":patch});
    execute_child_task_patch(&fixture.state, Some(parent), args.as_object().unwrap())
        .map(|output| serde_json::from_str(&output).unwrap())
}

fn error_code(result: Result<Value, String>) -> String {
    crate::skills::parse_structured_skill_error(&result.unwrap_err())
        .unwrap()
        .error_code
}

#[test]
fn exact_node_reference_reviews_then_applies_through_existing_checkpoint() {
    let fixture = ChildPatchFixture::new("short-ref-apply");
    bind_node(&fixture, "writer_main");
    let reference = "child_node:writer_main";
    let review = invoke(
        &fixture,
        &fixture.parent,
        reference,
        &fixture.patch_ref,
        "review_child_patch",
    )
    .unwrap();
    assert_eq!(review["child_task_id"], fixture.child_task_id);
    assert_eq!(review["patch_ref"], fixture.patch_ref);
    let applied = invoke(
        &fixture,
        &fixture.parent,
        reference,
        &fixture.patch_ref,
        "apply_child_patch",
    )
    .unwrap();
    assert_eq!(applied["disposition"], "applied");
    assert_eq!(applied["cleanup_status"], "complete");
    assert!(applied["workspace_patch"]["checkpoint_id"].is_string());
    assert_eq!(
        fs::read_to_string(fixture._repo.path.join("README.md")).unwrap(),
        "child change\n"
    );
}

#[test]
fn exact_node_reference_rejects_without_changing_parent_files() {
    let fixture = ChildPatchFixture::new("short-ref-reject");
    bind_node(&fixture, "writer_main");
    let before = fs::read(fixture._repo.path.join("README.md")).unwrap();
    let result = invoke(
        &fixture,
        &fixture.parent,
        "child_node:writer_main",
        &fixture.patch_ref,
        "reject_child_patch",
    )
    .unwrap();
    assert_eq!(result["disposition"], "rejected");
    assert_eq!(
        fs::read(fixture._repo.path.join("README.md")).unwrap(),
        before
    );
    assert!(!fixture.worktree_root.exists());
}

#[test]
fn short_reference_is_parent_scoped_and_does_not_disclose_another_patch() {
    let fixture = ChildPatchFixture::new("short-ref-parent");
    bind_node(&fixture, "writer_main");
    let mut other = fixture.parent.clone();
    other.task_id = "unrelated-parent".to_string();
    let error = invoke(
        &fixture,
        &other,
        "child_node:writer_main",
        &fixture.patch_ref,
        "review_child_patch",
    )
    .unwrap_err();
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "child_patch_node_not_found"
    );
    assert!(!error.contains(&fixture.child_task_id));
    assert!(!error.contains(&fixture.patch_ref));
    assert!(fixture.worktree_root.exists());
}

#[test]
fn unknown_invalid_or_mistyped_references_are_never_fuzzy_matched() {
    let fixture = ChildPatchFixture::new("short-ref-exact");
    bind_node(&fixture, "writer_main");
    for reference in [
        "child_node:",
        "child_node:writer/main",
        "child_node: writer_main",
    ] {
        assert_eq!(
            error_code(invoke(
                &fixture,
                &fixture.parent,
                reference,
                &fixture.patch_ref,
                "review_child_patch"
            )),
            "child_patch_node_ref_invalid"
        );
    }
    for reference in ["child_node:writer_maim", "child_node:writer_main_extra"] {
        assert_eq!(
            error_code(invoke(
                &fixture,
                &fixture.parent,
                reference,
                &fixture.patch_ref,
                "review_child_patch"
            )),
            "child_patch_node_not_found"
        );
    }
    let wrong_id = format!("{}-typo", fixture.child_task_id);
    assert_eq!(
        error_code(invoke(
            &fixture,
            &fixture.parent,
            &wrong_id,
            &fixture.patch_ref,
            "review_child_patch"
        )),
        "child_patch_task_not_found"
    );
    assert_eq!(
        fixture.run("review_child_patch").unwrap()["child_task_id"],
        fixture.child_task_id
    );
}

#[test]
fn ambiguous_parent_node_references_fail_closed_and_are_not_projected() {
    let fixture = ChildPatchFixture::new("short-ref-duplicate");
    bind_node(&fixture, "writer_main");
    let other_id = "second-child-same-node";
    let mut payload = child_payload(&fixture.parent.task_id, other_id);
    payload["child_task_contract"]["scope"]["node_id"] = json!("writer_main");
    insert_task(
        &fixture.state,
        other_id,
        "succeeded",
        &payload,
        &stored_result_json(&fixture.state, &fixture.child_task_id),
    );
    assert_eq!(
        error_code(invoke(
            &fixture,
            &fixture.parent,
            "child_node:writer_main",
            &fixture.patch_ref,
            "review_child_patch"
        )),
        "child_patch_node_ref_ambiguous"
    );
    let db = fixture.state.core.db.get().unwrap();
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &fixture.parent.task_id)
        .unwrap()
        .unwrap();
    assert!(graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|node| node["child_task_ref"].is_null()));
    assert!(fixture.worktree_root.exists());
}

#[test]
fn short_reference_keeps_patch_pin_and_artifact_integrity_checks() {
    let fixture = ChildPatchFixture::new("short-ref-integrity");
    bind_node(&fixture, "writer_main");
    let reference = "child_node:writer_main";
    let before = stored_result_json(&fixture.state, &fixture.child_task_id);
    let wrong_patch = fixture
        .patch_ref
        .trim_start_matches("child_worktree_patch:");
    let error = invoke(
        &fixture,
        &fixture.parent,
        reference,
        wrong_patch,
        "review_child_patch",
    )
    .unwrap_err();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "child_patch_ref_mismatch");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["details"]["child_task_ref"], reference);
    assert_eq!(extra["details"]["expected_patch_ref"], fixture.patch_ref);
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(
        stored_result_json(&fixture.state, &fixture.child_task_id),
        before
    );
    invoke(
        &fixture,
        &fixture.parent,
        reference,
        &fixture.patch_ref,
        "review_child_patch",
    )
    .unwrap();
    fs::write(&fixture.artifact_path, b"tampered artifact").unwrap();
    let result = invoke(
        &fixture,
        &fixture.parent,
        reference,
        &fixture.patch_ref,
        "review_child_patch",
    );
    assert_ne!(error_code(result), "child_patch_ref_mismatch");
    assert!(fixture.worktree_root.exists());
}

#[test]
fn node_reference_projection_uses_frozen_contract_not_child_output() {
    let fixture = ChildPatchFixture::new("short-ref-projection");
    bind_node(&fixture, "writer_main");
    let db = fixture.state.core.db.get().unwrap();
    db.execute("UPDATE tasks SET result_json=json_set(result_json,'$.child_task_result.child_task_ref','child_node:forged') WHERE task_id=?1", rusqlite::params![fixture.child_task_id]).unwrap();
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &fixture.parent.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        graph["nodes"][0]["child_task_ref"],
        "child_node:writer_main"
    );
    db.execute("UPDATE tasks SET payload_json=json_set(payload_json,'$.child_task_contract.parent_task_id','unrelated-parent') WHERE task_id=?1", rusqlite::params![fixture.child_task_id]).unwrap();
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &fixture.parent.task_id)
        .unwrap()
        .unwrap();
    assert!(graph["nodes"][0]["child_task_ref"].is_null());
    drop(db);
    assert_eq!(
        error_code(invoke(
            &fixture,
            &fixture.parent,
            "child_node:writer_main",
            &fixture.patch_ref,
            "review_child_patch"
        )),
        "child_patch_parent_mismatch"
    );
}

#[test]
fn malformed_payload_is_not_a_node_reference_or_an_sql_error() {
    let fixture = ChildPatchFixture::new("short-ref-json");
    let db = fixture.state.core.db.get().unwrap();
    db.execute(
        "UPDATE tasks SET payload_json='{invalid' WHERE task_id=?1",
        rusqlite::params![fixture.child_task_id],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        error_code(invoke(
            &fixture,
            &fixture.parent,
            "child_node:writer_main",
            &fixture.patch_ref,
            "review_child_patch"
        )),
        "child_patch_node_not_found"
    );
}
