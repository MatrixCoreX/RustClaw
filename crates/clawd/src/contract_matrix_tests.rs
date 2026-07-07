use std::{collections::BTreeMap, path::PathBuf};

use super::*;
use crate::task_contract::fallback_required_evidence_fields_for_output_contract;
use crate::{
    OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, ResumeBehavior, RiskCeiling,
    RouteResult, ScheduleKind,
};
#[path = "contract_matrix_recent_artifacts_tests.rs"]
mod recent_artifacts_tests;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn load_workspace_matrix() -> ContractMatrix {
    ContractMatrix::load_from_workspace(&workspace_root()).expect("load contract matrix")
}

fn route_with_machine_capability_ref(capability_ref: &str) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: capability_ref.to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: capability_ref.to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            semantic_kind: OutputSemanticKind::None,
            ..IntentOutputContract::default()
        },
    }
}

#[test]
fn route_capability_ref_allows_config_archive_policy_without_semantic_kind() {
    for (capability_ref, skill, args, expected_action, expected_evidence) in [
        (
            "capability_ref=config.validate",
            "config_basic",
            serde_json::json!({"action":"validate","path":"configs/config.toml"}),
            "config_basic.validate",
            vec!["valid"],
        ),
        (
            "capability_ref=config.read_field",
            "system_basic",
            serde_json::json!({"action":"extract_field","path":"configs/config.toml","field_path":"server.port"}),
            "config_basic.read_field",
            vec!["field_value"],
        ),
        (
            "capability_ref=config.read_fields",
            "config_basic",
            serde_json::json!({"action":"read_fields","path":"configs/config.toml","field_paths":["server.port"]}),
            "config_basic.read_fields",
            vec!["field_value"],
        ),
        (
            "capability_ref=config.list_keys",
            "system_basic",
            serde_json::json!({"action":"structured_keys","path":"configs/config.toml"}),
            "config_basic.list_keys",
            vec!["field_value"],
        ),
        (
            "capability_ref=archive.pack",
            "archive_basic",
            serde_json::json!({"action":"pack","source":"tmp/report","archive":"tmp/report.zip"}),
            "archive_basic.pack",
            vec!["path"],
        ),
        (
            "capability_ref=archive.unpack",
            "archive_basic",
            serde_json::json!({"action":"unpack","archive":"tmp/report.zip","dest":"tmp/report"}),
            "archive_basic.unpack",
            vec!["path"],
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
        assert_eq!(policy.required_evidence, expected_evidence);
    }
}

#[test]
fn route_capability_ref_allows_filesystem_observe_policy_without_semantic_kind() {
    for (capability_ref, skill, args, expected_action) in [
        (
            "capability_ref=filesystem.stat_paths",
            "system_basic",
            serde_json::json!({"action":"path_batch_facts","paths":["README.md"]}),
            "fs_basic.stat_paths",
        ),
        (
            "capability_ref=filesystem.list_entries",
            "system_basic",
            serde_json::json!({"action":"inventory_dir","path":"crates","names_only":true}),
            "fs_basic.list_dir",
        ),
        (
            "capability_ref=filesystem.count_entries",
            "system_basic",
            serde_json::json!({"action":"count_inventory","path":"crates"}),
            "fs_basic.count_entries",
        ),
        (
            "capability_ref=filesystem.read_text",
            "system_basic",
            serde_json::json!({"action":"read_range","path":"README.md","start_line":1,"end_line":8}),
            "fs_basic.read_text_range",
        ),
        (
            "capability_ref=filesystem.find_entries",
            "system_basic",
            serde_json::json!({"action":"find_path","path":".","name":"Cargo.toml"}),
            "fs_basic.find_entries",
        ),
        (
            "capability_ref=filesystem.grep_text",
            "fs_basic",
            serde_json::json!({"action":"grep_text","root":"crates","query":"capability_ref"}),
            "fs_basic.grep_text",
        ),
        (
            "capability_ref=filesystem.compare_paths",
            "system_basic",
            serde_json::json!({"action":"compare_paths","left_path":"README.md","right_path":"README.zh-CN.md"}),
            "fs_basic.compare_paths",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
    }
}

#[test]
fn route_capability_ref_allows_filesystem_mutate_policy_without_semantic_kind() {
    for (capability_ref, args, expected_action) in [
        (
            "capability_ref=filesystem.write_text",
            serde_json::json!({"action":"write_text","path":"tmp/report.txt","content":"ok"}),
            "fs_basic.write_text",
        ),
        (
            "capability_ref=filesystem.append_file",
            serde_json::json!({"action":"append_text","path":"tmp/report.txt","content":"more"}),
            "fs_basic.append_text",
        ),
        (
            "capability_ref=filesystem.create_dir",
            serde_json::json!({"action":"make_dir","path":"tmp/generated"}),
            "fs_basic.make_dir",
        ),
        (
            "capability_ref=filesystem.delete_path",
            serde_json::json!({"action":"remove_path","path":"tmp/generated"}),
            "fs_basic.remove_path",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), "fs_basic", &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
    }
}

#[test]
fn route_capability_ref_allows_service_and_docker_policy_without_semantic_kind() {
    for (capability_ref, skill, args, expected_action) in [
        (
            "capability_ref=service.status",
            "service_control",
            serde_json::json!({"action":"status","target":"clawd"}),
            "service_control.status",
        ),
        (
            "capability_ref=service.verify",
            "service_control",
            serde_json::json!({"action":"verify","target":"clawd"}),
            "service_control.verify",
        ),
        (
            "capability_ref=service.logs",
            "service_control",
            serde_json::json!({"action":"logs","target":"clawd","lines":80}),
            "service_control.logs",
        ),
        (
            "capability_ref=service.restart",
            "service_control",
            serde_json::json!({"action":"restart","target":"clawd"}),
            "service_control.restart",
        ),
        (
            "capability_ref=docker.list_containers",
            "docker_basic",
            serde_json::json!({"action":"ps","limit":5}),
            "docker_basic.ps",
        ),
        (
            "capability_ref=docker.list_images",
            "docker_basic",
            serde_json::json!({"action":"images","limit":5}),
            "docker_basic.images",
        ),
        (
            "capability_ref=docker.version",
            "docker_basic",
            serde_json::json!({"action":"version"}),
            "docker_basic.version",
        ),
        (
            "capability_ref=docker.inspect_container",
            "docker_basic",
            serde_json::json!({"action":"inspect","container":"rustclaw"}),
            "docker_basic.inspect",
        ),
        (
            "capability_ref=docker.read_logs",
            "docker_basic",
            serde_json::json!({"action":"logs","container":"rustclaw","tail":100}),
            "docker_basic.logs",
        ),
        (
            "capability_ref=docker.restart_container",
            "docker_basic",
            serde_json::json!({"action":"restart","container":"rustclaw"}),
            "docker_basic.restart",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
    }
}

#[test]
fn route_capability_ref_allows_system_policy_without_semantic_kind() {
    for (capability_ref, args, expected_action) in [
        (
            "capability_ref=system.info",
            serde_json::json!({"action":"info"}),
            "system_basic.info",
        ),
        (
            "capability_ref=system.runtime_status",
            serde_json::json!({"action":"runtime_status","kind":"current_user"}),
            "system_basic.runtime_status",
        ),
        (
            "capability_ref=system.inventory_dir",
            serde_json::json!({"action":"inventory_dir","path":"crates"}),
            "fs_basic.list_dir",
        ),
        (
            "capability_ref=system.tree_summary",
            serde_json::json!({"action":"tree_summary","path":"crates","max_depth":2}),
            "system_basic.tree_summary",
        ),
        (
            "capability_ref=system.read_range",
            serde_json::json!({"action":"read_range","path":"README.md","start_line":1,"end_line":5}),
            "fs_basic.read_text_range",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), "system_basic", &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
    }
}

#[test]
fn route_capability_ref_allows_process_and_task_control_policy_without_semantic_kind() {
    for (capability_ref, skill, args, expected_action) in [
        (
            "capability_ref=process.ps",
            "process_basic",
            serde_json::json!({"action":"ps","limit":5}),
            "process_basic.ps",
        ),
        (
            "capability_ref=process.port_list",
            "process_basic",
            serde_json::json!({"action":"port_list","port":8080}),
            "process_basic.port_list",
        ),
        (
            "capability_ref=process.kill",
            "process_basic",
            serde_json::json!({"action":"kill","pid":12345,"signal":"TERM"}),
            "process_basic.kill",
        ),
        (
            "capability_ref=process.tail_log",
            "process_basic",
            serde_json::json!({"action":"tail_log","path":"logs/claw.log","n":20}),
            "process_basic.tail_log",
        ),
        (
            "capability_ref=task_control.list",
            "task_control",
            serde_json::json!({"action":"list","limit":10}),
            "task_control.list",
        ),
        (
            "capability_ref=task_control.get",
            "task_control",
            serde_json::json!({"action":"get","task_id":"task-1"}),
            "task_control.get",
        ),
        (
            "capability_ref=task_control.cancel_one",
            "task_control",
            serde_json::json!({"action":"cancel_one","index":1,"confirm":true}),
            "task_control.cancel_one",
        ),
        (
            "capability_ref=task_control.resume",
            "task_control",
            serde_json::json!({"action":"resume","task_id":"task-1","dry_run":true}),
            "task_control.resume",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
    }
}

#[test]
fn route_capability_ref_uses_registry_metadata_for_exact_skill_actions_without_semantic_kind() {
    for (capability_ref, skill, args, expected_action) in [
        (
            "capability_ref=weather.current",
            "weather",
            serde_json::json!({"action":"query","city":"Shanghai"}),
            "weather.query",
        ),
        (
            "capability_ref=web.search_results",
            "web_search_extract",
            serde_json::json!({"action":"search_extract","query":"rustclaw"}),
            "web_search_extract.search_extract",
        ),
        (
            "capability_ref=kb.search",
            "kb",
            serde_json::json!({"action":"search","namespace":"docs","query":"agent loop"}),
            "kb.search",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
        assert_eq!(policy.contract_repair_source, "capability_ref_route_policy");
    }
}

#[test]
fn route_capability_ref_reads_final_answer_shape_from_registry_metadata() {
    let route = route_with_machine_capability_ref("capability_ref=config.read_field");

    assert_eq!(
        route.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert_eq!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );

    let trace = action_trace_for_route(&route, "config_basic.read_field")
        .expect("route capability action trace");

    assert_eq!(trace["contract_match"], "capability_ref");
    assert_eq!(trace["final_answer_shape"], "scalar");
}

#[test]
fn route_capability_ref_rejects_registry_action_mismatch_without_semantic_kind() {
    let route = route_with_machine_capability_ref("capability_ref=weather.current");

    let policy = action_policy_for_route(
        Some(&route),
        "kb",
        &serde_json::json!({"action":"search","namespace":"docs","query":"weather"}),
    );

    assert!(
        policy
            .as_ref()
            .is_none_or(|policy| policy.contract_match != "capability_ref"),
        "{policy:?}"
    );
}

#[test]
fn route_capability_ref_uses_registry_machine_aliases_without_fallback_match() {
    for (capability_ref, skill, args, expected_action) in [
        (
            "capability_ref=config.guard",
            "config_guard",
            serde_json::json!({}),
            "config_edit.guard_config",
        ),
        (
            "capability_ref=config.risk",
            "config_guard",
            serde_json::json!({}),
            "config_edit.guard_config",
        ),
        (
            "capability_ref=db.list",
            "db_basic",
            serde_json::json!({"action":"list_tables","db_path":"data/app.db"}),
            "db_basic.list_tables",
        ),
        (
            "capability_ref=sqlite.query",
            "db_basic",
            serde_json::json!({"action":"sqlite_query","db_path":"data/app.db","sql":"select 1"}),
            "db_basic.sqlite_query",
        ),
        (
            "capability_ref=git.repository_state",
            "git_basic",
            serde_json::json!({"action":"status","repo":"."}),
            "git_basic.status",
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
    }
}

#[test]
fn route_capability_ref_overrides_bridge_semantic_policy_match() {
    for (semantic_kind, capability_ref, skill, args) in [
        (
            OutputSemanticKind::ConfigValidation,
            "capability_ref=config.validate",
            "config_basic",
            serde_json::json!({"action":"validate","path":"configs/config.toml"}),
        ),
        (
            OutputSemanticKind::ArchivePack,
            "capability_ref=archive.pack",
            "archive_basic",
            serde_json::json!({"action":"pack","source":"tmp/report","archive":"tmp/report.zip"}),
        ),
    ] {
        let mut route = route_with_machine_capability_ref(capability_ref);
        route.output_contract.semantic_kind = semantic_kind;

        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("policy decision for {capability_ref}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.contract_match, "capability_ref");
        assert_eq!(policy.contract_repair_source, "capability_ref_route_policy");
    }
}

#[test]
fn route_capability_ref_policy_does_not_inherit_wrong_semantic_shape() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let policy = action_policy_for_route(
        Some(&route),
        "config_basic",
        &serde_json::json!({
            "action": "validate",
            "path": "configs/config.toml",
        }),
    )
    .expect("route capability policy");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.contract_match, "capability_ref");
    assert_eq!(
        policy.final_answer_shape_kind,
        FinalAnswerShape::ValidationVerdict
    );
    assert_eq!(policy.final_answer_shape, "validation_verdict");
}

#[test]
fn route_action_policy_canonicalizes_virtual_config_validation() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;

    let policy = action_policy_for_route(
        Some(&route),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("route action policy");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(
        policy.original_action_ref,
        "system_basic.validate_structured"
    );
    assert_eq!(
        policy.replacement_action_ref.as_deref(),
        Some("config_basic.validate")
    );
    assert_eq!(policy.contract_match, "capability_ref");
}

#[test]
fn route_capability_ref_action_policy_does_not_require_semantic_bridge() {
    let route = route_with_machine_capability_ref("capability_ref=config.validate");

    let policy = action_policy_for_route(
        Some(&route),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("route capability policy");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.contract_match, "capability_ref");
    assert_eq!(policy.required_evidence, vec!["valid"]);
}

#[test]
fn config_list_keys_capability_ref_supplies_key_list_shape_without_semantic_kind() {
    let route = route_with_machine_capability_ref("capability_ref=config.list_keys");

    assert_eq!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::KeyListOrKeySummary)
    );
}

#[test]
fn config_validation_capability_refs_supply_verdict_shape_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=config.validate",
        "capability_ref=config.guard_rustclaw_config",
        "capability_ref=config.validate_after_change",
        "capability_ref=config.guard_config",
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        assert_eq!(
            final_answer_shape_for_route(&route),
            Some(FinalAnswerShape::ValidationVerdict),
            "{capability_ref}"
        );
    }
}

#[test]
fn service_status_capability_refs_supply_status_shape_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=service.status",
        "capability_ref=service_control.status",
        "capability_ref=system.health_check",
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        assert_eq!(
            final_answer_shape_for_route(&route),
            Some(FinalAnswerShape::StatusWithSource),
            "{capability_ref}"
        );
    }

    let route = route_with_machine_capability_ref("capability_ref=service.logs");
    assert_ne!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::StatusWithSource)
    );
}

