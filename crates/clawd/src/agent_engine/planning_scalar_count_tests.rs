use super::*;
use crate::{
    AgentAction, AskMode, IntentOutputContract, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_planning_scalar_count_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn scalar_count_route(root_path: &str) -> RouteResult {
    RouteResult {
        ask_mode: AskMode::planner_execute_plain(),
        resolved_intent: "count directories".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: Default::default(),
            semantic_kind: OutputSemanticKind::ScalarCount,
            locator_hint: root_path.to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn expect_planned_call<'a>(action: &'a AgentAction, tool: &str, action_name: &str) -> &'a Value {
    let (actual_tool, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => panic!("expected {tool}.{action_name} call, got {action:?}"),
    };
    assert_eq!(actual_tool, tool);
    assert_eq!(
        args.get("action").and_then(Value::as_str),
        Some(action_name)
    );
    args
}

#[test]
fn scalar_count_listing_plan_normalizes_filter_kind_directories_alias() {
    let root = TempDirGuard::new("filter_kind_directories_alias");
    fs::write(root.path.join("a.txt"), "a").expect("write a");
    fs::create_dir_all(root.path.join("child")).expect("create child");
    let root_path = root.path.display().to_string();
    let route = scalar_count_route(&root_path);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
            "filter_kind": "directories",
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count directories",
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "count_entries");
    assert_eq!(args.get("kind_filter").and_then(Value::as_str), Some("dir"));
    assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(true));
    assert_eq!(
        args.get("count_files").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(args.get("dirs_only").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("files_only").and_then(Value::as_bool), Some(false));
}

#[test]
fn scalar_count_listing_plan_applies_contract_extension_filter() {
    let root = TempDirGuard::new("contract_extension_filter");
    fs::write(root.path.join("a.md"), "a").expect("write md");
    fs::write(root.path.join("b.txt"), "b").expect("write txt");
    let root_path = root.path.display().to_string();
    let mut route = scalar_count_route(&root_path);
    route.output_contract.self_extension.scalar_count_filter = crate::OutputScalarCountFilter {
        target_kind: crate::OutputScalarCountTargetKind::File,
        include_hidden: Some(false),
        recursive: Some(true),
        extensions: vec!["md".to_string()],
    };
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": root_path.clone(),
        }),
    }];

    let normalized = replace_scalar_count_plan_with_count_inventory(
        Some(&route),
        &LoopState::new(1),
        Some(&root_path),
        "count md files",
        actions,
    );

    let args = expect_planned_call(&normalized[0], "fs_basic", "count_entries");
    assert_eq!(
        args.get("kind_filter").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(args.get("count_files").and_then(Value::as_bool), Some(true));
    assert_eq!(args.get("count_dirs").and_then(Value::as_bool), Some(false));
    assert_eq!(args.get("recursive").and_then(Value::as_bool), Some(true));
    let ext_filter = args
        .get("ext_filter")
        .and_then(Value::as_array)
        .expect("ext_filter array");
    assert_eq!(ext_filter, &vec![Value::String("md".to_string())]);
}
