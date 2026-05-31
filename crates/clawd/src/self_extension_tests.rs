use super::{
    effective_request, extract_best_execution_output, handle_permanent_extension_with,
    handle_temporary_fix_with, localized_permanent_enable_failure,
    localized_permanent_materialization_failure, localized_permanent_plan_reply,
    localized_permanent_registration_failure, localized_permanent_reload_failure,
    localized_permanent_runtime_enabled_reply, localized_permanent_validation_failure,
    localized_plan_reply, self_extension_enabled_for_route,
    should_bypass_self_extension_for_execution_recipe, ReplyLanguage,
};
use claw_core::config::SelfExtensionConfig;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn self_extension_execution_prefers_last_non_empty_stdout() {
    let value = json!({
        "extra": {
            "command_runs": [
                {"stdout": "", "stderr": ""},
                {"stdout": "42\n", "stderr": ""}
            ]
        }
    });
    assert_eq!(extract_best_execution_output(&value).as_deref(), Some("42"));
}

#[test]
fn localized_plan_reply_mentions_disabled_package_install() {
    let plan = json!({
        "summary": "Write a small parser script.",
        "files": [{"path":"tmp/extension_manager/a.py"}],
        "commands": [{"runtime":"python3","script_path":"tmp/extension_manager/a.py"}],
        "packages": [{"ecosystem":"python","modules":["tomli"]}]
    });
    let reply = localized_plan_reply(None, ReplyLanguage::En, &plan, false, false);
    assert!(reply.contains("automatic package install is currently disabled"));
}

#[test]
fn self_extension_gating_requires_enabled_runtime_and_non_none_mode() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "do it with a temporary script".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            self_extension: crate::SelfExtensionContract {
                mode: crate::SelfExtensionMode::TemporaryFix,
                trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                execute_now: true,
            },
            ..Default::default()
        },
    };
    assert!(!self_extension_enabled_for_route(false, false, &route));
    assert!(self_extension_enabled_for_route(true, false, &route));
}

#[test]
fn capability_gap_trigger_requires_auto_flag() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "handle this by extending the system".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            self_extension: crate::SelfExtensionContract {
                mode: crate::SelfExtensionMode::TemporaryFix,
                trigger: crate::SelfExtensionTrigger::CapabilityGap,
                execute_now: false,
            },
            ..Default::default()
        },
    };
    assert!(!self_extension_enabled_for_route(true, false, &route));
    assert!(self_extension_enabled_for_route(true, true, &route));
}

#[test]
fn ops_closed_loop_request_bypasses_self_extension() {
    assert!(should_bypass_self_extension_for_execution_recipe(Some(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
        }
    )));
}

#[test]
fn plain_temporary_fix_request_does_not_bypass_self_extension() {
    assert!(!should_bypass_self_extension_for_execution_recipe(Some(
        crate::execution_recipe::ExecutionRecipeSpec::default()
    )));
    assert!(!should_bypass_self_extension_for_execution_recipe(None));
}

#[test]
fn localized_permanent_plan_reply_mentions_skill_name() {
    let plan = json!({
        "skill_name": "pdf_compare",
        "capability_summary": "Compare PDFs and summarize differences.",
        "actions": ["compare", "summarize"],
        "rationale": "Reusable document workflow."
    });
    let reply = localized_permanent_plan_reply(None, ReplyLanguage::En, &plan, false);
    assert!(reply.contains("pdf_compare"));
    assert!(reply.contains("reusable capability"));
}

#[test]
fn localized_permanent_materialization_failure_mentions_scaffold() {
    let reply = localized_permanent_materialization_failure(
        None,
        ReplyLanguage::En,
        "pdf_compare",
        "write failed",
    );
    assert!(reply.contains("external_skills/pdf_compare"));
    assert!(reply.contains("write failed"));
}