#[test]
fn service_lifecycle_capability_refs_supply_lifecycle_shape_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=service.start",
        "capability_ref=service.stop",
        "capability_ref=service.restart",
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        assert_eq!(
            final_answer_shape_for_route(&route),
            Some(FinalAnswerShape::LifecycleResult),
            "{capability_ref}"
        );
    }

    let route = route_with_machine_capability_ref("capability_ref=service.verify");
    assert_ne!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::LifecycleResult)
    );
}

#[test]
fn docker_capability_refs_supply_docker_shapes_without_semantic_kind() {
    for (capability_ref, expected_shape) in [
        (
            "capability_ref=docker.list_containers",
            FinalAnswerShape::ContainerList,
        ),
        ("capability_ref=docker.ps", FinalAnswerShape::ContainerList),
        (
            "capability_ref=docker.list_images",
            FinalAnswerShape::ImageList,
        ),
        ("capability_ref=docker.images", FinalAnswerShape::ImageList),
        (
            "capability_ref=docker.read_logs",
            FinalAnswerShape::LogExcerptOrSummary,
        ),
        (
            "capability_ref=docker.logs",
            FinalAnswerShape::LogExcerptOrSummary,
        ),
        (
            "capability_ref=docker.restart_container",
            FinalAnswerShape::LifecycleResult,
        ),
        (
            "capability_ref=docker.stop",
            FinalAnswerShape::LifecycleResult,
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        assert_eq!(
            final_answer_shape_for_route(&route),
            Some(expected_shape),
            "{capability_ref}"
        );
    }

    let route = route_with_machine_capability_ref("capability_ref=docker.version");
    assert_ne!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::ContainerList)
    );
}

