use serde_json::json;

#[test]
fn state_patch_accepts_path_like_direct_alias_map() {
    let patch = json!({
        "甲文件": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_path_field() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "path": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "locator_kind": "path",
            "scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_surface_value_fields() {
    let patch = json!({
        "alias_bindings": [{
            "surface": "甲文件",
            "kind": "path",
            "value": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_target_value_field() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "target_kind": "path",
            "target_value": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_target_path_field() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "자료A",
            "target_path": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "scope": "session"
        }],
        "required_content_literals": ["기억했습니다"]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "자료A");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_locator_field() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "locator": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "locator_kind": "path",
            "scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_locator_value_field() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "locator_kind": "path",
            "locator_value": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_array_locator_hint_field() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "자료A",
            "locator_kind": "path",
            "locator_hint": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "자료A");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_object_map() {
    let patch = json!({
        "alias_bindings": {
            "甲": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "ALPHA_DOC": {
                "target_value": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            }
        }
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|binding| {
        binding.alias == "甲"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }));
    assert!(bindings.iter().any(|binding| {
        binding.alias == "ALPHA_DOC"
            && binding.target == "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    }));
}

#[test]
fn state_patch_accepts_alias_bindings_object_map_value_field() {
    let patch = json!({
        "alias_bindings": {
            "자료A": {
                "kind": "path",
                "value": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
                "scope": "session",
                "created_by": "user_request"
            }
        },
        "required_content_literals": ["기억했습니다"]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "자료A");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_accepts_alias_bindings_add_or_update_schema() {
    let patch = json!({
        "alias_bindings": {
            "add_or_update": [{
                "alias": "자료A",
                "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
                "scope": "session"
            }],
            "remove": []
        },
        "required_content_literals": ["업데이트했습니다"]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "자료A");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_add_or_update_rejects_nonempty_remove() {
    let patch = json!({
        "alias_bindings": {
            "add_or_update": [{
                "alias": "자료A",
                "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            }],
            "remove": ["자료B"]
        }
    });

    assert!(!super::state_patch_is_alias_bindings_only(&patch));
}

#[test]
fn state_patch_accepts_alias_bindings_record_object() {
    let patch = json!({
        "alias_bindings": {
            "action": "replace",
            "name": "甲文件",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "target_abs": "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        }
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_allow_visibility_constraint_metadata() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        }],
        "forbidden_visible_literals": ["scripts/nl_tests/fixtures/device_local/docs/service_notes.md"],
        "required_content_literals": ["記憶しました"],
        "required_visible_literals": ["ack-token"],
        "primary_task_update": {"new_task": false}
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_allow_required_machine_field_metadata() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "자료A",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "scope": "session"
        }],
        "required_machine_fields": ["alias_bindings"],
        "required_content_literals": ["업데이트했습니다"]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "자료A");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_accept_alias_key_target_fields() {
    let patch = json!({
        "alias_bindings": [{
            "alias_key": "甲文件",
            "alias_target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "binding_scope": "session"
        }]
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn state_patch_alias_bindings_allow_alias_update_primary_task_metadata() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "previous_target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "scope": "session",
            "action": "rebind"
        }],
        "primary_task_update": {
            "action": "alias_update",
            "alias": "甲文件",
            "new_target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "previous_target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "confirmation_required": true
        }
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "甲文件");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_allow_alias_rebind_kind_metadata() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "자료A",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        }],
        "required_machine_fields": ["alias_bindings"],
        "primary_task_update": {
            "kind": "alias_rebind"
        }
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "자료A");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_allow_primary_task_projection_metadata() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "ALPHA_DOC",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "scope": "session"
        }],
        "primary_task_update": {
            "last_primary_task_prompt": "current request surface",
            "last_primary_task_output": "ack surface"
        }
    });

    assert!(super::state_patch_is_alias_bindings_only(&patch));
    let bindings = super::session_alias_bindings_from_state_patch(Some(&patch));
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].alias, "ALPHA_DOC");
    assert_eq!(
        bindings[0].target,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn state_patch_alias_bindings_reject_active_primary_task_update() {
    let patch = json!({
        "alias_bindings": [{
            "alias": "甲文件",
            "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
        }],
        "primary_task_update": {"new_task": true}
    });

    assert!(!super::state_patch_is_alias_bindings_only(&patch));
}

#[test]
fn state_patch_rejects_non_locator_direct_alias_map() {
    let patch = json!({
        "甲文件": "the checklist from before"
    });

    assert!(!super::state_patch_is_alias_bindings_only(&patch));
    assert!(super::session_alias_bindings_from_state_patch(Some(&patch)).is_empty());
}
