use super::{
    apply_action_effect_failure, apply_action_effect_success, assess_validation_output,
    classify_skill_action_effect, effective_action_effect_for_recipe,
    stop_signal_for_validation_failure, validation_satisfies_recipe_profile, ActionEffect,
    ExecutionRecipeKind, ExecutionRecipePhase, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
    ExecutionRecipeSpec, ExecutionRecipeTargetScope, ValidationObservation,
};
use crate::{AgentRuntimeConfig, AppState, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID};
use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

fn test_state() -> AppState {
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

fn test_state_with_registry(toml: &str, skills: &[&str]) -> AppState {
    let path = std::env::temp_dir().join(format!(
        "execution_recipe_registry_{}_{}_{}.toml",
        std::process::id(),
        crate::now_ts_u64(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_file(path);
    let mut state = test_state();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
}

#[test]
fn goal_overlay_includes_code_change_guidance_for_current_repo() {
    let overlay = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::CodeChange,
        target_scope: ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    })
    .goal_overlay()
    .expect("overlay");
    assert!(overlay.contains("profile=code_change"));
    assert!(overlay.contains("target_scope=current_repo"));
    assert!(overlay.contains("compile/test/lint/runtime evidence"));
    assert!(overlay.contains("current repository/workspace"));
}

#[test]
fn package_manager_dry_run_install_is_observe_effect() {
    let state = test_state();
    let dry_run = classify_skill_action_effect(
        &state,
        "package_manager",
        &json!({"action":"smart_install","packages":["jq"],"dry_run":true}),
    );
    assert!(dry_run.observes);
    assert!(!dry_run.mutates);

    let real_install = classify_skill_action_effect(
        &state,
        "package_manager",
        &json!({"action":"smart_install","packages":["jq"],"dry_run":false}),
    );
    assert!(real_install.mutates);
}

#[test]
fn goal_overlay_includes_skill_authoring_and_greenfield_guidance() {
    let overlay = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::SkillAuthoring,
        target_scope: ExecutionRecipeTargetScope::Greenfield,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    })
    .goal_overlay()
    .expect("overlay");
    assert!(overlay.contains("profile=skill_authoring"));
    assert!(overlay.contains("target_scope=greenfield"));
    assert!(overlay.contains("reusable skill/extension"));
    assert!(overlay.contains("minimal new files or scaffold"));
}

#[test]
fn classify_skill_action_effect_prefers_registry_planner_effect() {
    let state = test_state_with_registry(
        r#"
[[skills]]
name = "fs_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
planner_capabilities = [
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe" },
  { name = "filesystem.write_file", action = "write_text", effect = "mutate" },
  { name = "config.validate", action = "validate", effect = "validate" },
]
"#,
        &["fs_basic"],
    );
    let observe = classify_skill_action_effect(
        &state,
        "fs_basic",
        &json!({"action": "read_text_range", "path": "README.md"}),
    );
    assert!(observe.observes);
    assert!(!observe.mutates);
    assert!(!observe.validates);

    let mutate = classify_skill_action_effect(
        &state,
        "fs_basic",
        &json!({"action": "write_text", "path": "out.txt", "content": "hello"}),
    );
    assert!(mutate.mutates);
    assert!(!mutate.validates);

    let validate = classify_skill_action_effect(&state, "fs_basic", &json!({"action": "validate"}));
    assert!(validate.observes);
    assert!(!validate.mutates);
    assert!(validate.validates);
}

#[test]
fn registry_observe_http_get_with_expectation_becomes_validation() {
    let state = test_state_with_registry(
        r#"
[[skills]]
name = "http_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
planner_capabilities = [
  { name = "http.get", action = "get", effect = "observe" },
]
"#,
        &["http_basic"],
    );

    let plain = classify_skill_action_effect(
        &state,
        "http_basic",
        &json!({"action": "get", "url": "http://127.0.0.1:12345/"}),
    );
    assert!(plain.observes);
    assert!(!plain.mutates);
    assert!(!plain.validates);

    let validation = classify_skill_action_effect(
        &state,
        "http_basic",
        &json!({
            "action": "get",
            "url": "http://127.0.0.1:12345/",
            "expect_contains": "VALIDATION_PASSED"
        }),
    );
    assert!(validation.observes);
    assert!(!validation.mutates);
    assert!(validation.validates);
}

