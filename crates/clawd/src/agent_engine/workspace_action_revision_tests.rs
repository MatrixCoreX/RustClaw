use super::*;

fn result(fingerprint: &str, path: &str, revision: &str) -> CapabilityResultEnvelope {
    let mut value = CapabilityResultEnvelope::ok(
        "filesystem.fixture",
        None,
        json!({"output": {
            "source": "workspace_mutation", "isolation_root": "workspace://current",
            "state": "applied", "target_path": path, "mutation_id": revision,
        }}),
    );
    value.effect = Some("mutate".to_string());
    value.provenance = json!({"source":"runtime_step", "task_id":"task", "host_workspace_primitive":true,
        "action_fingerprint": fingerprint, "step_id": revision});
    value
}

#[test]
fn recreated_file_is_a_new_operation_but_repeated_cleanup_is_not() {
    let mut results = vec![result("delete", "file.txt", "delete-1")];
    assert_eq!(
        revision_fingerprint("delete", "file.txt", "task", &results),
        "delete"
    );
    results.push(result("write", "file.txt", "write-2"));
    let next = revision_fingerprint("delete", "file.txt", "task", &results);
    assert_ne!(next, "delete");
    assert_eq!(original_fingerprint(&next).as_deref(), Some("delete"));
    results.push(result(&next, "file.txt", "delete-2"));
    assert_eq!(
        revision_fingerprint("delete", "file.txt", "task", &results),
        next
    );
    results.push(result("write-again", "file.txt", "write-3"));
    assert_ne!(
        revision_fingerprint("delete", "file.txt", "task", &results),
        next
    );
}

#[test]
fn new_read_uses_latest_mutation_revision_and_survives_restore() {
    let mut read = result("read", "file.txt", "unused");
    read.effect = Some("observe".to_string());
    let mut results = vec![read.clone(), result("write", "file.txt", "write-2")];
    let next = revision_fingerprint("read", "file.txt", "task", &results);
    read.provenance["action_fingerprint"] = json!(next);
    results.push(read);
    let restored: Vec<CapabilityResultEnvelope> = serde_json::from_value(json!(results)).unwrap();
    assert_eq!(
        revision_fingerprint("read", "file.txt", "task", &restored),
        next
    );
}

#[test]
fn unrelated_failed_untrusted_or_noop_changes_do_not_unlock_replay() {
    let previous = result("delete", "file.txt", "delete-1");
    for variant in 0..8 {
        let mut change = result("write", "file.txt", "write-2");
        match variant {
            0 => change.data["output"]["target_path"] = json!("other.txt"),
            1 => change.status = CapabilityResultStatus::Error,
            2 => change.provenance["task_id"] = json!("other-task"),
            3 => change.provenance["source"] = json!("model"),
            4 => change.data["output"]["state"] = json!("no_op"),
            5 => change.data["output"]["source"] = json!("external_service"),
            6 => change.data["output"]["mutation_id"] = json!(""),
            _ => change.provenance["host_workspace_primitive"] = json!(false),
        }
        assert_eq!(
            revision_fingerprint("delete", "file.txt", "task", &[previous.clone(), change]),
            "delete"
        );
    }
}

#[test]
fn external_calls_and_workspace_escape_have_no_local_revision_scope() {
    let state = AppState::test_default_with_fixture_provider();
    for (skill, args) in [
        (
            "run_cmd",
            json!({"command":"echo fixture", "path":"file.txt"}),
        ),
        (
            "http_basic",
            json!({"action":"request", "method":"POST", "path":"file.txt"}),
        ),
        ("remove_file", json!({"path":"../outside.txt"})),
        ("remove_file", json!({"path":"/outside/file.txt"})),
    ] {
        assert!(local_primitive_path(
            &state,
            &AgentAction::CallSkill {
                skill: skill.into(),
                args
            }
        )
        .is_none());
    }
    assert_eq!(
        local_primitive_path(
            &state,
            &AgentAction::CallSkill {
                skill: "remove_file".into(),
                args: json!({"path":"./file.txt"}),
            }
        )
        .as_deref(),
        Some("file.txt")
    );
}

#[test]
fn execution_keys_change_only_after_attested_same_path_effects() {
    let state = AppState::test_default_with_fixture_provider();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "task".into(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let policy = super::super::support::load_agent_loop_guard_policy(&state);
    for (skill, args, changes) in [
        ("remove_file", json!({"path":"file.txt"}), true),
        (
            "write_file",
            json!({"path":"file.txt","content":"same"}),
            true,
        ),
        ("read_file", json!({"path":"file.txt"}), true),
        (
            "run_cmd",
            json!({"command":"echo fixture", "path":"file.txt"}),
            false,
        ),
        (
            "http_basic",
            json!({"action":"request","method":"POST","path":"file.txt"}),
            false,
        ),
    ] {
        let action = AgentAction::CallSkill {
            skill: skill.into(),
            args,
        };
        let mut loop_state = LoopState::new();
        let base = execution_fingerprint(&state, &task, &policy, &loop_state, &action);
        loop_state
            .capability_results
            .push(result(&base, "file.txt", "first"));
        loop_state
            .successful_action_fingerprints
            .insert(base.clone(), 1);
        assert_eq!(
            execution_fingerprint(&state, &task, &policy, &loop_state, &action),
            base
        );
        loop_state
            .capability_results
            .push(result("other-action", "file.txt", "new-revision"));
        let next = execution_fingerprint(&state, &task, &policy, &loop_state, &action);
        assert_eq!(next != base, changes, "{skill}");
        assert_eq!(
            loop_state
                .successful_action_fingerprints
                .contains_key(&next),
            !changes
        );
        // A pending invocation does not create a new successful receipt. Resume
        // therefore derives the same key for the mutation ledger to reconcile.
        let mut resumed = LoopState::new();
        resumed.capability_results =
            serde_json::from_value(json!(loop_state.capability_results)).unwrap();
        assert_eq!(
            execution_fingerprint(&state, &task, &policy, &resumed, &action),
            next
        );
    }
}

#[test]
fn identical_content_recreated_again_has_a_distinct_committed_step() {
    let mut results = vec![result("delete", "file.txt", "deleted")];
    let mut change = result("write", "file.txt", "same-content-digest");
    change.provenance["step_id"] = json!("step_4");
    results.push(change.clone());
    let second_delete = revision_fingerprint("delete", "file.txt", "task", &results);
    results.push(result(&second_delete, "file.txt", "same-deletion-digest"));
    change.provenance["step_id"] = json!("step_7");
    results.push(change);
    let third_delete = revision_fingerprint("delete", "file.txt", "task", &results);
    assert_ne!(third_delete, second_delete);
    results.last_mut().unwrap().provenance["step_id"] = Value::Null;
    assert_eq!(
        revision_fingerprint("delete", "file.txt", "task", &results),
        second_delete
    );
}