#[test]
fn filesystem_count_entries_capability_ref_supplies_scalar_shape_without_semantic_kind() {
    let mut route = route_with_machine_capability_ref("capability_ref=filesystem.count_entries");
    route.output_contract.response_shape = OutputResponseShape::Scalar;

    assert_eq!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );
}

#[test]
fn filesystem_list_entries_capability_ref_supplies_grouped_name_shape_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=filesystem.list_entries",
        "capability_ref=filesystem.list_dir",
    ] {
        let route = route_with_machine_capability_ref(capability_ref);

        assert_eq!(
            final_answer_shape_for_route(&route),
            Some(FinalAnswerShape::GroupedNameList),
            "{capability_ref}"
        );
    }
}

#[test]
fn system_runtime_status_capability_ref_supplies_scalar_shape_only_for_scalar_contract() {
    let mut route = route_with_machine_capability_ref("capability_ref=system.runtime_status");
    route.output_contract.response_shape = OutputResponseShape::Scalar;

    assert_eq!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );

    route.output_contract.response_shape = OutputResponseShape::Strict;
    assert_ne!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );
}

#[test]
fn config_read_field_capability_ref_supplies_scalar_shape_only_for_single_scalar_contract() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.read_field");
    route.output_contract.response_shape = OutputResponseShape::Scalar;

    assert_eq!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );

    route.route_reason = "capability_ref=config.read_fields".to_string();
    route.resolved_intent = "capability_ref=config.read_fields".to_string();
    assert_ne!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );
}