#[test]
fn classify_skill_action_effect_uses_registry_read_only_fallback_before_skill_name_branch() {
    let state = test_state_with_registry(
        r#"
[[skills]]
name = "custom_readonly"
enabled = true
kind = "builtin"
planner_kind = "tool"
side_effect = false
"#,
        &["custom_readonly"],
    );
    let effect = classify_skill_action_effect(
        &state,
        "custom_readonly",
        &json!({"action": "inspect_status"}),
    );
    assert!(effect.observes);
    assert!(!effect.mutates);
    assert!(!effect.validates);
}

#[test]
fn classify_run_cmd_restart_as_mutation() {
    let state = test_state();
    let effect = classify_skill_action_effect(
        &state,
        "run_cmd",
        &json!({"command":"systemctl restart sing-box"}),
    );
    assert!(effect.mutates);
    assert!(!effect.validates);
}

#[test]
fn classify_run_cmd_combined_mutation_and_validation() {
    let state = test_state();
    let effect = classify_skill_action_effect(
        &state,
        "run_cmd",
        &json!({"command":"cd /tmp/demo && python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 & sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED"}),
    );
    assert!(effect.mutates);
    assert!(effect.validates);
}

#[test]
fn structured_validation_marks_custom_run_cmd_as_code_validation() {
    let state = test_state();
    let args = json!({
        "command": "bash /tmp/rustclaw-validation-case/check.sh",
        "_clawd_validation": {
            "profile": "code_change",
            "validator_type": "custom",
            "validated_target": "/tmp/rustclaw-validation-case"
        }
    });
    let effect = classify_skill_action_effect(&state, "run_cmd", &args);
    assert!(effect.observes);
    assert!(effect.validates);
    assert!(!effect.mutates);

    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::CodeChange,
        target_scope: ExecutionRecipeTargetScope::ExternalWorkspace,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    });
    assert!(validation_satisfies_recipe_profile(
        recipe, &state, "run_cmd", &args
    ));
}

#[test]
fn structured_validation_success_fallback_accepts_custom_command_output() {
    let state = test_state();
    let args = json!({
        "command": "bash /tmp/rustclaw-validation-case/check.sh",
        "_clawd_validation": {
            "profile": "code_change",
            "validator_type": "custom",
            "validated_target": "/tmp/rustclaw-validation-case"
        }
    });
    let observation = assess_validation_output(&state, "run_cmd", &args, "OK\n");
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn code_change_run_cmd_build_or_test_satisfies_recipe_profile() {
    let state = test_state();
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::CodeChange,
        target_scope: ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    });
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "cargo test -p clawd"})
    ));
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "npm run build"})
    ));
    assert!(!validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "cat README.md"})
    ));
}

#[test]
fn skill_authoring_run_cmd_build_or_test_satisfies_recipe_profile() {
    let state = test_state();
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::SkillAuthoring,
        target_scope: ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    });
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "cargo check -p skill-runner"})
    ));
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "python3 -m pytest tests"})
    ));
    assert!(!validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "ls crates/skills"})
    ));
}

#[test]
fn parse_execution_recipe_profile_accepts_package_and_database_tokens() {
    assert_eq!(
        super::parse_execution_recipe_profile_text("package_change"),
        ExecutionRecipeProfile::PackageChange
    );
    assert_eq!(
        super::parse_execution_recipe_profile_text("dependency_change"),
        ExecutionRecipeProfile::PackageChange
    );
    assert_eq!(
        super::parse_execution_recipe_profile_text("database_change"),
        ExecutionRecipeProfile::DatabaseChange
    );
    assert_eq!(
        super::parse_execution_recipe_profile_text("schema_change"),
        ExecutionRecipeProfile::DatabaseChange
    );
}

#[test]
fn config_change_accepts_structured_config_and_run_cmd_validation() {
    let state = test_state();
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::ConfigChange,
        target_scope: ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    });
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "config_edit",
        &json!({"action": "validate_config", "path": "configs/config.toml"})
    ));
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "config_edit",
        &json!({"action": "read_back", "path": "configs/config.toml", "field_path": "tools.allow_sudo"})
    ));
    assert!(validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "nginx -t"})
    ));
    assert!(!validation_satisfies_recipe_profile(
        recipe,
        &state,
        "run_cmd",
        &json!({"command": "cat configs/config.toml"})
    ));
}

