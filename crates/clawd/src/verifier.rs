use std::collections::{HashMap, HashSet};

use claw_core::skill_registry::PrimaryFallbackRole;

use crate::{AppState, ClaimedTask, PlanResult, PlanStep};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyMode {
    ObserveOnly,
    Enforce,
}

impl Default for VerifyMode {
    fn default() -> Self {
        Self::ObserveOnly
    }
}

impl VerifyMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "ObserveOnly",
            Self::Enforce => "Enforce",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyIssueKind {
    SkillNotVisible,
    MissingRequiredArg,
    InvalidDependsOn,
    ConfirmationRequired,
    PrimaryFallbackConflict,
    RouteClarifyRequired,
}

impl Default for VerifyIssueKind {
    fn default() -> Self {
        Self::SkillNotVisible
    }
}

impl VerifyIssueKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SkillNotVisible => "SkillNotVisible",
            Self::MissingRequiredArg => "MissingRequiredArg",
            Self::InvalidDependsOn => "InvalidDependsOn",
            Self::ConfirmationRequired => "ConfirmationRequired",
            Self::PrimaryFallbackConflict => "PrimaryFallbackConflict",
            Self::RouteClarifyRequired => "RouteClarifyRequired",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VerifyIssue {
    pub(crate) step_id: String,
    pub(crate) kind: VerifyIssueKind,
    pub(crate) detail: String,
}

pub(crate) struct VerifyInput<'a> {
    pub(crate) route_result: Option<&'a crate::RouteResult>,
    pub(crate) context_bundle_summary: Option<&'a str>,
    pub(crate) plan_result: &'a PlanResult,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct VerifyResult {
    pub(crate) mode: VerifyMode,
    pub(crate) approved: bool,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) shadow_blocked_reason: Option<String>,
    pub(crate) approved_steps: Vec<PlanStep>,
    pub(crate) needs_confirmation: bool,
    pub(crate) rewritten_steps: Vec<PlanStep>,
    pub(crate) issues: Vec<VerifyIssue>,
}

fn required_args_for_skill(skill: &str) -> &'static [&'static str] {
    match skill {
        "run_cmd" => &["command"],
        "read_file" => &["path"],
        "write_file" => &["path", "content"],
        "remove_file" => &["path"],
        "make_dir" => &["path"],
        _ => &[],
    }
}

fn is_confirmation_like_skill(skill: &str) -> bool {
    matches!(
        skill,
        "run_cmd" | "write_file" | "remove_file" | "make_dir" | "schedule"
    )
}

fn route_has_confirmation_resume(route_result: Option<&crate::RouteResult>) -> bool {
    route_result
        .map(|route| matches!(route.resume_behavior, crate::ResumeBehavior::ResumeExecute))
        .unwrap_or(false)
}

fn manifest_required_args(state: &AppState, normalized_skill: &str) -> Vec<String> {
    state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| manifest.input_schema)
        .and_then(|schema| schema.get("required").cloned())
        .and_then(|required| required.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn push_group_conflict_issues(
    issues: &mut Vec<VerifyIssue>,
    group: &str,
    entries: &[(String, String, PrimaryFallbackRole)],
    detail: String,
) {
    for (step_id, normalized_skill, _) in entries {
        issues.push(VerifyIssue {
            step_id: step_id.clone(),
            kind: VerifyIssueKind::PrimaryFallbackConflict,
            detail: format!("group `{group}` skill `{normalized_skill}` conflict: {detail}"),
        });
    }
}

fn verify_primary_fallback_conflicts(
    state: &AppState,
    plan_result: &PlanResult,
    issues: &mut Vec<VerifyIssue>,
) {
    let mut grouped: HashMap<String, Vec<(String, String, PrimaryFallbackRole)>> = HashMap::new();

    for step in &plan_result.steps {
        if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            continue;
        }
        let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
        let Some(manifest) = state.skill_manifest(&normalized_skill) else {
            continue;
        };
        let Some(group) = manifest
            .group
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let role = manifest
            .primary_fallback_role
            .unwrap_or(PrimaryFallbackRole::None);
        if matches!(role, PrimaryFallbackRole::None) {
            continue;
        }
        grouped.entry(group.to_string()).or_default().push((
            step.step_id.clone(),
            normalized_skill,
            role,
        ));
    }

    for (group, entries) in grouped {
        let primary_count = entries
            .iter()
            .filter(|(_, _, role)| matches!(role, PrimaryFallbackRole::Primary))
            .count();
        let fallback_count = entries
            .iter()
            .filter(|(_, _, role)| matches!(role, PrimaryFallbackRole::Fallback))
            .count();

        if primary_count > 0 && fallback_count > 0 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "both primary and fallback steps are present in the same plan".to_string(),
            );
            continue;
        }
        if primary_count > 1 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "multiple primary steps are present in the same group".to_string(),
            );
            continue;
        }
        if fallback_count > 1 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "multiple fallback steps are present in the same group".to_string(),
            );
        }
    }
}