#[test]
fn system_extract_field_capability_ref_supplies_scalar_shape_only_for_single_scalar_contract() {
    let mut route = route_with_machine_capability_ref("capability_ref=system_basic.extract_field");
    route.output_contract.response_shape = OutputResponseShape::Scalar;

    assert_eq!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );

    route.route_reason = "capability_ref=system_basic.extract_fields".to_string();
    route.resolved_intent = "capability_ref=system_basic.extract_fields".to_string();
    assert_ne!(
        final_answer_shape_for_route(&route),
        Some(FinalAnswerShape::Scalar)
    );
}

#[test]
fn route_capability_ref_arg_policy_does_not_require_semantic_bridge() {
    let route = route_with_machine_capability_ref("capability_ref=config.validate");

    let policy = arg_policy_decision_for_route(
        Some(&route),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("route arg policy");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.contract_match, "capability_ref");
}

#[test]
fn route_arg_policy_prefers_capability_ref_over_bridge_semantic_match() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;

    let policy = arg_policy_decision_for_route(
        Some(&route),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("route arg policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.contract_match, "capability_ref");
    assert_eq!(policy.expected_target_args, vec!["path"]);
}

#[test]
fn route_capability_ref_arg_policy_does_not_inherit_wrong_semantic_shape() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let policy = arg_policy_decision_for_route(
        Some(&route),
        "config_basic",
        &serde_json::json!({
            "action": "validate",
            "path": "configs/config.toml",
        }),
    )
    .expect("route capability arg policy");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.contract_match, "capability_ref");
    assert_eq!(policy.final_answer_shape, "validation_verdict");
}

#[test]
fn route_arg_policy_ignores_legacy_marker_without_capability_ref() {
    let route = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "contract_marker=config_validation".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        },
    };

    let policy = arg_policy_decision_for_route(
        Some(&route),
        "config_basic",
        &serde_json::json!({"action": "validate"}),
    );

    assert!(policy.is_none());
}

#[test]
fn route_capability_ref_action_trace_does_not_inherit_wrong_semantic_shape() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let trace = action_trace_for_route(&route, "config_basic.validate")
        .expect("route capability action trace");

    assert_eq!(trace["contract_match"], "capability_ref");
    assert_eq!(trace["final_answer_shape"], "validation_verdict");
}

#[test]
fn route_capability_ref_action_refs_do_not_inherit_wrong_semantic_matrix_scope() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let allowed = allowed_action_refs_for_route(&route)
        .into_iter()
        .map(|action| action.as_key())
        .collect::<Vec<_>>();
    let preferred = preferred_action_refs_for_route(&route)
        .into_iter()
        .map(|action| action.as_key())
        .collect::<Vec<_>>();

    assert_eq!(allowed, vec!["config_basic.validate"]);
    assert_eq!(preferred, vec!["config_basic.validate"]);
}

#[test]
fn route_capability_ref_compact_prompt_line_does_not_inherit_wrong_semantic_matrix_scope() {
    let mut route = route_with_machine_capability_ref("capability_ref=config.validate");
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let line = compact_prompt_line_for_route(&route).expect("capability compact line");

    assert!(line.contains("capability_policy"));
    assert!(line.contains("match=capability_ref"));
    assert!(line.contains("capability_refs=config.validate"));
    assert!(line.contains("available_action_refs=config_basic.validate"));
    assert!(line.contains("preferred_action_refs=config_basic.validate"));
    assert!(!line.contains("allowed_actions="));
    assert!(!line.contains("contract_matrix"));
    assert!(!line.contains("fs_basic.list_dir"));
}

#[test]
fn unknown_route_capability_ref_does_not_fall_back_to_semantic_matrix_actions() {
    let mut route = route_with_machine_capability_ref("capability_ref=unknown.future_action");
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    assert!(allowed_action_refs_for_route(&route).is_empty());
    assert!(preferred_action_refs_for_route(&route).is_empty());

    let line = compact_prompt_line_for_route(&route).expect("unknown capability compact line");
    assert!(line.contains("capability_refs=unknown.future_action"));
    assert!(line.contains("available_action_refs=none"));
    assert!(line.contains("preferred_action_refs=none"));
    assert!(!line.contains("allowed_actions="));
    assert!(!line.contains("fs_basic.list_dir"));
}

#[test]
fn route_policy_without_capability_ref_does_not_reject_config_action() {
    let mut route = route_with_machine_capability_ref("machine_context=no_capability_ref");
    route.route_reason.clear();
    route.resolved_intent.clear();

    let policy = action_policy_for_route(
        Some(&route),
        "config_basic",
        &serde_json::json!({"action":"validate","path":"configs/config.toml"}),
    );

    assert!(policy.is_none());
}

#[test]
fn recent_scalar_equality_allows_structured_field_extractors() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::RecentScalarEqualityCheck)
        .expect("recent scalar equality contract");
    let matched = MatchedContract::Semantic(contract);
    for action in [
        "config_basic.read_field",
        "config_basic.read_fields",
        "system_basic.info",
        "system_basic.runtime_status",
    ] {
        let action_ref = ActionRef::parse(action).expect("action parses");
        assert_eq!(
            matched.action_policy(&action_ref),
            ActionPolicyDecision::Allowed,
            "{action} should be allowed for scalar field and local runtime observations"
        );
    }
}