#[test]
fn package_and_database_profiles_require_matching_validation() {
    let state = test_state();
    let package_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::PackageChange,
        target_scope: ExecutionRecipeTargetScope::System,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    });
    assert!(validation_satisfies_recipe_profile(
        package_recipe,
        &state,
        "run_cmd",
        &json!({"command": "cargo build"})
    ));
    assert!(validation_satisfies_recipe_profile(
        package_recipe,
        &state,
        "package_manager",
        &json!({"action": "detect"})
    ));
    assert!(!validation_satisfies_recipe_profile(
        package_recipe,
        &state,
        "package_manager",
        &json!({"action": "install", "package": "jq"})
    ));

    let database_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        profile: ExecutionRecipeProfile::DatabaseChange,
        target_scope: ExecutionRecipeTargetScope::CurrentRepo,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
    });
    assert!(validation_satisfies_recipe_profile(
        database_recipe,
        &state,
        "db_basic",
        &json!({"action": "schema_version", "db_path": "data/app.db"})
    ));
    assert!(validation_satisfies_recipe_profile(
        database_recipe,
        &state,
        "db_basic",
        &json!({"action": "sqlite_query", "db_path": "data/app.db", "sql": "SELECT 1"})
    ));
    assert!(!validation_satisfies_recipe_profile(
        database_recipe,
        &state,
        "db_basic",
        &json!({"action": "sqlite_execute", "db_path": "data/app.db", "sql": "UPDATE users SET active=1", "confirm": true})
    ));
}

#[test]
fn structured_validation_success_marker_accepts_matching_output() {
    let state = test_state();
    let args = json!({
        "command": "bash /tmp/rustclaw-validation-case/check.sh",
        "_clawd_validation": {
            "profile": "code_change",
            "validator_type": "custom",
            "validated_target": "/tmp/rustclaw-validation-case",
            "success_marker": {
                "marker": "OK",
                "match_mode": "contains",
                "case_sensitive": true
            }
        }
    });
    let observation = assess_validation_output(&state, "run_cmd", &args, "script says OK\n");
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn structured_validation_success_marker_rejects_missing_output_marker() {
    let state = test_state();
    let args = json!({
        "command": "bash /tmp/rustclaw-validation-case/check.sh",
        "_clawd_validation": {
            "profile": "code_change",
            "validator_type": "custom",
            "validated_target": "/tmp/rustclaw-validation-case",
            "success_marker": {
                "marker": "OK",
                "match_mode": "equals"
            }
        }
    });
    let observation = assess_validation_output(&state, "run_cmd", &args, "DONE\n");
    assert_eq!(
        observation,
        ValidationObservation::Failed("validation output missing required marker=OK".to_string())
    );
}

#[test]
fn structured_validation_result_from_skill_output_takes_precedence() {
    let state = test_state();
    let args = json!({
        "command": "bash /tmp/rustclaw-validation-case/check.sh",
        "_clawd_validation": {
            "profile": "code_change",
            "validator_type": "custom",
            "validated_target": "/tmp/rustclaw-validation-case"
        }
    });
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &args,
        r#"{"validation":{"result":"failed","detail":"expected marker missing"}}"#,
    );
    assert_eq!(
        observation,
        ValidationObservation::Failed("expected marker missing".to_string())
    );
}

#[test]
fn split_combined_run_cmd_into_mutate_and_validate_parts() {
    let command = "cd /tmp/demo && nohup python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 & sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED";
    let (mutate_part, validate_part) =
        super::split_run_cmd_mutation_and_validation(command).expect("split combined command");
    assert_eq!(
            mutate_part,
            "cd /tmp/demo && nohup python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 &"
        );
    assert_eq!(
            validate_part,
            "sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED"
        );
}

#[test]
fn classify_service_status_as_validation() {
    let state = test_state();
    let effect = classify_skill_action_effect(
        &state,
        "service_control",
        &json!({"action":"status","target":"sing-box"}),
    );
    assert!(effect.observes);
    assert!(effect.validates);
}

#[test]
fn validation_failure_moves_recipe_to_repair() {
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    apply_action_effect_success(&mut recipe, ActionEffect::observe());
    apply_action_effect_success(&mut recipe, ActionEffect::mutate());
    assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
    apply_action_effect_failure(&mut recipe, ActionEffect::validate());
    assert_eq!(recipe.phase, ExecutionRecipePhase::Repair);
    assert_eq!(recipe.repair_count, 1);
}

#[test]
fn combined_mutate_and_validate_marks_recipe_done() {
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    apply_action_effect_success(&mut recipe, ActionEffect::observe());
    apply_action_effect_success(
        &mut recipe,
        ActionEffect {
            observes: true,
            mutates: true,
            validates: true,
        },
    );
    assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
    assert!(recipe.saw_validation);
}