fn verify_step_args(
    state: &AppState,
    step: &PlanStep,
    normalized_skill: &str,
    issues: &mut Vec<VerifyIssue>,
) {
    let manifest_required = manifest_required_args(state, normalized_skill);
    let fallback_required = required_args_for_skill(normalized_skill);
    let required: Vec<String> = if manifest_required.is_empty() {
        fallback_required
            .iter()
            .map(|key| (*key).to_string())
            .collect()
    } else {
        manifest_required
    };
    if required.is_empty() {
        return;
    }
    let Some(obj) = step.args.as_object() else {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::MissingRequiredArg,
            detail: format!("skill `{normalized_skill}` args must be an object"),
        });
        return;
    };
    for key in &required {
        let missing = obj
            .get(key)
            .map(|v| {
                (v.is_string() && v.as_str().map(str::trim).unwrap_or("").is_empty()) || v.is_null()
            })
            .unwrap_or(true);
        if missing {
            issues.push(VerifyIssue {
                step_id: step.step_id.clone(),
                kind: VerifyIssueKind::MissingRequiredArg,
                detail: format!("skill `{normalized_skill}` missing required arg `{key}`"),
            });
        }
    }
}

fn issue_blocks_in_enforce(kind: VerifyIssueKind) -> bool {
    matches!(
        kind,
        VerifyIssueKind::SkillNotVisible
            | VerifyIssueKind::MissingRequiredArg
            | VerifyIssueKind::InvalidDependsOn
            | VerifyIssueKind::PrimaryFallbackConflict
            | VerifyIssueKind::RouteClarifyRequired
    )
}

fn first_shadow_blocked_reason(issues: &[VerifyIssue]) -> Option<String> {
    issues
        .iter()
        .find(|issue| issue_blocks_in_enforce(issue.kind))
        .map(|issue| issue.detail.clone())
}