#[test]
fn command_output_summary_allows_structured_field_extractors() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::CommandOutputSummary)
        .expect("command output summary contract");
    let matched = MatchedContract::Semantic(contract);
    for action in [
        "config_basic.read_field",
        "config_basic.read_fields",
        "fs_basic.stat_paths",
        "fs_basic.read_text_range",
        "archive_basic.list",
        "archive_basic.read",
        "db_basic.list_tables",
    ] {
        let action_ref = ActionRef::parse(action).expect("action parses");
        assert_eq!(
            matched.action_policy(&action_ref),
            ActionPolicyDecision::Allowed,
            "{action} should be allowed for mixed evidence summaries"
        );
    }
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    for (skill, args, expected_action) in [
        (
            "system_basic",
            serde_json::json!({"action":"extract_field","path":"Cargo.toml","field_path":"package.name"}),
            "config_basic.read_field",
        ),
        (
            "system_basic",
            serde_json::json!({"action":"extract_fields","path":"Cargo.toml","field_paths":["package.name"]}),
            "config_basic.read_fields",
        ),
        (
            "system_basic",
            serde_json::json!({"action":"path_batch_facts","paths":["README.md"]}),
            "fs_basic.stat_paths",
        ),
        (
            "archive_basic",
            serde_json::json!({"action":"list","archive":"tmp/test_bundle.zip"}),
            "archive_basic.list",
        ),
        (
            "archive_basic",
            serde_json::json!({"action":"read","archive":"tmp/test_bundle.zip","member":"notes.txt"}),
            "archive_basic.read",
        ),
        (
            "db_basic",
            serde_json::json!({"action":"list_tables","db_path":"data/test_contract.sqlite"}),
            "db_basic.list_tables",
        ),
    ] {
        let policy = action_policy_for_output_contract(Some(&output_contract), skill, &args)
            .expect("runtime-equivalent policy decision");
        assert!(policy.is_allowed(), "{policy:?}");
        assert!(policy.action_matches_preferred(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "command_output_summary");
    }
    let evidence = matched.evidence_expression();
    assert!(evidence.any_of.contains(&"exists".to_string()));
    assert!(evidence.any_of.contains(&"path".to_string()));
}

#[test]
fn content_excerpt_summary_allows_runtime_equivalent_config_guard() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "config_edit",
        &serde_json::json!({
            "action": "guard_config",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("content excerpt contract should classify config guard");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn archive_list_allows_compound_readonly_archive_and_db_observations() {
    for (capability_ref, skill, args, expected_action, expected_evidence) in [
        (
            "capability_ref=archive.list",
            "archive_basic",
            serde_json::json!({"action":"list","archive":"tmp/test_bundle.zip"}),
            "archive_basic.list",
            vec!["candidates"],
        ),
        (
            "capability_ref=archive.read",
            "archive_basic",
            serde_json::json!({"action":"read","archive":"tmp/test_bundle.zip","member":"notes.txt"}),
            "archive_basic.read",
            vec!["content_excerpt"],
        ),
        (
            "capability_ref=database.list_tables",
            "db_basic",
            serde_json::json!({"action":"list_tables","db_path":"data/test_contract.sqlite"}),
            "db_basic.list_tables",
            vec!["candidates"],
        ),
    ] {
        let route = route_with_machine_capability_ref(capability_ref);
        let policy = action_policy_for_route(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("route policy decision for {expected_action}"));
        assert!(policy.is_allowed(), "{policy:?}");
        assert!(policy.action_matches_preferred(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "capability_ref");
        assert_eq!(policy.required_evidence, expected_evidence);
    }
}

#[test]
fn service_status_allows_http_basic_observation() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ServiceStatus)
        .expect("service status contract");
    let matched = MatchedContract::Semantic(contract);
    let action_ref = ActionRef::parse("http_basic.get").expect("action parses");

    assert_eq!(
        matched.action_policy(&action_ref),
        ActionPolicyDecision::Allowed
    );
}

#[test]
fn rss_news_fetch_allows_rss_fetch_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RssNewsFetch,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "rss_fetch",
        &serde_json::json!({"action":"latest","category":"general","limit":3}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "rss_fetch.latest");
    assert_eq!(policy.contract_match, "none");
}

#[test]
fn execution_failed_step_contract_accepts_command_output_evidence() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ExecutionFailedStep)
        .expect("execution failed step contract");
    let matched = MatchedContract::Semantic(contract);
    let evidence_expression = matched.evidence_expression();

    assert_eq!(matched.required_evidence(), vec!["command_output"]);
    for token in ["command_output", "field_value"] {
        assert!(
            evidence_expression.any_of.contains(&token.to_string()),
            "missing {token} in {evidence_expression:?}"
        );
    }
}

#[test]
fn service_status_allows_system_basic_info_observation() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ServiceStatus)
        .expect("service status contract");
    let matched = MatchedContract::Semantic(contract);
    let action_ref = ActionRef::parse("system_basic.info").expect("action parses");

    assert_eq!(
        matched.action_policy(&action_ref),
        ActionPolicyDecision::Allowed
    );
}