#[test]
fn service_control_stopped_status_is_validation_failure() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "service_control",
        &json!({"action":"status","target":"telegramd"}),
        r#"{"status":"ok","service_name":"telegramd","requested_action":"status","pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"summary":"Status: telegramd=stopped"}"#,
    );
    assert!(matches!(observation, ValidationObservation::Failed(_)));
}

#[test]
fn service_control_verify_running_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "service_control",
        &json!({"action":"verify","target":"telegramd"}),
        r#"{"status":"ok","service_name":"telegramd","requested_action":"verify","post_state":"running","verified":true,"summary":"Verify: running"}"#,
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn health_check_with_closed_port_is_validation_failure() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "health_check",
        &json!({}),
        r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":false}"#,
    );
    assert!(matches!(observation, ValidationObservation::Failed(_)));
}

#[test]
fn run_cmd_active_output_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"systemctl is-active sing-box"}),
        "active\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_inactive_output_is_validation_failure() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"systemctl status sing-box"}),
        "inactive (dead)\n",
    );
    assert!(matches!(observation, ValidationObservation::Failed(_)));
}

#[test]
fn run_cmd_sing_box_check_exit_zero_sentinel_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"sing-box check -c /tmp/config.json"}),
        "exit=0 command=sing-box check -c /tmp/config.json",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_nginx_test_ok_output_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"nginx -t"}),
            "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful",
        );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_ss_without_rows_is_validation_failure() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"ss -ltn sport = :8787"}),
        "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess",
    );
    assert!(matches!(observation, ValidationObservation::Failed(_)));
}

#[test]
fn run_cmd_ss_with_listener_row_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
            &state,
            "run_cmd",
            &json!({"command":"ss -ltn sport = :8787"}),
            "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0      128    127.0.0.1:8787      0.0.0.0:*",
        );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_curl_exit_zero_sentinel_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"curl -fsS http://127.0.0.1:8787/v1/health -o /dev/null"}),
        "exit=0 command=curl -fsS http://127.0.0.1:8787/v1/health -o /dev/null",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_curl_validation_marker_output_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"curl -s http://127.0.0.1:8787/ | grep -q 'ops-demo-ok' && echo VALIDATION_PASSED || echo VALIDATION_FAILED"}),
        "VALIDATION_PASSED\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_curl_grep_match_output_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"curl -s http://127.0.0.1:8787/ | grep -o 'ops-demo-ok'"}),
        "ops-demo-ok\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn run_cmd_validation_marker_output_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "run_cmd",
        &json!({"command":"python3 -m http.server 65429 --bind 127.0.0.1 > /tmp/http.log 2>&1 & sleep 2 && curl -s http://127.0.0.1:65429/ | grep -q ops-demo-ok && echo VALIDATION_PASSED || echo VALIDATION_FAILED"}),
        "VALIDATION_PASSED\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn http_basic_2xx_is_validation_pass() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "http_basic",
        &json!({"action":"get","url":"http://127.0.0.1:8080/health"}),
        "status=200\n{\"ok\":true}\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn http_basic_without_expectation_is_observation_not_validation() {
    let state = test_state();
    assert_eq!(
        classify_skill_action_effect(
            &state,
            "http_basic",
            &json!({"action":"get","url":"http://127.0.0.1:8080/no_such_path"}),
        ),
        ActionEffect::observe()
    );
    let observation = assess_validation_output(
        &state,
        "http_basic",
        &json!({"action":"get","url":"http://127.0.0.1:8080/no_such_path"}),
        "status=404\nnot found\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn http_basic_missing_expected_content_is_validation_fail() {
    let state = test_state();
    let observation = assess_validation_output(
        &state,
        "http_basic",
        &json!({
            "action":"get",
            "url":"http://127.0.0.1:8080/health",
            "expect_contains":"ops-repair-ok"
        }),
        "status=200\nops-repair-bad\n",
    );
    assert!(matches!(observation, ValidationObservation::Failed(_)));
}

#[test]
fn http_basic_expected_status_allows_non_success_response() {
    let state = test_state();
    assert_eq!(
        classify_skill_action_effect(
            &state,
            "http_basic",
            &json!({
                "action":"get",
                "url":"http://127.0.0.1:8080/no_such_path",
                "expect_status":404
            }),
        ),
        ActionEffect::validate()
    );
    let observation = assess_validation_output(
        &state,
        "http_basic",
        &json!({
            "action":"get",
            "url":"http://127.0.0.1:8080/no_such_path",
            "expect_status":404
        }),
        "status=404\nnot found\n",
    );
    assert_eq!(observation, ValidationObservation::Passed);
}