#[test]
fn localized_permanent_validation_failure_mentions_validation_steps() {
    let reply = localized_permanent_validation_failure(
        None,
        ReplyLanguage::En,
        "pdf_compare",
        "cargo check failed",
    );
    assert!(reply.contains("external_skills/pdf_compare"));
    assert!(reply.contains("cargo check failed"));
}

#[test]
fn localized_permanent_runtime_enabled_reply_mentions_reload_completion() {
    let reply = localized_permanent_runtime_enabled_reply(None, ReplyLanguage::En, "pdf_compare");
    assert!(reply.contains("external_skills/pdf_compare"));
    assert!(reply.contains("visible to the runtime"));
}

#[test]
fn localized_permanent_registration_failure_mentions_runtime_block() {
    let reply = localized_permanent_registration_failure(
        None,
        ReplyLanguage::En,
        "pdf_compare",
        "registry write failed",
    );
    assert!(reply.contains("external_skills/pdf_compare"));
    assert!(reply.contains("registry write failed"));
}

#[test]
fn localized_permanent_enable_failure_mentions_release_build() {
    let reply = localized_permanent_enable_failure(
        None,
        ReplyLanguage::En,
        "pdf_compare",
        "release build failed",
    );
    assert!(reply.contains("release build failed"));
    assert!(reply.contains("runtime use"));
}

#[test]
fn localized_permanent_reload_failure_mentions_manual_reload() {
    let reply =
        localized_permanent_reload_failure(None, ReplyLanguage::En, "pdf_compare", "reload failed");
    assert!(reply.contains("reload failed"));
    assert!(reply.contains("restart clawd"));
}

