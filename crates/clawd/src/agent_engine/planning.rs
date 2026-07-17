use super::planning_action_normalization::normalize_planned_actions;
use super::planning_actions::build_plan_result_with_notes;
use super::planning_parse::parse_single_plan_actions;
use super::planning_prompt::{
    build_incremental_plan_prompt, incremental_prompt_spec, round1_prompt_spec, runtime_os_label,
    runtime_shell_label,
};
use super::planning_repair::repair_plan_actions;
use tracing::{info, warn};

use super::{
    attempt_ledger::build_attempt_ledger_compact, build_loop_history_compact,
    build_planner_skill_context, build_single_plan_prompt, build_turn_analysis_prompt_block,
    AgentLoopGuardPolicy, LoopState, AGENT_TOOL_SPEC_PATH,
};
use crate::{llm_gateway, AgentAction, AppState, ClaimedTask, PlanKind, PlanResult};

/// Planner-visible tool and skill inventory for one loop round.
///
/// This helper only prepares prompt/tool-library material. It must not build a
/// `PlanResult`, choose a capability, or short-circuit the planner LLM.
struct PlannerToolLibrary<'a> {
    state: &'a AppState,
    task: &'a ClaimedTask,
}

impl<'a> PlannerToolLibrary<'a> {
    fn new(state: &'a AppState, task: &'a ClaimedTask) -> Self {
        Self { state, task }
    }

    fn skill_context(
        &self,
        loop_state: &LoopState,
    ) -> super::planner_skill_context::PlannerSkillContext {
        build_planner_skill_context(self.state, self.task, loop_state)
    }

    fn tool_spec(&self) -> Result<String, String> {
        crate::bootstrap::load_required_prompt_template_for_state(self.state, AGENT_TOOL_SPEC_PATH)
            .map(|resolved| {
                let capability_map =
                    crate::capability_map::build_capability_map_for_task(self.state, self.task);
                let mut spec = String::new();
                spec.push_str("runtime_capability_map_v1");
                spec.push('\n');
                spec.push_str(&capability_map);
                spec.push('\n');
                spec.push('\n');
                spec.push_str(&resolved.0);
                spec
            })
            .map_err(|err| err.to_string())
    }
}

#[path = "planner_abort_recovery.rs"]
mod planner_abort_recovery;
use planner_abort_recovery::*;