fn load_registry_from_text(raw: &str) -> SkillsRegistry {
    let path = std::env::temp_dir().join(format!(
        "contract_matrix_test_registry_{}_{}.toml",
        std::process::id(),
        fnv1a_hex(raw)
    ));
    std::fs::write(&path, raw).expect("write registry fixture");
    let registry = SkillsRegistry::load_from_path(&path).expect("load registry fixture");
    let _ = std::fs::remove_file(path);
    registry
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GeneratedContractMatch {
    Semantic(OutputSemanticKind),
    Generic(String),
}

#[derive(Debug, Clone)]
struct GeneratedMatrixCase {
    id: String,
    matched: GeneratedContractMatch,
    action: Option<ActionRef>,
    expected_decision: Option<ActionPolicyDecision>,
    expected_required_evidence: Vec<String>,
    expected_final_answer_shape: String,
}

fn generated_allowed_action(matched: &MatchedContract<'_>) -> Option<ActionRef> {
    for raw in matched.allowed_actions() {
        let action = ActionRef::parse(raw)?;
        if matched.action_policy(&action) == ActionPolicyDecision::Allowed {
            return Some(action);
        }
    }
    if matches!(
        matched,
        MatchedContract::Semantic(MatrixContract {
            none_passthrough: true,
            ..
        })
    ) {
        return ActionRef::parse("respond");
    }
    None
}

fn generated_negative_action(
    matched: &MatchedContract<'_>,
) -> Option<(ActionRef, ActionPolicyDecision)> {
    for raw in matched.forbidden_actions() {
        let action = ActionRef::parse(raw)?;
        let decision = matched.action_policy(&action);
        if decision != ActionPolicyDecision::Allowed {
            return Some((action, decision));
        }
    }

    let probes = [
        "run_cmd",
        "fs_basic.list_dir",
        "fs_basic.read_text_range",
        "fs_basic.write_text",
        "archive_basic.pack",
        "config_basic.validate",
        "docker_basic",
        "package_manager.detect",
        "db_basic",
        "health_check",
        "respond",
    ];
    for probe in probes {
        let action = ActionRef::parse(probe).expect("probe action parses");
        let decision = matched.action_policy(&action);
        if decision != ActionPolicyDecision::Allowed {
            return Some((action, decision));
        }
    }
    None
}

fn push_generated_case(
    cases: &mut Vec<GeneratedMatrixCase>,
    id: String,
    matched: GeneratedContractMatch,
    contract: &MatchedContract<'_>,
    action: Option<ActionRef>,
    expected_decision: Option<ActionPolicyDecision>,
) {
    cases.push(GeneratedMatrixCase {
        id,
        matched,
        action,
        expected_decision,
        expected_required_evidence: contract.required_evidence(),
        expected_final_answer_shape: contract.final_answer_shape().to_string(),
    });
}

fn generated_contract_cases(
    matrix: &ContractMatrix,
    minimum_count: usize,
) -> Vec<GeneratedMatrixCase> {
    let mut cases = Vec::new();

    for kind in OutputSemanticKind::ALL {
        let contract = matrix
            .semantic_contract(*kind)
            .expect("semantic contract exists");
        let matched = MatchedContract::Semantic(contract);
        let case_match = GeneratedContractMatch::Semantic(*kind);
        let prefix = kind.as_str();

        push_generated_case(
            &mut cases,
            format!("{prefix}::evidence_shape"),
            case_match.clone(),
            &matched,
            None,
            None,
        );

        if let Some(action) = generated_allowed_action(&matched) {
            let decision = matched.action_policy(&action);
            push_generated_case(
                &mut cases,
                format!("{prefix}::allowed::{}", action.as_key()),
                case_match.clone(),
                &matched,
                Some(action),
                Some(decision),
            );
        }

        if let Some((action, decision)) = generated_negative_action(&matched) {
            push_generated_case(
                &mut cases,
                format!("{prefix}::negative::{}", action.as_key()),
                case_match,
                &matched,
                Some(action),
                Some(decision),
            );
        }
    }

    for profile in &matrix.generic_profiles {
        let matched = MatchedContract::Generic(profile);
        let case_match = GeneratedContractMatch::Generic(profile.name.clone());
        let prefix = format!("generic::{}", profile.name);

        push_generated_case(
            &mut cases,
            format!("{prefix}::evidence_shape"),
            case_match.clone(),
            &matched,
            None,
            None,
        );

        if let Some(action) = generated_allowed_action(&matched) {
            let decision = matched.action_policy(&action);
            push_generated_case(
                &mut cases,
                format!("{prefix}::allowed::{}", action.as_key()),
                case_match.clone(),
                &matched,
                Some(action),
                Some(decision),
            );
        }

        if let Some((action, decision)) = generated_negative_action(&matched) {
            push_generated_case(
                &mut cases,
                format!("{prefix}::negative::{}", action.as_key()),
                case_match,
                &matched,
                Some(action),
                Some(decision),
            );
        }
    }

    assert!(
        cases.len() >= minimum_count,
        "matrix generated only {} cases, expected at least {minimum_count}",
        cases.len()
    );
    cases
}

fn matched_for_generated_case<'a>(
    matrix: &'a ContractMatrix,
    case: &GeneratedMatrixCase,
) -> MatchedContract<'a> {
    match &case.matched {
        GeneratedContractMatch::Semantic(kind) => MatchedContract::Semantic(
            matrix
                .semantic_contract(*kind)
                .expect("semantic contract exists"),
        ),
        GeneratedContractMatch::Generic(name) => MatchedContract::Generic(
            matrix
                .generic_profiles
                .iter()
                .find(|profile| profile.name == *name)
                .expect("generic profile exists"),
        ),
    }
}

#[test]
fn workspace_contract_matrix_loads_and_has_shape() {
    let matrix = load_workspace_matrix();

    assert!(matrix.validate_shape().is_empty());
    assert_eq!(matrix.schema_version, 1);
    assert!(!matrix.matrix_version_hash().is_empty());
    assert!(matrix
        .failure_attribution
        .contains(&"model_error".to_string()));
    assert_eq!(matrix.policy.unknown_semantic, "reject");
    assert_eq!(
        matrix.trace_policy.evidence_storage,
        "redacted_excerpt_hash"
    );
    assert_eq!(
        matrix.trace_policy.provider_evidence_view,
        "provider_safe_redacted"
    );
    let photo = matrix
        .contracts
        .get("photo_organization")
        .expect("photo organization transitional contract");
    assert_eq!(
        photo.migration_status,
        "transitional_capability_owned_evidence_pending"
    );
    assert_eq!(photo.migration_owner, "photo_organize.planner_capabilities");
}

#[test]
fn delete_contracts_cannot_be_satisfied_by_read_or_list_actions() {
    let matrix = load_workspace_matrix();
    for (name, contract) in &matrix.contracts {
        assert_delete_policy_for_actions(
            &format!("contract `{name}`"),
            &contract.operation,
            &contract.allowed_actions,
        );
    }
    for profile in &matrix.generic_profiles {
        for raw in &profile.allowed_actions {
            let Some(action) = ActionRef::parse(raw) else {
                continue;
            };
            assert!(
                !action_is_delete_mutation(&action),
                "generic profile `{}` allows delete action `{}`",
                profile.name,
                action.as_key()
            );
        }
    }
}

fn assert_delete_policy_for_actions(context: &str, operation: &str, actions: &[String]) {
    let operation = normalize_action_token(operation);
    for raw in actions {
        let Some(action) = ActionRef::parse(raw) else {
            continue;
        };
        if operation == "delete" {
            assert!(
                !action_is_read_or_list_observation(&action),
                "{context} allows read/list observation action `{}` for delete operation",
                action.as_key()
            );
        } else if operation != "mutate" {
            assert!(
                !action_is_delete_mutation(&action),
                "{context} allows delete action `{}` without delete operation",
                action.as_key()
            );
        }
    }
}

fn action_is_delete_mutation(action: &ActionRef) -> bool {
    matches!(
        (action.skill.as_str(), action.action.as_deref()),
        ("fs_basic", Some("remove_path")) | ("remove_file", _)
    )
}