#[test]
fn temporary_fix_without_execute_returns_plan_reply_and_single_plan_call() {
    let runtime = SelfExtensionConfig {
        enabled: true,
        allow_execute: false,
        ..Default::default()
    };
    let seen_actions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen_actions_closure = seen_actions.clone();
    let reply = run_async(handle_temporary_fix_with(
        None,
        None,
        &runtime,
        "Use a temporary script to parse the input.",
        true,
        ReplyLanguage::En,
        move |args| {
            seen_actions_closure.borrow_mut().push(
                args.get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
            std::future::ready(Ok(json!({
                "status": "ok",
                "text": "plan ready",
                "extra": {
                    "plan": {
                        "summary": "Write a parser script.",
                        "files": [{"path":"tmp/extension_manager/a.py"}],
                        "commands": [{"runtime":"python3","script_path":"tmp/extension_manager/a.py"}],
                        "packages": []
                    }
                }
            })))
        },
    ))
    .expect("temporary plan should succeed");

    assert_eq!(reply.text.contains("did not execute it yet"), true);
    assert_eq!(seen_actions.borrow().as_slice(), ["temporary_fix_plan"]);
}

#[test]
fn temporary_fix_missing_plan_uses_failure_contract_fallback() {
    let runtime = SelfExtensionConfig {
        enabled: true,
        allow_execute: true,
        ..Default::default()
    };
    let seen_actions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let seen_actions_closure = seen_actions.clone();
    let reply = run_async(handle_temporary_fix_with(
        None,
        None,
        &runtime,
        "Use a temporary script to parse the input.",
        true,
        ReplyLanguage::En,
        move |args| {
            seen_actions_closure.borrow_mut().push(
                args.get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
            std::future::ready(Ok(json!({
                "status": "ok",
                "text": "plan ready",
                "extra": {}
            })))
        },
    ))
    .expect("temporary missing-plan failure should be user-visible");

    assert!(reply.text.contains("controlled self-extension path"));
    assert!(reply.text.contains("missing temporary fix plan"));
    assert_eq!(seen_actions.borrow().as_slice(), ["temporary_fix_plan"]);
}

#[test]
fn temporary_fix_execute_returns_command_stdout_and_calls_execute() {
    let runtime = SelfExtensionConfig {
        enabled: true,
        allow_execute: true,
        ..Default::default()
    };
    let seen_actions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let responses: Rc<RefCell<Vec<serde_json::Value>>> = Rc::new(RefCell::new(vec![
        json!({
            "status": "ok",
            "text": "plan ready",
            "extra": {
                "plan": {
                    "summary": "Write a parser script.",
                    "files": [{"path":"tmp/extension_manager/a.py"}],
                    "commands": [{"runtime":"python3","script_path":"tmp/extension_manager/a.py"}],
                    "packages": []
                }
            }
        }),
        json!({
            "status": "ok",
            "text": "executed",
            "extra": {
                "command_runs": [
                    {"stdout":"", "stderr":""},
                    {"stdout":"parsed successfully\n", "stderr":""}
                ]
            }
        }),
    ]));
    let seen_actions_closure = seen_actions.clone();
    let responses_closure = responses.clone();
    let reply = run_async(handle_temporary_fix_with(
        None,
        None,
        &runtime,
        "Use a temporary script to parse the input.",
        true,
        ReplyLanguage::En,
        move |args| {
            seen_actions_closure.borrow_mut().push(
                args.get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
            let next = responses_closure.borrow_mut().remove(0);
            std::future::ready(Ok(next))
        },
    ))
    .expect("temporary execution should succeed");

    assert_eq!(reply.text, "parsed successfully");
    assert_eq!(
        seen_actions.borrow().as_slice(),
        ["temporary_fix_plan", "temporary_fix_execute"]
    );
}

#[test]
fn permanent_extension_runtime_enable_runs_full_chain_and_reloads() {
    let runtime = SelfExtensionConfig {
        enabled: true,
        allow_permanent_extension: true,
        allow_runtime_enable: true,
        ..Default::default()
    };
    let seen_actions: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let responses: Rc<RefCell<Vec<serde_json::Value>>> = Rc::new(RefCell::new(vec![
        json!({
            "status":"ok",
            "text":"plan ready",
            "extra":{"plan":{
                "skill_name":"demo_ext",
                "capability_summary":"Reply to ping with a short success message.",
                "actions":["ping"],
                "rationale":"Reusable ping demo."
            }}
        }),
        json!({"status":"ok","text":"scaffolded","extra":{"skill_name":"demo_ext"}}),
        json!({"status":"ok","text":"implemented","extra":{"skill_name":"demo_ext"}}),
        json!({"status":"ok","text":"validated","extra":{"skill_name":"demo_ext"}}),
        json!({"status":"ok","text":"registered","extra":{"skill_name":"demo_ext"}}),
        json!({"status":"ok","text":"enabled","extra":{"skill_name":"demo_ext"}}),
    ]));
    let reload_count = Rc::new(Cell::new(0usize));
    let seen_actions_closure = seen_actions.clone();
    let responses_closure = responses.clone();
    let reload_count_closure = reload_count.clone();
    let reply = run_async(handle_permanent_extension_with(
        None,
        None,
        &runtime,
        "Do not use existing skills. Create and enable a reusable ping skill.",
        true,
        ReplyLanguage::En,
        move |args| {
            seen_actions_closure.borrow_mut().push(
                args.get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
            let next = responses_closure.borrow_mut().remove(0);
            std::future::ready(Ok(next))
        },
        move || {
            reload_count_closure.set(reload_count_closure.get() + 1);
            Ok(())
        },
    ))
    .expect("permanent extension should succeed");

    assert!(reply.text.contains("external_skills/demo_ext"));
    assert!(reply.text.contains("visible to the runtime"));
    assert_eq!(
        seen_actions.borrow().as_slice(),
        [
            "permanent_extension_plan",
            "scaffold_external_skill",
            "implement_external_skill",
            "validate_external_skill",
            "register_external_skill",
            "enable_external_skill",
        ]
    );
    assert_eq!(reload_count.get(), 1);
}

#[test]
fn effective_request_prefers_resolved_intent() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Use a temporary script instead of built-in skills.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let request = effective_request("resolved prompt", "请不要走现有技能", &route);
    assert_eq!(
        request,
        "Use a temporary script instead of built-in skills."
    );
}

fn run_async<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime should build")
        .block_on(future)
}