#[test]
fn http_basic_expect_success_fails_non_success_response() {
    let state = test_state();
    assert_eq!(
        classify_skill_action_effect(
            &state,
            "http_basic",
            &json!({
                "action":"get",
                "url":"http://127.0.0.1:8080/no_such_path",
                "expect_success":true
            }),
        ),
        ActionEffect::validate()
    );
    let observation = assess_validation_output(
        &state,
        "http_basic",
        &json!({
            "action":"get",
            "url":"http://127.0.0.1:8080/no_such_path",
            "expect_success":true
        }),
        "status=404\nnot found\n",
    );
    assert!(matches!(observation, ValidationObservation::Failed(_)));
}

#[test]
fn repair_budget_exhausted_after_limit() {
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    recipe.saw_mutation = true;
    apply_action_effect_failure(&mut recipe, ActionEffect::validate());
    assert_eq!(
        stop_signal_for_validation_failure(&recipe),
        "recoverable_failure_continue_round"
    );
    apply_action_effect_failure(&mut recipe, ActionEffect::validate());
    assert_eq!(
        stop_signal_for_validation_failure(&recipe),
        "recoverable_failure_continue_round"
    );
    apply_action_effect_failure(&mut recipe, ActionEffect::validate());
    assert_eq!(
        stop_signal_for_validation_failure(&recipe),
        "recipe_repair_budget_exhausted"
    );
}

#[test]
fn pre_mutation_validation_is_treated_as_inspect() {
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    let effect = effective_action_effect_for_recipe(recipe, ActionEffect::validate());
    assert!(effect.observes);
    assert!(!effect.validates);
}

#[test]
fn pre_mutation_validation_failure_advances_to_apply_phase() {
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    let effect = effective_action_effect_for_recipe(recipe, ActionEffect::validate());
    apply_action_effect_failure(&mut recipe, effect);
    assert!(recipe.saw_inspect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
    assert_eq!(recipe.repair_count, 0);
}

#[test]
fn pre_mutation_combined_mutate_and_validate_keeps_mutation() {
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    let effect = effective_action_effect_for_recipe(
        recipe,
        ActionEffect {
            observes: true,
            mutates: true,
            validates: true,
        },
    );
    assert!(effect.mutates);
    assert!(effect.validates);
}

#[test]
fn failed_http_preflight_then_repair_mutate_then_validate_passes() {
    let state = test_state();
    let validate_args = json!({
        "action":"get",
        "url":"http://127.0.0.1:51179/",
        "expect_contains":"ops-repair-ok"
    });
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });

    let preflight_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "http_basic", &validate_args),
    );
    let preflight_observation = assess_validation_output(
        &state,
        "http_basic",
        &validate_args,
        "status=200\nops-repair-bad\n",
    );
    assert!(matches!(
        preflight_observation,
        ValidationObservation::Failed(_)
    ));
    apply_action_effect_failure(&mut recipe, preflight_effect);
    assert_eq!(
        stop_signal_for_validation_failure(&recipe),
        "recoverable_failure_continue_round"
    );
    assert!(recipe.saw_inspect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
    assert_eq!(recipe.repair_count, 0);

    let inspect_effect = classify_skill_action_effect(
        &state,
        "read_file",
        &json!({"path":"document/nl_ops_http_demo/index.html"}),
    );
    apply_action_effect_success(&mut recipe, inspect_effect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);

    let mutate_effect = classify_skill_action_effect(
        &state,
        "write_file",
        &json!({
            "path":"document/nl_ops_http_demo/index.html",
            "content":"ops-repair-ok\n"
        }),
    );
    apply_action_effect_success(&mut recipe, mutate_effect);
    assert!(recipe.saw_mutation);
    assert!(!recipe.saw_validation);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
    assert!(recipe.needs_validation());

    let post_repair_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "http_basic", &validate_args),
    );
    let post_repair_observation = assess_validation_output(
        &state,
        "http_basic",
        &validate_args,
        "status=200\nops-repair-ok\n",
    );
    assert_eq!(post_repair_observation, ValidationObservation::Passed);
    assert!(post_repair_effect.validates);
    apply_action_effect_success(&mut recipe, post_repair_effect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
    assert!(recipe.saw_validation);
    assert!(!recipe.needs_validation());
    assert_eq!(recipe.repair_count, 0);
}