fn action_is_read_or_list_observation(action: &ActionRef) -> bool {
    matches!(
        (action.skill.as_str(), action.action.as_deref()),
        ("read_file" | "list_dir" | "doc_parse", _)
            | (
                "fs_basic",
                Some("list_dir" | "read_text_range" | "find_entries" | "grep_text")
            )
            | ("archive_basic", Some("list" | "read"))
            | (
                "config_basic",
                Some("read_field" | "read_fields" | "list_keys")
            )
            | ("db_basic", Some("list_tables" | "query"))
    )
}

#[test]
fn failure_attribution_enum_matches_workspace_matrix() {
    let matrix = load_workspace_matrix();
    let configured = matrix
        .failure_attribution
        .iter()
        .filter_map(|value| FailureAttribution::parse(value))
        .collect::<BTreeSet<_>>();
    let expected = FailureAttribution::ALL.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(configured, expected);
}

#[test]
fn failure_attribution_rejects_unknown_tokens() {
    let mut matrix = ContractMatrix {
        schema_version: 1,
        matrix_version: "test".to_string(),
        failure_attribution: FailureAttribution::ALL
            .iter()
            .map(|kind| kind.as_str().to_string())
            .chain(std::iter::once("mystery_bucket".to_string()))
            .collect(),
        ..Default::default()
    };
    matrix.trace_policy = MatrixTracePolicy {
        evidence_storage: "redacted_excerpt_hash".to_string(),
        provider_evidence_view: "provider_safe_redacted".to_string(),
        raw_excerpt_policy: "no_full_raw_excerpt".to_string(),
        max_items: 24,
        max_excerpt_chars: 240,
    };

    let errors = matrix.validate_shape();

    assert!(errors
        .iter()
        .any(|error| error == "invalid failure attribution `mystery_bucket`"));
}

#[test]
fn bundled_contract_matrix_result_exposes_load_errors() {
    let matrix = bundled_contract_matrix_result().expect("bundled matrix should load");

    assert_eq!(matrix.schema_version, 1);

    let err = parse_contract_matrix_source(
        r#"schema_version = 1
matrix_version = "broken"
"#,
    )
    .expect_err("invalid matrix should report a concrete error");
    assert!(err.contains("contract matrix shape invalid"));
    assert!(err.contains("missing failure attribution"));
}

#[test]
fn existence_contract_can_express_negative_evidence() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .semantic_contract(OutputSemanticKind::ExistenceWithPath)
        .expect("existence contract");
    let expression = contract.evidence_expression();

    assert_eq!(expression.all_of, vec!["kind", "path"]);
    assert_eq!(expression.one_of, vec!["exists_false", "exists_true"]);
    assert_eq!(expression.negative_evidence, vec!["exists_false"]);
}

#[test]
fn trace_snapshot_includes_evidence_expression_trace_policy_and_sources() {
    let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    })
    .expect("trace snapshot");

    assert_eq!(
        snapshot
            .get("trace_policy")
            .and_then(|value| value.get("provider_evidence_view"))
            .and_then(Value::as_str),
        Some("provider_safe_redacted")
    );
    assert_eq!(
        snapshot.get("policy_mode").and_then(Value::as_str),
        Some("enforce")
    );
    assert_eq!(
        snapshot.get("contract_marker").and_then(Value::as_str),
        Some("file_names")
    );
    assert!(snapshot.get("semantic_kind").is_none());
    assert_eq!(
        snapshot.get("evidence_scope").and_then(Value::as_str),
        Some("current_task")
    );
    assert_eq!(
        snapshot.get("freshness").and_then(Value::as_str),
        Some("current_task")
    );
    assert_eq!(
        snapshot.get("artifact_kind").and_then(Value::as_str),
        Some("text")
    );
    assert_eq!(
        snapshot.get("channel_visibility").and_then(Value::as_str),
        Some("user_visible")
    );
    assert_eq!(
        snapshot.get("evidence_profile").and_then(Value::as_str),
        Some("generic")
    );
    assert_eq!(
        snapshot
            .get("evidence_expression")
            .and_then(|value| value.get("all_of"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("candidates")
    );
    assert!(snapshot
        .get("observation_sources")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir"))));
    assert!(snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| {
            item.get("source").and_then(Value::as_str) == Some("fs_basic.list_dir")
                && item.get("extractor_kind").and_then(Value::as_str) == Some("structured_json")
        })));
}

#[test]
fn raw_command_observation_source_defaults_to_text_legacy_extractor() {
    let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        ..IntentOutputContract::default()
    })
    .expect("trace snapshot");

    assert!(snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| {
            item.get("source").and_then(Value::as_str) == Some("run_cmd")
                && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
        })));
}

#[test]
fn configured_legacy_text_observation_extractors_are_reflected_in_trace() {
    let scalar_snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..IntentOutputContract::default()
    })
    .expect("scalar trace snapshot");
    let scalar_extractors = scalar_snapshot
        .get("observation_extractors")
        .and_then(Value::as_array)
        .expect("scalar observation extractors");
    assert!(scalar_extractors.iter().any(|item| {
        item.get("source").and_then(Value::as_str) == Some("archive_basic")
            && item.get("extractor_kind").and_then(Value::as_str) == Some("text_legacy")
    }));

    let archive_route = route_with_machine_capability_ref("capability_ref=archive.list");
    let archive_trace = action_trace_for_route(&archive_route, "archive_basic.list")
        .expect("archive route action trace");
    assert_eq!(
        archive_trace
            .get("observation_extractor")
            .and_then(|item| item.get("source"))
            .and_then(Value::as_str),
        Some("archive_basic.list")
    );
    assert_eq!(
        archive_trace
            .get("observation_extractor")
            .and_then(|item| item.get("extractor_kind"))
            .and_then(Value::as_str),
        Some("structured_json")
    );
    assert_eq!(
        archive_trace.get("contract_match").and_then(Value::as_str),
        Some("capability_ref")
    );
}

#[test]
fn normalizer_schema_capability_bridges_fall_back_to_generic_contracts() {
    let matrix = load_workspace_matrix();

    for kind in OutputSemanticKind::ALL
        .iter()
        .filter(|kind| kind.is_normalizer_schema_capability_bridge())
    {
        let output_contract = IntentOutputContract {
            semantic_kind: *kind,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        };
        let matched = matrix
            .match_output_contract(&output_contract)
            .unwrap_or_else(|| panic!("generic match for {}", kind.as_str()));
        assert_ne!(
            matched.match_name(),
            kind.as_str(),
            "{} must not directly own matrix policy after normalizer bridge demotion",
            kind.as_str()
        );
    }
}

