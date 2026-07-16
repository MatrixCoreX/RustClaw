use serde_json::{json, Value};

use super::{step_output_excerpt_for_journal, TaskJournal};

fn workspace_patch_output(file_count: usize) -> String {
    let files = (0..file_count)
        .map(|index| {
            json!({
                "path": format!("src/file_{index}.rs"),
                "existed": true,
                "before_sha256": format!("sha256:before-{index}"),
                "after_sha256": format!("sha256:after-{index}"),
                "backup_file": format!("before/{index}.bin"),
                "additions": 2,
                "deletions": 1,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "source": "workspace_patch",
        "status": "ok",
        "action": "apply_patch",
        "message_key": "workspace.patch.applied",
        "patch_id": "sha256:patch-1",
        "checkpoint_id": "patch_checkpoint_1",
        "isolation_root": "workspace://current",
        "reversible": true,
        "changed_files": ["src/file_0.rs", "src/file_1.rs"],
        "additions": 4,
        "deletions": 2,
        "hunk_count": 2,
        "changed_hunks": 2,
        "files": files,
        "artifact_refs": [
            {"kind": "workspace_patch", "ref": "workspace_patch:sha256:patch-1"},
            {"kind": "workspace_checkpoint", "ref": "workspace_checkpoint:patch_checkpoint_1"},
        ],
    })
    .to_string()
}

#[test]
fn workspace_patch_excerpt_preserves_bounded_rewind_evidence() {
    let excerpt = step_output_excerpt_for_journal(&workspace_patch_output(160));
    let value: Value = serde_json::from_str(&excerpt).expect("compact patch output");

    assert_eq!(
        value.pointer("/extra/patch_id"),
        Some(&json!("sha256:patch-1"))
    );
    assert_eq!(
        value.pointer("/extra/checkpoint_id"),
        Some(&json!("patch_checkpoint_1"))
    );
    assert_eq!(value.pointer("/extra/reversible"), Some(&json!(true)));
    assert_eq!(value.pointer("/extra/changed_hunks"), Some(&json!(2)));
    assert_eq!(
        value
            .pointer("/extra/files")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(128)
    );
    assert_eq!(
        value.pointer("/extra/files/0/before_sha256"),
        Some(&json!("sha256:before-0"))
    );
    assert!(value.pointer("/extra/files/0/backup_file").is_none());
}

#[test]
fn patch_and_verification_events_reference_the_workspace_checkpoint() {
    let mut journal = TaskJournal::for_task("task-workspace-patch", "ask", "patch and test");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_patch".to_string(),
        skill: "workspace_patch".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(workspace_patch_output(2)),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_verify".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "command": "cargo test -p demo",
                "test_command": "cargo test -p demo",
                "test_status": "passed",
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let trace = journal.to_trace_json();
    assert_eq!(
        trace.pointer("/step_results/0/structured_workspace_mutation/checkpoint_id"),
        Some(&json!("patch_checkpoint_1"))
    );
    assert_eq!(
        trace.pointer("/step_results/0/artifact_refs/0/ref"),
        Some(&json!("workspace_patch:sha256:patch-1"))
    );
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event stream");
    let milestone = events
        .iter()
        .find(|event| {
            event
                .pointer("/payload/checkpoint_kind")
                .and_then(Value::as_str)
                == Some("verified_workspace_checkpoint")
        })
        .expect("verified workspace checkpoint");
    assert_eq!(
        milestone.pointer("/payload/workspace_checkpoint_ids/0"),
        Some(&json!("patch_checkpoint_1"))
    );
    assert_eq!(
        milestone.pointer("/payload/patch_ids/0"),
        Some(&json!("sha256:patch-1"))
    );
    assert_eq!(
        milestone.pointer("/payload/verification_status"),
        Some(&json!("verified"))
    );
    let shell_event = events
        .iter()
        .find(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("tool_finished")
                && event.pointer("/payload/step_id").and_then(Value::as_str) == Some("step_verify")
        })
        .expect("shell tool event");
    assert_eq!(
        shell_event.pointer("/payload/reversible"),
        Some(&json!(false))
    );
    assert_eq!(
        shell_event.pointer("/payload/reversibility_reason_code"),
        Some(&json!("shell_side_effects_not_tracked"))
    );
}

#[test]
fn structured_write_and_rewind_preserve_mutation_compensation_fields() {
    let write_output = json!({
        "schema_version": 1,
        "source": "workspace_mutation",
        "status": "ok",
        "action": "write_text",
        "message_key": "workspace.mutation.applied",
        "checkpoint_id": "mutation_checkpoint_1",
        "mutation_id": "sha256:mutation-1",
        "state": "applied",
        "target_path": "src/lib.rs",
        "isolation_root": "workspace://current",
        "reversible": true,
        "changed_files": ["src/lib.rs"],
        "before": [{
            "path": "src/lib.rs",
            "kind": "file",
            "sha256": "sha256:before",
            "size_bytes": 6,
            "backup_file": "before/0000.bin"
        }],
        "after": [{
            "path": "src/lib.rs",
            "kind": "file",
            "sha256": "sha256:after",
            "size_bytes": 5
        }],
        "artifact_refs": [
            {"kind": "workspace_mutation", "ref": "workspace_mutation:sha256:mutation-1"},
            {"kind": "workspace_checkpoint", "ref": "workspace_checkpoint:mutation_checkpoint_1"}
        ]
    })
    .to_string();
    let rewind_output = json!({
        "schema_version": 1,
        "source": "workspace_mutation",
        "status": "ok",
        "action": "rewind",
        "message_key": "workspace.mutation.rewound",
        "checkpoint_id": "mutation_checkpoint_1",
        "mutation_id": "sha256:mutation-1",
        "compensates_checkpoint_id": "mutation_checkpoint_1",
        "compensates_mutation_id": "sha256:mutation-1",
        "state": "rewound",
        "target_path": "src/lib.rs",
        "isolation_root": "workspace://current",
        "reversible": false
    })
    .to_string();

    let compact = step_output_excerpt_for_journal(&write_output);
    let compact: Value = serde_json::from_str(&compact).expect("compact mutation");
    assert_eq!(
        compact.pointer("/extra/mutation_id"),
        Some(&json!("sha256:mutation-1"))
    );
    assert_eq!(
        compact.pointer("/extra/before/0/sha256"),
        Some(&json!("sha256:before"))
    );
    assert!(compact.pointer("/extra/before/0/backup_file").is_none());

    let mut journal = TaskJournal::for_task("task-structured-write", "ask", "write and rewind");
    for (step_id, skill, output, started_at) in [
        ("step_write", "write_file", write_output, 1),
        ("step_rewind", "workspace_patch", rewind_output, 3),
    ] {
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(output),
            error: None,
            started_at,
            finished_at: started_at + 1,
        });
    }
    let trace = journal.to_trace_json();
    assert_eq!(
        trace.pointer("/step_results/0/structured_workspace_mutation/mutation_id"),
        Some(&json!("sha256:mutation-1"))
    );
    assert_eq!(
        trace.pointer("/step_results/1/structured_workspace_mutation/compensates_checkpoint_id"),
        Some(&json!("mutation_checkpoint_1"))
    );
    let events = trace
        .get("event_stream")
        .and_then(Value::as_array)
        .expect("event stream");
    assert!(events.iter().any(|event| {
        event
            .pointer("/payload/compensates_mutation_id")
            .and_then(Value::as_str)
            == Some("sha256:mutation-1")
    }));
}