pub(crate) fn verify_plan(
    state: &AppState,
    task: &ClaimedTask,
    input: VerifyInput<'_>,
    mode: VerifyMode,
) -> VerifyResult {
    if input
        .route_result
        .map(|route| route.needs_clarify)
        .unwrap_or(false)
        && input
            .plan_result
            .steps
            .iter()
            .any(|step| matches!(step.action_type.as_str(), "call_skill" | "call_tool"))
    {
        let detail = format!(
            "route requires clarify before execution; context={}",
            input.context_bundle_summary.unwrap_or("<none>")
        );
        let shadow_blocked_reason = Some(detail.clone());
        let blocked_reason = matches!(mode, VerifyMode::Enforce).then_some(detail.clone());
        return VerifyResult {
            mode,
            approved: blocked_reason.is_none(),
            blocked_reason,
            shadow_blocked_reason,
            approved_steps: input.plan_result.steps.clone(),
            needs_confirmation: false,
            rewritten_steps: Vec::new(),
            issues: vec![VerifyIssue {
                step_id: "route".to_string(),
                kind: VerifyIssueKind::RouteClarifyRequired,
                detail,
            }],
        };
    }
    let visible_skills: HashSet<String> = state
        .planner_visible_skills_for_task(task)
        .into_iter()
        .collect();
    let all_step_ids: HashSet<String> = input
        .plan_result
        .steps
        .iter()
        .map(|step| step.step_id.clone())
        .collect();
    let confirmation_already_granted = route_has_confirmation_resume(input.route_result);
    let mut issues = Vec::new();
    let mut needs_confirmation = false;

    for step in &input.plan_result.steps {
        if matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
            if !visible_skills.contains(&normalized_skill) {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::SkillNotVisible,
                    detail: format!("skill `{normalized_skill}` is not in planner visible skills"),
                });
            }
            verify_step_args(state, step, &normalized_skill, &mut issues);
            if !confirmation_already_granted
                && (state.skill_requires_confirmation_policy(&normalized_skill)
                    || is_confirmation_like_skill(&normalized_skill))
            {
                needs_confirmation = true;
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::ConfirmationRequired,
                    detail: format!("skill `{normalized_skill}` may require explicit confirmation"),
                });
            }
        }

        for dep in &step.depends_on {
            if !all_step_ids.contains(dep) {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::InvalidDependsOn,
                    detail: format!("depends_on references missing step `{dep}`"),
                });
            }
        }
    }

    verify_primary_fallback_conflicts(state, input.plan_result, &mut issues);

    let shadow_blocked_reason = first_shadow_blocked_reason(&issues);
    let blocked_reason = if matches!(mode, VerifyMode::Enforce) {
        shadow_blocked_reason.clone()
    } else {
        None
    };

    let approved = blocked_reason.is_none();
    let approved_steps = input.plan_result.steps.clone();

    VerifyResult {
        mode,
        approved,
        blocked_reason,
        shadow_blocked_reason,
        approved_steps,
        needs_confirmation,
        rewritten_steps: Vec::new(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::Instant;

    use claw_core::config::{
        AgentConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, ToolsConfig,
    };
    use claw_core::skill_registry::SkillsRegistry;
    use reqwest::Client;
    use rusqlite::Connection;
    use serde_json::json;
    use tokio::sync::Semaphore;

    use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, CommandIntentRuntime, PlanKind, PlanResult,
        PlanStep, RateLimiter, RouteResult, RoutedMode, ScheduleKind, ScheduleRuntime,
        SkillViewsSnapshot, ToolsPolicy,
    };

    fn test_registry() -> SkillsRegistry {
        let toml = r#"
[[skills]]
name = "read_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["command"], properties = { command = { type = "string" } } }

[[skills]]
name = "primary_reader"
enabled = true
kind = "runner"
output_kind = "text"
group = "reader"
primary_fallback_role = "primary"

[[skills]]
name = "fallback_reader"
enabled = true
kind = "runner"
output_kind = "text"
group = "reader"
primary_fallback_role = "fallback"
"#;
        let path = std::env::temp_dir().join(format!(
            "verifier_registry_{}_{}_{}.toml",
            std::process::id(),
            crate::now_ts_u64(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, toml).expect("write registry");
        let registry = SkillsRegistry::load_from_path(&path).expect("load registry");
        let _ = std::fs::remove_file(path);
        registry
    }

    fn test_state() -> AppState {
        let registry = Arc::new(test_registry());
        let skills_list = Arc::new(
            ["read_file", "run_cmd", "primary_reader", "fallback_reader"]
                .into_iter()
                .map(str::to_string)
                .collect::<HashSet<_>>(),
        );
        let agents_by_id = HashMap::from([(
            crate::DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            started_at: Instant::now(),
            queue_limit: 1,
            db: Arc::new(Mutex::new(Connection::open_in_memory().expect("open db"))),
            llm_providers: Vec::new(),
            agents_by_id: Arc::new(agents_by_id),
            skill_timeout_seconds: 30,
            skill_runner_path: std::path::PathBuf::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: Some(registry),
                skills_list,
            }))),
            skill_semaphore: Arc::new(Semaphore::new(1)),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(60, 30))),
            llm_calls_per_task: Arc::new(Mutex::new(HashMap::new())),
            maintenance: MaintenanceConfig::default(),
            memory: MemoryConfig::default(),
            workspace_root: std::env::temp_dir(),
            default_locator_search_dir: std::env::temp_dir(),
            locator_scan_max_depth: 3,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            active_provider_type: None,
            cmd_timeout_seconds: 30,
            max_cmd_length: 4096,
            allow_path_outside_workspace: false,
            allow_sudo: false,
            worker_task_timeout_seconds: 300,
            worker_task_heartbeat_seconds: 10,
            worker_running_no_progress_timeout_seconds: 300,
            worker_running_recovery_check_interval_seconds: 30,
            last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
            routing: RoutingConfig::default(),
            persona_prompt: String::new(),
            command_intent: CommandIntentRuntime {
                all_result_suffixes: Vec::new(),
                default_locale: "zh-CN".to_string(),
                verify_enforce_enabled: false,
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: String::new(),
                intent_prompt_source: String::new(),
                intent_rules_template: String::new(),
                locale: "zh-CN".to_string(),
                i18n_dict: HashMap::new(),
            },
            telegram_bot_token: String::new(),
            telegram_configured_bot_names: Arc::new(Vec::new()),
            whatsapp_cloud_enabled: false,
            whatsapp_api_base: String::new(),
            whatsapp_access_token: String::new(),
            whatsapp_phone_number_id: String::new(),
            whatsapp_web_enabled: false,
            whatsapp_web_bridge_base_url: String::new(),
            future_adapters_enabled: Arc::new(Vec::new()),
            wechat_send_config: None,
            feishu_send_config: None,
            lark_send_config: None,
            http_client: Client::new(),
            database_sqlite_path: std::path::PathBuf::new(),
            database_busy_timeout_ms: 5_000,
            config_path_for_reload: String::new(),
            registry_path_for_reload: None,
            skill_switches_for_reload: Arc::new(HashMap::new()),
            initial_skills_list_for_reload: Vec::new(),
        }
    }

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "task-verify".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn route_result(needs_clarify: bool) -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::ChatAct,
            resolved_intent: "test".to_string(),
            needs_clarify,
            route_reason: "test".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: vec!["read_file".to_string()],
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    fn plan_result(steps: Vec<PlanStep>) -> PlanResult {
        PlanResult {
            goal: "test".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps,
            planner_notes: String::new(),
            plan_kind: PlanKind::Single,
            raw_plan_text: String::new(),
        }
    }

    #[test]
    fn observe_mode_keeps_route_clarify_as_shadow_only() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(true)),
                context_bundle_summary: Some("need more info"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "README.md" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.approved);
        assert!(result.blocked_reason.is_none());
        assert!(matches!(
            result.issues.first().map(|issue| issue.kind),
            Some(VerifyIssueKind::RouteClarifyRequired)
        ));
        assert!(result.shadow_blocked_reason.is_some());
    }

    #[test]
    fn enforce_mode_blocks_missing_required_arg() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(matches!(
            result.issues.first().map(|issue| issue.kind),
            Some(VerifyIssueKind::MissingRequiredArg)
        ));
        assert!(result
            .blocked_reason
            .as_deref()
            .unwrap_or_default()
            .contains("missing required arg"));
    }

    #[test]
    fn enforce_mode_blocks_skill_not_visible() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "totally_fake_skill".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result
            .issues
            .iter()
            .any(|issue| { matches!(issue.kind, VerifyIssueKind::SkillNotVisible) }));
    }

    #[test]
    fn enforce_mode_blocks_primary_fallback_conflict() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "primary_reader".to_string(),
                        args: json!({}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "fallback_reader".to_string(),
                        args: json!({}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ]),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result
            .issues
            .iter()
            .any(|issue| { matches!(issue.kind, VerifyIssueKind::PrimaryFallbackConflict) }));
    }

    #[test]
    fn resume_execute_route_skips_confirmation_requirement() {
        let state = test_state();
        let task = test_task();
        let mut resumed_route = route_result(false);
        resumed_route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&resumed_route),
                context_bundle_summary: Some("resume"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "pwd" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved);
        assert!(!result.needs_confirmation);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    }
}