#[test]
fn final_shape_honors_scalar_hidden_entries_and_structured_key_verdicts() {
    assert_eq!(
        final_answer_shape_for_output_contract(&IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            ..IntentOutputContract::default()
        }),
        Some(FinalAnswerShape::Scalar)
    );
    assert_eq!(
        final_answer_shape_for_output_contract(&IntentOutputContract {
            response_shape: OutputResponseShape::Free,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            ..IntentOutputContract::default()
        }),
        Some(FinalAnswerShape::ValidationVerdict)
    );
}

#[test]
fn generic_delivery_snapshot_defaults_to_file_artifact() {
    let snapshot = trace_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::None,
        delivery_required: true,
        ..IntentOutputContract::default()
    })
    .expect("trace snapshot");

    assert_eq!(
        snapshot.get("contract_match").and_then(Value::as_str),
        Some("generic_delivery")
    );
    assert_eq!(
        snapshot.get("artifact_kind").and_then(Value::as_str),
        Some("file")
    );
}

#[test]
fn action_trace_records_contract_decision_and_shape() {
    let trace = action_trace_for_output_contract(
        &IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        },
        "fs_basic.list_dir",
    )
    .expect("action trace should resolve");

    assert_eq!(
        trace.get("contract_match").and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        trace.get("decision").and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        trace.get("final_answer_shape").and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        trace.get("evidence_profile").and_then(Value::as_str),
        Some("generic")
    );
    assert_eq!(
        trace
            .get("required_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["candidates"])
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/extractor_kind")
            .and_then(Value::as_str),
        Some("structured_json")
    );
}

#[test]
fn action_trace_marks_run_cmd_extractor_as_text_legacy() {
    let trace = action_trace_for_output_contract(
        &IntentOutputContract {
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            ..IntentOutputContract::default()
        },
        "run_cmd",
    )
    .expect("action trace should resolve");

    assert_eq!(
        trace.get("decision").and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/source")
            .and_then(Value::as_str),
        Some("run_cmd")
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/extractor_kind")
            .and_then(Value::as_str),
        Some("text_legacy")
    );
    assert_eq!(
        trace
            .pointer("/observation_extractor/registry/extractor_ref")
            .and_then(Value::as_str),
        Some("run_cmd.text_legacy_v1")
    );
}

#[test]
fn contract_runtime_fields_are_validated() {
    let err = parse_contract_matrix_source(
        r#"
schema_version = 1
matrix_version = "invalid-runtime-field"
failure_attribution = [
  "model_error",
  "schema_error",
  "code_gap",
  "contract_gap",
  "tool_gap",
  "permission_denied",
  "budget_exhausted",
  "prompt_budget_error",
  "delivery_error",
  "provider_error",
]

[trace_policy]
evidence_storage = "redacted_excerpt_hash"
provider_evidence_view = "provider_safe_redacted"
raw_excerpt_policy = "no_full_raw_excerpt"
max_items = 24
max_excerpt_chars = 240

[contracts.none]
semantic_kind = "none"
operation = "unknown"
target_object = "unknown"
delivery_shape = "summary"
policy_mode = "maybe"
allowed_actions = []
preferred_actions = []
forbidden_actions = []
required_evidence = []
final_answer_shape = "free"
none_passthrough = true
failure_policy = "no_retry"
"#,
    )
    .expect_err("invalid runtime field should fail shape validation");

    assert!(err.contains("invalid policy_mode"));
}

#[test]
fn contract_runtime_rejects_natural_language_evidence_profile() {
    let source = include_str!("../../../configs/task_contract_matrix.toml").replace(
        "evidence_profile = \"workspace_user_docs_first\"",
        "evidence_profile = \"read user setup docs first\"",
    );
    let err = parse_contract_matrix_source(&source)
        .expect_err("natural-language evidence profile should fail shape validation");

    assert!(err.contains("invalid evidence_profile"));
}

#[test]
fn configured_observation_extractors_must_exist_in_registry() {
    let source = format!(
            "{}\n[[contracts.service_status.observation_extractors]]\nsource = \"run_cmd\"\nextractor_kind = \"structured_json\"\n",
            include_str!("../../../configs/task_contract_matrix.toml")
        );
    let err = parse_contract_matrix_source(&source)
        .expect_err("unregistered explicit extractor should fail validation");

    assert!(err.contains(
            "observation_extractor source `run_cmd` with extractor_kind `structured_json` is not declared in the evidence extractor registry"
        ));
}

#[test]
fn runtime_contract_snapshot_binds_matrix_and_compact_prompt_block() {
    let snapshot = runtime_contract_snapshot_for_output_contract(&IntentOutputContract {
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    })
    .expect("runtime contract snapshot");

    assert_eq!(
        snapshot
            .get("matrix")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("bundled:configs/task_contract_matrix.toml")
    );
    assert!(snapshot
        .get("matrix")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(
        snapshot
            .get("registry")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("bundled:configs/skills_registry.toml")
    );
    assert!(snapshot
        .get("registry")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(
        snapshot
            .get("prompt_layer")
            .and_then(|value| value.get("source"))
            .and_then(Value::as_str),
        Some("bundled:prompts/layers/manifest.toml")
    );
    assert!(snapshot
        .get("prompt_layer")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert!(snapshot
        .get("compact_contract_block")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("final_answer_shape"))
            .and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("final_answer_shape_class"))
            .and_then(Value::as_str),
        Some("strict_list")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("coarse_response_shape"))
            .and_then(Value::as_str),
        Some("strict")
    );
    assert_eq!(
        snapshot
            .get("contract")
            .and_then(|value| value.get("allows_model_language"))
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[path = "contract_matrix_tests/runtime_policy_and_generic_profiles.rs"]
mod runtime_policy_and_generic_profiles;

#[path = "contract_matrix_tests/artifact_and_external_policy.rs"]
mod artifact_and_external_policy;

#[path = "contract_matrix_tests/action_policy_registry.rs"]
mod action_policy_registry;