#[test]
fn failed_service_status_preflight_then_restart_then_verify_passes() {
    let state = test_state();
    let status_args = json!({"command":"systemctl status sing-box"});
    let verify_args = json!({"command":"systemctl is-active sing-box"});
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });

    let preflight_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "run_cmd", &status_args),
    );
    let preflight_observation =
        assess_validation_output(&state, "run_cmd", &status_args, "inactive (dead)\n");
    assert!(matches!(
        preflight_observation,
        ValidationObservation::Failed(_)
    ));
    assert!(preflight_effect.observes);
    assert!(!preflight_effect.validates);
    apply_action_effect_failure(&mut recipe, preflight_effect);
    assert!(recipe.saw_inspect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
    assert_eq!(recipe.repair_count, 0);

    let mutate_effect = classify_skill_action_effect(
        &state,
        "run_cmd",
        &json!({"command":"systemctl restart sing-box"}),
    );
    apply_action_effect_success(&mut recipe, mutate_effect);
    assert!(recipe.saw_mutation);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
    assert!(recipe.needs_validation());

    let verify_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "run_cmd", &verify_args),
    );
    let verify_observation = assess_validation_output(&state, "run_cmd", &verify_args, "active\n");
    assert_eq!(verify_observation, ValidationObservation::Passed);
    assert!(verify_effect.validates);
    apply_action_effect_success(&mut recipe, verify_effect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
    assert!(recipe.saw_validation);
    assert!(!recipe.needs_validation());
    assert_eq!(recipe.repair_count, 0);
}

#[test]
fn failed_run_cmd_validation_then_repair_and_validate_passes() {
    let state = test_state();
    let preflight_args = json!({
        "command":"grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo VALIDATION_PASSED || echo VALIDATION_FAILED"
    });
    let combined_repair = "printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html & sleep 1 && grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo VALIDATION_PASSED || echo VALIDATION_FAILED";
    let (mutate_part, validate_part) =
        super::split_run_cmd_mutation_and_validation(combined_repair)
            .expect("split repair command");
    let mutate_args = json!({ "command": mutate_part });
    let validate_args = json!({ "command": validate_part });
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });

    let preflight_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "run_cmd", &preflight_args),
    );
    let preflight_observation =
        assess_validation_output(&state, "run_cmd", &preflight_args, "VALIDATION_FAILED\n");
    assert!(matches!(
        preflight_observation,
        ValidationObservation::Failed(_)
    ));
    assert!(preflight_effect.observes);
    assert!(!preflight_effect.validates);
    apply_action_effect_failure(&mut recipe, preflight_effect);
    assert!(recipe.saw_inspect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Apply);
    assert_eq!(recipe.repair_count, 0);

    let mutate_effect = classify_skill_action_effect(&state, "run_cmd", &mutate_args);
    apply_action_effect_success(&mut recipe, mutate_effect);
    assert!(recipe.saw_mutation);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);

    let failed_validate_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "run_cmd", &validate_args),
    );
    let failed_validate_observation =
        assess_validation_output(&state, "run_cmd", &validate_args, "VALIDATION_FAILED\n");
    assert!(matches!(
        failed_validate_observation,
        ValidationObservation::Failed(_)
    ));
    assert!(failed_validate_effect.validates);
    apply_action_effect_failure(&mut recipe, failed_validate_effect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Repair);
    assert_eq!(recipe.repair_count, 1);
    assert!(recipe.needs_validation());

    let retry_mutate_effect = classify_skill_action_effect(&state, "run_cmd", &mutate_args);
    apply_action_effect_success(&mut recipe, retry_mutate_effect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Validate);
    assert!(!recipe.saw_validation);

    let passed_validate_effect = effective_action_effect_for_recipe(
        recipe,
        classify_skill_action_effect(&state, "run_cmd", &validate_args),
    );
    let passed_validate_observation =
        assess_validation_output(&state, "run_cmd", &validate_args, "VALIDATION_PASSED\n");
    assert_eq!(passed_validate_observation, ValidationObservation::Passed);
    apply_action_effect_success(&mut recipe, passed_validate_effect);
    assert_eq!(recipe.phase, ExecutionRecipePhase::Done);
    assert!(recipe.saw_validation);
    assert_eq!(recipe.repair_count, 1);
    assert!(!recipe.needs_validation());
}