pub(super) async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    turn_analysis_for_prompt: Option<&crate::turn_context::TurnAnalysis>,
    boundary_envelope_for_prompt: Option<&crate::turn_boundary_envelope::TurnBoundaryEnvelope>,
    _auto_locator_path: Option<&str>,
) -> Result<PlanResult, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let agent_runtime_identity = state.agent_runtime_identity_label().to_string();
    let recent_assistant_replies = crate::memory::build_recent_assistant_replies_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        3,
        220,
    );
    let planner_tool_library = PlannerToolLibrary::new(state, task);
    let skill_context = planner_tool_library.skill_context(loop_state);
    let skill_playbooks = &skill_context.text;
    let tool_spec_template = planner_tool_library.tool_spec()?;
    let turn_analysis =
        build_turn_analysis_prompt_block(turn_analysis_for_prompt, boundary_envelope_for_prompt);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let attempt_ledger = build_attempt_ledger_compact(loop_state);
    let (prompt_name, prompt_source, prompt_version, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_name, prompt_logical_path) = round1_prompt_spec();
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        (
            prompt_name,
            resolved.source,
            resolved.version,
            build_single_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                &agent_runtime_identity,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            ),
        )
    } else {
        let history_compact = build_loop_history_compact(loop_state);
        // Phase 3.3 / observation history regression fix:
        // 之前这里只读 delivery_messages.last()。delivery_messages 仅承载最终 respond/交付
        // 文本，observation-only 步骤（fs_search/list_dir/read_file/run_cmd 等）的输出从不
        // 写入这里。结果是 round N+1 的 loop planner 看到 "Last round output: (none)"，
        // 完全看不到 round N 的工具输出，于是会重复同一观察步骤，最终触发 plan_unactionable
        // 兜底（i18n 模板被误用作 "provider unavailable" 文案）。
        // 真正记录每步输出的字段是 LoopState.last_output（agent_engine.rs 中
        // register_step_output / register_failed_step_output 都会维护）。优先使用它，
        // 仅在确无 step output 时回退到 delivery_messages，最后退化到占位符。
        let last_output = loop_state
            .last_output
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| crate::truncate_for_log(s))
            .or_else(|| loop_state.delivery_messages.last().cloned())
            .unwrap_or_else(|| "(none)".to_string());
        let (prompt_name, prompt_logical_path) = incremental_prompt_spec();
        let resolved = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            prompt_logical_path,
        )
        .map_err(|e| e.to_string())?;
        (
            prompt_name,
            resolved.source,
            resolved.version,
            build_incremental_plan_prompt(
                &resolved.template,
                &user_request_for_prompt,
                goal,
                &turn_analysis,
                &tool_spec_template,
                &skill_playbooks,
                &recent_assistant_replies,
                &request_language_hint,
                &state.policy.command_intent.default_locale,
                &agent_runtime_identity,
                loop_state.round_no,
                &history_compact,
                &attempt_ledger,
                &last_output,
                &runtime_os,
                &runtime_shell,
                &workspace_root,
            ),
        )
    };
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        prompt_name,
        &prompt_source,
        prompt_version.as_deref(),
        Some(loop_state.round_no),
    );
    info!(
        "{} loop_round_plan task_id={} round={} max_rounds={} max_steps={} multi_round_enabled={}",
        crate::highlight_tag("loop"),
        task.task_id,
        loop_state.round_no,
        policy.max_rounds,
        policy.max_steps,
        policy.multi_round_enabled
    );
    info!(
        "plan_llm_request task_id={} round={} planner_mode=agent_loop prompt_chars={} tool_spec_chars={} skill_context_chars={} skill_context_mode={} selected_skills={} quick_index_chars={} playbook_chars={} recent_replies_chars={} user_request={}",
        task.task_id,
        loop_state.round_no,
        prompt_text.chars().count(),
        tool_spec_template.chars().count(),
        skill_playbooks.chars().count(),
        skill_context.disclosure_mode,
        skill_context.selected_skills.join(","),
        skill_context.quick_index_chars,
        skill_context.playbook_chars,
        recent_assistant_replies.chars().count(),
        crate::truncate_for_log(user_text)
    );
    let plan_raw = llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt_text,
        &prompt_source,
    )
    .await?;
    info!(
        "plan_llm_response task_id={} round={} raw={}",
        task.task_id,
        loop_state.round_no,
        crate::truncate_for_log(&plan_raw)
    );
    let initial_actions = parse_single_plan_actions(&plan_raw, state, task)
        .await
        .map(|actions| normalize_planned_actions(state, actions));
    let (plan_actions, plan_kind, raw_plan_text, planner_notes) = if initial_actions.is_none() {
        let repair_reason = "plan_parse_failed";
        warn!(
            "plan_repair_required task_id={} round={} reason={}",
            task.task_id, loop_state.round_no, repair_reason
        );
        let repaired = repair_plan_actions(
            state,
            task,
            goal,
            &turn_analysis,
            user_text,
            repair_reason,
            &tool_spec_template,
            &skill_playbooks,
            &attempt_ledger,
            &plan_raw,
            loop_state.round_no,
        )
        .await?;
        let repaired_actions = parse_single_plan_actions(&repaired, state, task)
            .await
            .map(|actions| normalize_planned_actions(state, actions));
        if let Some(actions) = repaired_actions {
            (
                actions,
                PlanKind::Repair,
                repaired,
                planner_notes_for_repair_success(repair_reason, None),
            )
        } else if let Some((actions, raw)) = try_compact_abort_recovery_plan(
            state,
            task,
            goal,
            &turn_analysis,
            user_text,
            loop_state,
            &tool_spec_template,
            &skill_playbooks,
            &attempt_ledger,
            &plan_raw,
            Some(&repaired),
        )
        .await?
        {
            (
                actions,
                PlanKind::Repair,
                raw,
                planner_notes_for_repair_success(
                    repair_reason,
                    Some("planner_abort_compact_retry"),
                ),
            )
        } else {
            return Err("plan_parse_failed_no_executable_steps".to_string());
        }
    } else {
        (
            initial_actions.expect("checked Some above"),
            if loop_state.round_no <= 1 {
                PlanKind::Single
            } else {
                PlanKind::Incremental
            },
            plan_raw.clone(),
            String::new(),
        )
    };
    let plan_result = build_plan_result_with_notes(
        goal,
        &raw_plan_text,
        plan_kind,
        &plan_actions,
        &planner_notes,
    );
    let labels = plan_result.step_labels();
    info!(
        "act_split_trace task_id={} round={} split_steps={}",
        task.task_id,
        loop_state.round_no,
        serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(plan_result)
}

#[allow(clippy::too_many_arguments)]
async fn try_compact_abort_recovery_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    turn_analysis: &str,
    user_text: &str,
    loop_state: &LoopState,
    tool_spec_template: &str,
    skill_playbooks: &str,
    attempt_ledger: &str,
    first_raw_plan: &str,
    latest_raw_plan: Option<&str>,
) -> Result<Option<(Vec<AgentAction>, String)>, String> {
    let Some((actions, raw)) = compact_retry_plan_actions(
        state,
        task,
        PlannerAbortRecoveryInput {
            goal,
            turn_analysis,
            user_text,
            tool_spec: tool_spec_template,
            skill_playbooks,
            attempt_ledger,
            first_raw_plan,
            latest_raw_plan,
            round_no: loop_state.round_no,
            loop_state,
        },
    )
    .await?
    else {
        return Ok(None);
    };
    let actions = normalize_planned_actions(state, actions);
    Ok(Some((actions, raw)))
}

fn planner_notes_for_repair_success(first_reason: &str, second_reason: Option<&str>) -> String {
    let mut notes = vec![format!("repair_reason_code={first_reason}")];
    if let Some(second_reason) = second_reason {
        notes.push(format!("second_repair_reason_code={second_reason}"));
    }
    notes.join(" ")
}
