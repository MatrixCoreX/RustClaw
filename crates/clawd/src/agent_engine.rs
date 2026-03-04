use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};
use toml::Value as TomlValue;
use tracing::{info, warn};

use crate::{execution_adapters, intent_router, llm_gateway, repo, AgentAction, AppState, AskReply, ClaimedTask};

const SKILL_PROMPT_ARCHIVE_BASIC: &str = include_str!("../../../prompts/skills/archive_basic.md");
const SKILL_PROMPT_AUDIO_SYNTHESIZE: &str = include_str!("../../../prompts/skills/audio_synthesize.md");
const SKILL_PROMPT_AUDIO_TRANSCRIBE: &str = include_str!("../../../prompts/skills/audio_transcribe.md");
const SKILL_PROMPT_CONFIG_GUARD: &str = include_str!("../../../prompts/skills/config_guard.md");
const SKILL_PROMPT_CRYPTO: &str = include_str!("../../../prompts/skills/crypto.md");
const SKILL_PROMPT_DB_BASIC: &str = include_str!("../../../prompts/skills/db_basic.md");
const SKILL_PROMPT_DOCKER_BASIC: &str = include_str!("../../../prompts/skills/docker_basic.md");
const SKILL_PROMPT_FS_SEARCH: &str = include_str!("../../../prompts/skills/fs_search.md");
const SKILL_PROMPT_GIT_BASIC: &str = include_str!("../../../prompts/skills/git_basic.md");
const SKILL_PROMPT_HEALTH_CHECK: &str = include_str!("../../../prompts/skills/health_check.md");
const SKILL_PROMPT_HTTP_BASIC: &str = include_str!("../../../prompts/skills/http_basic.md");
const SKILL_PROMPT_IMAGE_EDIT: &str = include_str!("../../../prompts/skills/image_edit.md");
const SKILL_PROMPT_IMAGE_GENERATE: &str = include_str!("../../../prompts/skills/image_generate.md");
const SKILL_PROMPT_IMAGE_VISION: &str = include_str!("../../../prompts/skills/image_vision.md");
const SKILL_PROMPT_INSTALL_MODULE: &str = include_str!("../../../prompts/skills/install_module.md");
const SKILL_PROMPT_LOG_ANALYZE: &str = include_str!("../../../prompts/skills/log_analyze.md");
const SKILL_PROMPT_PACKAGE_MANAGER: &str = include_str!("../../../prompts/skills/package_manager.md");
const SKILL_PROMPT_PROCESS_BASIC: &str = include_str!("../../../prompts/skills/process_basic.md");
const SKILL_PROMPT_RSS_FETCH: &str = include_str!("../../../prompts/skills/rss_fetch.md");
const SKILL_PROMPT_SERVICE_CONTROL: &str = include_str!("../../../prompts/skills/service_control.md");
const SKILL_PROMPT_SYSTEM_BASIC: &str = include_str!("../../../prompts/skills/system_basic.md");
const SKILL_PROMPT_X: &str = include_str!("../../../prompts/skills/x.md");
const STEP_SPLIT_PROMPT_TEMPLATE: &str = include_str!("../../../prompts/step_split_strict.md");
const AGENT_TOOL_SPEC_TEMPLATE: &str = include_str!("../../../prompts/agent_tool_spec.md");
const RESPOND_DELIVERY_INTENT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/respond_delivery_intent_prompt.md");
const DEFAULT_CRYPTO_NEWS_ACTIONS: &[&str] = &["news"];
const DEFAULT_CRYPTO_MARKET_QUERY_ACTIONS: &[&str] = &[
    "quote",
    "get_price",
    "multi_quote",
    "get_multi_price",
    "book_ticker",
    "get_book_ticker",
    "positions",
];
const DEFAULT_CRYPTO_TRADE_PREVIEW_ACTIONS: &[&str] = &["trade_preview"];

const SKILL_PLAYBOOKS: &[(&str, &str)] = &[
    ("archive_basic", SKILL_PROMPT_ARCHIVE_BASIC),
    ("audio_synthesize", SKILL_PROMPT_AUDIO_SYNTHESIZE),
    ("audio_transcribe", SKILL_PROMPT_AUDIO_TRANSCRIBE),
    ("config_guard", SKILL_PROMPT_CONFIG_GUARD),
    ("crypto", SKILL_PROMPT_CRYPTO),
    ("db_basic", SKILL_PROMPT_DB_BASIC),
    ("docker_basic", SKILL_PROMPT_DOCKER_BASIC),
    ("fs_search", SKILL_PROMPT_FS_SEARCH),
    ("git_basic", SKILL_PROMPT_GIT_BASIC),
    ("health_check", SKILL_PROMPT_HEALTH_CHECK),
    ("http_basic", SKILL_PROMPT_HTTP_BASIC),
    ("image_edit", SKILL_PROMPT_IMAGE_EDIT),
    ("image_generate", SKILL_PROMPT_IMAGE_GENERATE),
    ("image_vision", SKILL_PROMPT_IMAGE_VISION),
    ("install_module", SKILL_PROMPT_INSTALL_MODULE),
    ("log_analyze", SKILL_PROMPT_LOG_ANALYZE),
    ("package_manager", SKILL_PROMPT_PACKAGE_MANAGER),
    ("process_basic", SKILL_PROMPT_PROCESS_BASIC),
    ("rss_fetch", SKILL_PROMPT_RSS_FETCH),
    ("service_control", SKILL_PROMPT_SERVICE_CONTROL),
    ("system_basic", SKILL_PROMPT_SYSTEM_BASIC),
    ("x", SKILL_PROMPT_X),
];

fn load_rss_categories_for_prompt(state: &AppState) -> Vec<String> {
    let path = state.workspace_root.join("configs/rss.toml");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<TomlValue>(&raw) else {
        return Vec::new();
    };
    let mut out = value
        .get("rss")
        .and_then(|v| v.get("categories"))
        .and_then(|v| v.as_table())
        .map(|tbl| tbl.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    out.sort_unstable();
    out
}

fn build_rss_skill_prompt_with_categories(state: &AppState) -> String {
    let base = SKILL_PROMPT_RSS_FETCH.trim();
    let categories = load_rss_categories_for_prompt(state);
    if categories.is_empty() {
        return base.to_string();
    }
    format!(
        "{base}\n\n## Category Guardrails\n- Allowed `category` values come from `configs/rss.toml` `[rss.categories]`: {categories}\n- When calling `rss_fetch`, `category` must be one of the allowed values.\n- If user intent cannot be mapped confidently, use `general`.\n- Do not invent unseen category strings.",
        categories = categories.join(", ")
    )
}

fn build_skill_playbooks_text(state: &AppState) -> String {
    let mut sections = Vec::new();
    for (skill, body) in SKILL_PLAYBOOKS {
        let is_enabled = state.skills_list.contains(crate::canonical_skill_name(skill));
        let content = if !is_enabled {
            let disabled_text = crate::i18n_t_with_default(
                state,
                "clawd.msg.skill_disabled",
                "Skill is not enabled: {skill}. Please enable it in config and try again.",
            )
            .replace("{skill}", skill);
            format!(
                "Status: disabled.\n\nIf user intent requires this skill, do NOT call this skill.\nReturn `{{\"type\":\"respond\",\"content\":\"{disabled_text}\"}}`."
            )
        } else if *skill == "rss_fetch" {
            build_rss_skill_prompt_with_categories(state)
        } else {
            body.to_string()
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        sections.push(format!("### {skill}\n{trimmed}"));
    }
    if sections.is_empty() {
        "No skill playbooks configured.".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn build_numbered_subtask_summary(subtask_results: &[String]) -> String {
    subtask_results
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            let cleaned = line
                .trim()
                .strip_prefix(&format!("subtask#{} ", idx + 1))
                .unwrap_or(line.trim());
            format!("{}. {}", idx + 1, cleaned)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone)]
struct AgentLoopGuardPolicy {
    crypto_news_actions: HashSet<String>,
    crypto_market_query_actions: HashSet<String>,
    crypto_trade_preview_actions: HashSet<String>,
}

fn default_action_set(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| v.to_ascii_lowercase()).collect()
}

fn parse_action_set_from_toml(root: &TomlValue, path: &[&str], fallback: &[&str]) -> HashSet<String> {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return default_action_set(fallback);
        };
        cursor = next;
    }
    let Some(arr) = cursor.as_array() else {
        return default_action_set(fallback);
    };
    let parsed = arr
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect::<HashSet<_>>();
    if parsed.is_empty() {
        default_action_set(fallback)
    } else {
        parsed
    }
}

fn load_agent_loop_guard_policy(state: &AppState) -> AgentLoopGuardPolicy {
    let path = state.workspace_root.join("configs/agent_guard.toml");
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<TomlValue>(&raw).ok())
        .unwrap_or(TomlValue::Table(Default::default()));
    AgentLoopGuardPolicy {
        crypto_news_actions: parse_action_set_from_toml(
            &parsed,
            &["agent", "loop_guard", "crypto", "news_actions"],
            DEFAULT_CRYPTO_NEWS_ACTIONS,
        ),
        crypto_market_query_actions: parse_action_set_from_toml(
            &parsed,
            &["agent", "loop_guard", "crypto", "market_query_actions"],
            DEFAULT_CRYPTO_MARKET_QUERY_ACTIONS,
        ),
        crypto_trade_preview_actions: parse_action_set_from_toml(
            &parsed,
            &["agent", "loop_guard", "crypto", "trade_preview_actions"],
            DEFAULT_CRYPTO_TRADE_PREVIEW_ACTIONS,
        ),
    }
}

fn crypto_action_lower(args: &Value) -> Option<String> {
    args.as_object()
        .and_then(|m| m.get("action"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
}

fn is_crypto_news_call(canonical_skill: &str, args: &Value, policy: &AgentLoopGuardPolicy) -> bool {
    canonical_skill == "crypto"
        && crypto_action_lower(args)
            .as_deref()
            .map(|s| policy.crypto_news_actions.contains(s))
            .unwrap_or(false)
}

fn is_crypto_market_query_call(canonical_skill: &str, args: &Value, policy: &AgentLoopGuardPolicy) -> bool {
    if canonical_skill != "crypto" {
        return false;
    }
    crypto_action_lower(args)
        .as_deref()
        .map(|s| policy.crypto_market_query_actions.contains(s))
        .unwrap_or(false)
}

fn is_crypto_trade_preview_call(
    canonical_skill: &str,
    args: &Value,
    policy: &AgentLoopGuardPolicy,
) -> bool {
    canonical_skill == "crypto"
        && crypto_action_lower(args)
            .as_deref()
            .map(|s| policy.crypto_trade_preview_actions.contains(s))
            .unwrap_or(false)
}

fn is_crypto_trade_submit_call(canonical_skill: &str, args: &Value) -> bool {
    canonical_skill == "crypto"
        && crypto_action_lower(args)
            .as_deref()
            .map(|s| s == "trade_submit")
            .unwrap_or(false)
}

fn publish_progress_messages(state: &AppState, task: &ClaimedTask, delivery_messages: &[String]) {
    if delivery_messages.is_empty() {
        return;
    }
    let payload = json!({
        "progress_messages": delivery_messages,
    });
    if let Err(err) = repo::update_task_progress_result(state, &task.task_id, &payload.to_string()) {
        warn!(
            "run_agent_with_tools: task_id={} publish progress failed: {}",
            task.task_id, err
        );
    }
}

fn append_and_publish_progress_message(
    state: &AppState,
    task: &ClaimedTask,
    delivery_messages: &mut Vec<String>,
    message: String,
) {
    delivery_messages.push(message);
    publish_progress_messages(state, task, delivery_messages);
}

fn consume_tool_call_budget(tool_calls: &mut usize) -> Result<(), String> {
    if *tool_calls >= crate::AGENT_MAX_TOOL_CALLS {
        return Err("agent tool call limit exceeded".to_string());
    }
    *tool_calls += 1;
    Ok(())
}

fn extract_first_json_object(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            return serde_json::from_str::<Value>(&trimmed[start..=end]).ok();
        }
    }
    None
}

fn parse_steps_from_split_output(raw: &str) -> Option<Vec<String>> {
    let v = extract_first_json_object(raw)?;
    let steps = v
        .get("steps")
        .and_then(|x| x.as_array())?
        .iter()
        .filter_map(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if steps.is_empty() {
        None
    } else {
        Some(steps)
    }
}

fn build_step_split_prompt(text: &str) -> String {
    STEP_SPLIT_PROMPT_TEMPLATE.replace("__USER_REQUEST__", text)
}

fn sanitize_split_steps(user_request: &str, mut steps: Vec<String>) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();
    for step in steps.drain(..) {
        let s = step.trim();
        if s.is_empty() {
            continue;
        }
        if cleaned.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            continue;
        }
        cleaned.push(s.to_string());
    }
    if cleaned.is_empty() {
        return vec![user_request.trim().to_string()];
    }
    if cleaned.len() == 1 {
        return cleaned;
    }

    // Guardrail: if split text is overly expanded, likely hallucinated decomposition.
    let src_len = user_request.trim().chars().count().max(1);
    let total_steps_len = cleaned.iter().map(|s| s.chars().count()).sum::<usize>();
    if total_steps_len > src_len * 3 {
        return vec![user_request.trim().to_string()];
    }
    if cleaned.len() > 8 {
        return vec![user_request.trim().to_string()];
    }
    cleaned
}

async fn split_user_request_steps_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> Vec<String> {
    let text = user_request.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let split_prompt = build_step_split_prompt(text);
    match llm_gateway::run_with_fallback(state, task, &split_prompt).await {
        Ok(out) => {
            let parsed = parse_steps_from_split_output(&out).unwrap_or_else(|| vec![text.to_string()]);
            sanitize_split_steps(text, parsed)
        }
        Err(_) => vec![text.to_string()],
    }
}

async fn should_send_respond_to_user_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
) -> bool {
    let req = user_request.trim();
    if req.is_empty() {
        return false;
    }
    let prompt = RESPOND_DELIVERY_INTENT_PROMPT_TEMPLATE.replace("__USER_REQUEST__", req);
    info!(
        "prompt_invocation task_id={} prompt_name=respond_delivery_intent_prompt memory.long_term_summary=<none> memory.preferences=<none> memory.recalled_recent=<none>",
        task.task_id
    );
    match llm_gateway::run_with_fallback(state, task, &prompt).await {
        Ok(out) => {
            let parsed = crate::extract_json_object(&out)
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .or_else(|| serde_json::from_str::<Value>(&out).ok());
            parsed
                .as_ref()
                .and_then(|v| v.get("send_respond"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        }
        Err(_) => false,
    }
}

fn build_resume_context_error(
    plan_steps: &[String],
    user_request: &str,
    goal: &str,
    subtask_results: &[String],
    delivery_messages: &[String],
    failed_index: usize,
    failed_action: &str,
    err: &str,
) -> String {
    let completed_steps = if failed_index <= 1 {
        Vec::new()
    } else {
        subtask_results
            .iter()
            .take(failed_index.saturating_sub(1))
            .cloned()
            .collect::<Vec<_>>()
    };
    let remaining_steps = if plan_steps.len() > failed_index {
        plan_steps
            .iter()
            .skip(failed_index)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let resume_context = json!({
        "resume_context_id": format!("ctx-{}", uuid::Uuid::new_v4()),
        "user_request": user_request,
        "goal": goal,
        "plan_steps": plan_steps,
        "completed_steps": completed_steps,
        "completed_messages": delivery_messages,
        "failed_step": {
            "index": failed_index,
            "action": failed_action,
            "error": crate::truncate_for_agent_trace(err),
        },
        "remaining_steps": remaining_steps,
        "hint": "LLM should infer continuation from resume context and user follow-up."
    });
    let user_error = if resume_context
        .get("remaining_steps")
        .and_then(|v| v.as_array())
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        format!(
            "step {failed_index} failed ({failed_action}): {err}. Remaining steps are interrupted."
        )
    } else {
        format!("step {failed_index} failed ({failed_action}): {err}")
    };
    let payload = json!({
        "user_error": user_error,
        "resume_context": resume_context
    });
    format!("{}{}", crate::RESUME_CONTEXT_ERROR_PREFIX, payload)
}

pub(crate) async fn run_agent_with_tools(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_request: &str,
) -> Result<AskReply, String> {
    info!(
        "run_agent_with_tools: task_id={} user_id={} chat_id={} goal={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(goal)
    );
    let mut history: Vec<String> = Vec::new();
    let loop_guard_policy = load_agent_loop_guard_policy(state);
    let mut tool_calls = 0usize;
    let mut repeat_actions: HashMap<String, usize> = HashMap::new();
    let mut last_tool_or_skill_output: Option<String> = None;
    let mut last_image_file_tokens: Vec<String> = Vec::new();
    let routing_goal_seed = user_request.trim().to_string();
    let mut action_steps_executed = 0usize;
    let mut subtask_index = 0usize;
    let mut subtask_results: Vec<String> = Vec::new();
    let mut delivery_messages: Vec<String> = Vec::new();
    let mut last_success_run_cmd: Option<String> = None;
    let mut successful_crypto_market_query_signatures: HashSet<String> = HashSet::new();
    let mut crypto_market_query_param_retry_used = false;
    let mut awaiting_trade_confirmation = false;
    let mut has_successful_trade_submit = false;
    let mut image_generate_success_count = 0usize;
    let mut last_action_signature: Option<String> = None;
    let should_send_respond_to_user =
        should_send_respond_to_user_with_llm(state, task, user_request).await;
    let plan_steps = split_user_request_steps_with_llm(state, task, user_request).await;
    let estimated_plan_steps = plan_steps.len().max(1);
    if plan_steps.len() > 1 {
        let numbered_steps = plan_steps
            .iter()
            .enumerate()
            .map(|(idx, step)| format!("{}. {}", idx + 1, step))
            .collect::<Vec<_>>()
            .join("\n");
        let prefix = crate::i18n_t_with_default(
            state,
            "clawd.msg.multi_subtask_prefix",
            "Executed multiple instructions in order. Itemized results:\n",
        );
        let planning_message = format!("{prefix}\n{numbered_steps}");
        append_and_publish_progress_message(
            state,
            task,
            &mut delivery_messages,
            planning_message.clone(),
        );
        history.push(format!(
            "planner_steps: {}",
            crate::truncate_for_agent_trace(&planning_message)
        ));
    }
    info!(
        "run_agent_with_tools: task_id={} planned_steps={} plan={}",
        task.task_id,
        estimated_plan_steps,
        "llm-driven dynamic action planning"
    );
    crate::append_act_plan_log(
        state,
        task,
        "planned",
        estimated_plan_steps,
        action_steps_executed,
        tool_calls,
        "llm-driven dynamic action planning",
    );
    history.push("planner: llm-driven dynamic action planning".to_string());
    let skill_playbooks = build_skill_playbooks_text(state);

    for step in 1..=crate::AGENT_MAX_STEPS {
        let tool_spec = AGENT_TOOL_SPEC_TEMPLATE.trim();
        let hist_text = if history.is_empty() {
            "(empty)".to_string()
        } else {
            history.join("\n")
        };
        let mut dynamic_rules: Vec<&str> = Vec::new();
        dynamic_rules.push(
            "Never repeat an identical call_skill/call_tool with the same args after it already succeeded in this task. If repeated, output type=respond to finish.",
        );
        if !successful_crypto_market_query_signatures.is_empty() {
            dynamic_rules.push(
                "A crypto market/positions query already succeeded in this task. You may do at most ONE additional crypto query only when parameters are different (e.g. symbol/exchange changed). After that, do not call crypto query actions again; output type=respond.",
            );
        }
        if !delivery_messages.is_empty() {
            dynamic_rules.push(
                "Some tool/skill outputs were already delivered to user as progress messages. In final respond, return only NEW conversational content (e.g. joke/brief conclusion). Do not restate raw quote/positions/trade lines that were already delivered.",
            );
        }
        if has_successful_trade_submit {
            dynamic_rules.push(
                "A trade_submit already succeeded in this task. Do NOT call trade_submit again in this task. Output type=respond to end.",
            );
        }
        if awaiting_trade_confirmation {
            dynamic_rules.push(
                "A trade preview is already delivered and now waiting for user confirmation. Do not call more skills/tools. Output type=respond with empty content.",
            );
        }
        let dynamic_rule_block = format!(
            "\n\n## DYNAMIC_CONVERGENCE_RULES (RUNTIME)\n{}\n",
            dynamic_rules
                .into_iter()
                .enumerate()
                .map(|(idx, line)| format!("{}. {}", idx + 1, line))
                .collect::<Vec<_>>()
                .join("\n")
        );

        let prompt = crate::AGENT_RUNTIME_PROMPT_TEMPLATE
            .replace("__PERSONA_PROMPT__", &state.persona_prompt)
            .replace("__TOOL_SPEC__", tool_spec)
            .replace("__SKILL_PROMPTS__", &skill_playbooks)
            .replace("__GOAL__", goal)
            .replace("__STEP__", &step.to_string())
            .replace("__HISTORY__", &hist_text)
            + &dynamic_rule_block;
        info!(
            "prompt_invocation task_id={} prompt_name=agent_runtime_prompt memory.long_term_summary=<see worker_once ask memory log> memory.preferences=<see worker_once ask memory log> memory.recalled_recent=<see worker_once ask memory log> step={}",
            task.task_id,
            step
        );

        let llm_out = llm_gateway::run_with_fallback(state, task, &prompt).await?;
        let action_objects = crate::extract_agent_action_objects(&llm_out);
        let mut parsed_candidates: Vec<(String, AgentAction)> = Vec::new();
        for candidate in &action_objects {
            let raw_value: Value = match crate::parse_agent_action_json_with_repair(candidate) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let normalized_value = match crate::normalize_agent_action_value(raw_value) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let action: AgentAction = match serde_json::from_value(normalized_value) {
                Ok(v) => v,
                Err(_) => continue,
            };
            parsed_candidates.push((candidate.clone(), action));
        }

        if parsed_candidates.is_empty() {
            let json_str = action_objects
                .into_iter()
                .next()
                .or_else(|| crate::extract_json_object(&llm_out))
                .ok_or_else(|| format!("agent output is not valid json object: {llm_out}"))?;
            let raw_value: Value = crate::parse_agent_action_json_with_repair(&json_str)
                .map_err(|err| format!("parse agent action json failed: {err}; raw={json_str}"))?;
            let normalized_value = crate::normalize_agent_action_value(raw_value)
                .map_err(|err| format!("normalize agent action failed: {err}; raw={json_str}"))?;
            let action: AgentAction = serde_json::from_value(normalized_value)
                .map_err(|err| format!("parse agent action failed: {err}; raw={json_str}"))?;
            parsed_candidates.push((json_str, action));
        }

        let mut multi_action_note: Option<String> = None;
        let (selected_json, selected_action) = if parsed_candidates.len() > 1 {
            let selected_index = parsed_candidates
                .iter()
                .position(|(_, action)| {
                    matches!(
                        action,
                        AgentAction::CallTool { tool, .. } if tool == "write_file"
                    )
                })
                .or_else(|| {
                    parsed_candidates.iter().position(|(_, action)| {
                        matches!(
                            action,
                            AgentAction::CallTool { tool, .. } if tool != "run_cmd"
                        )
                    })
                })
                .or_else(|| {
                    parsed_candidates
                        .iter()
                        .position(|(_, action)| !matches!(action, AgentAction::Think { .. }))
                })
                .unwrap_or(0);
            let action_kinds = parsed_candidates
                .iter()
                .map(|(_, action)| crate::agent_action_signature(action))
                .collect::<Vec<_>>();
            let selected = parsed_candidates.swap_remove(selected_index);
            multi_action_note = Some(format!(
                "multi-action output detected (count={}); selected_one={}",
                action_kinds.len(),
                crate::agent_action_signature(&selected.1)
            ));
            warn!(
                "run_agent_with_tools: task_id={} step={} invalid multi-action output count={} selected={}",
                task.task_id,
                step,
                action_kinds.len(),
                crate::agent_action_signature(&selected.1)
            );
            crate::append_agent_trace_log(
                state,
                task,
                step,
                "invalid_multi_action_output",
                &json!({
                    "count": action_kinds.len(),
                    "selected": crate::agent_action_signature(&selected.1),
                    "candidates": action_kinds,
                    "raw_llm_out": crate::truncate_for_agent_trace(&llm_out),
                }),
            );
            selected
        } else {
            parsed_candidates.swap_remove(0)
        };

        let original_action = selected_action.clone();
        let routing_goal = user_request.trim().to_string();
        let (action, rewrite_note) = crate::rewrite_agent_action_for_safety(selected_action, &routing_goal);
        let rewrite_note = if rewrite_note.is_some() {
            rewrite_note
        } else {
            multi_action_note
        };
        if let Some(ref note) = rewrite_note {
            crate::append_routing_log(state, task, &routing_goal, &original_action, &action, note);
            history.push(format!("router: {}", note));
        }
        crate::append_agent_trace_log(
            state,
            task,
            step,
            "action_parsed",
            &json!({
                "routing_goal": crate::truncate_for_agent_trace(&routing_goal),
                "raw_llm_out": crate::truncate_for_agent_trace(&llm_out),
                "selected_json": crate::truncate_for_agent_trace(&selected_json),
                "original_action": crate::agent_action_log_value(&original_action),
                "final_action": crate::agent_action_log_value(&action),
                "rewrite_note": rewrite_note,
            }),
        );

        let pre_repeat_run_cmd_command = if let AgentAction::CallTool { tool, args } = &action {
            if tool == "run_cmd" {
                args.as_object()
                    .and_then(|m| m.get("command"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        } else {
            None
        };
        if let (Some(command), Some(last_command)) = (
            pre_repeat_run_cmd_command.as_deref(),
            last_success_run_cmd.as_deref(),
        ) {
            if command == last_command {
                let message = format!("Command already succeeded earlier; skip duplicate run_cmd: {command}");
                crate::append_agent_trace_log(
                    state,
                    task,
                    step,
                    "run_cmd_duplicate_short_circuit",
                    &json!({
                        "command": crate::truncate_for_agent_trace(command),
                    }),
                );
                history.push(format!("tool(run_cmd): {}", message));
                return Ok(AskReply::non_llm(message));
            }
        }

        let action_sig = crate::agent_action_signature(&action);
        if let AgentAction::CallSkill { skill, args } = &action {
            let canonical_skill = crate::canonical_skill_name(skill);
            let is_crypto_news = is_crypto_news_call(canonical_skill, args, &loop_guard_policy);
            let is_crypto_market_query =
                is_crypto_market_query_call(canonical_skill, args, &loop_guard_policy);
            let is_crypto_trade_preview =
                is_crypto_trade_preview_call(canonical_skill, args, &loop_guard_policy);
            let same_as_last_action =
                last_action_signature.as_deref() == Some(action_sig.as_str());
            if is_crypto_news && same_as_last_action {
                if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                    if !last_out.trim().is_empty() {
                        crate::append_agent_trace_log(
                            state,
                            task,
                            step,
                            "crypto_news_loop_short_circuit",
                            &json!({
                                "reason": "reuse previous crypto news output to avoid repeated call_skill loop",
                            }),
                        );
                        if subtask_index == 0 {
                            return Ok(AskReply::non_llm(last_out.to_string()));
                        }
                        append_and_publish_progress_message(
                            state,
                            task,
                            &mut delivery_messages,
                            last_out.to_string(),
                        );
                        history.push(format!("skill({}): {}", skill, last_out));
                        last_action_signature = Some(action_sig.clone());
                        continue;
                    }
                }
            }
            if is_crypto_market_query && same_as_last_action {
                if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                    if !last_out.trim().is_empty() {
                        crate::append_agent_trace_log(
                            state,
                            task,
                            step,
                            "crypto_market_loop_short_circuit",
                            &json!({
                                "reason": "reuse previous crypto market output to avoid repeated call_skill loop",
                                "action_signature": crate::truncate_for_agent_trace(&action_sig),
                            }),
                        );
                        // Query-type actions should converge after first successful output.
                        // Do not continue looping on identical market queries.
                        if delivery_messages
                            .last()
                            .map(|m| m.trim() == last_out.trim())
                            .unwrap_or(false)
                        {
                            return Ok(AskReply::non_llm(String::new())
                                .with_messages(delivery_messages.clone()));
                        }
                        return Ok(AskReply::non_llm(last_out.to_string()));
                    }
                }
            }
            if is_crypto_market_query && !successful_crypto_market_query_signatures.is_empty() {
                let seen_same_query = successful_crypto_market_query_signatures.contains(&action_sig);
                if seen_same_query || crypto_market_query_param_retry_used {
                    if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                        if !last_out.trim().is_empty() {
                            crate::append_agent_trace_log(
                                state,
                                task,
                                step,
                                "crypto_market_post_success_short_circuit",
                                &json!({
                                    "reason": if seen_same_query {
                                        "same market-query signature already succeeded; stop repeated loop"
                                    } else {
                                        "one parameter-changed market-query retry already used; stop repeated loop"
                                    },
                                    "action_signature": crate::truncate_for_agent_trace(&action_sig),
                                    "retry_used": crypto_market_query_param_retry_used,
                                }),
                            );
                            if delivery_messages
                                .last()
                                .map(|m| m.trim() == last_out.trim())
                                .unwrap_or(false)
                            {
                                return Ok(AskReply::non_llm(String::new())
                                    .with_messages(delivery_messages.clone()));
                            }
                            return Ok(AskReply::non_llm(last_out.to_string()));
                        }
                    }
                }
            }
            if is_crypto_trade_preview && same_as_last_action {
                if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                    if !last_out.trim().is_empty() {
                        crate::append_agent_trace_log(
                            state,
                            task,
                            step,
                            "crypto_trade_preview_loop_short_circuit",
                            &json!({
                                "reason": "reuse previous crypto trade_preview output to avoid repeated call_skill loop",
                                "action_signature": crate::truncate_for_agent_trace(&action_sig),
                            }),
                        );
                        // For trade preview loops, stop immediately to avoid repeated
                        // confirmation text flooding channel progress delivery.
                        if delivery_messages
                            .last()
                            .map(|m| m.trim() == last_out.trim())
                            .unwrap_or(false)
                        {
                            return Ok(AskReply::non_llm(String::new())
                                .with_messages(delivery_messages.clone()));
                        }
                        return Ok(AskReply::non_llm(last_out.to_string()));
                    }
                }
            }
        }
        let state_fp = crate::repeat_state_fingerprint(
            false,
            false,
            action_steps_executed,
            last_tool_or_skill_output.as_deref(),
        );
        let repeat_key = format!("{action_sig}#state:{state_fp}");
        let repeat = repeat_actions.entry(repeat_key).or_insert(0);
        *repeat += 1;
        if *repeat > crate::AGENT_REPEAT_SAME_ACTION_LIMIT {
            crate::append_agent_trace_log(
                state,
                task,
                step,
                "repeat_action_abort",
                &json!({
                    "action_signature": crate::truncate_for_agent_trace(&action_sig),
                    "repeat_count": *repeat,
                    "limit": crate::AGENT_REPEAT_SAME_ACTION_LIMIT,
                }),
            );
            return Err(format!(
                "agent repeated same action too many times: count={}, action={}",
                *repeat,
                crate::truncate_for_agent_trace(&action_sig)
            ));
        }

        match action {
            AgentAction::Think { content } => {
                history.push(format!("think: {}", content));
                last_action_signature = Some(action_sig.clone());
            }
            AgentAction::Respond { content } => {
                info!(
                    "run_agent_with_tools: task_id={} completed action_steps={} tool_calls={} planned_steps={}",
                    task.task_id, action_steps_executed, tool_calls, estimated_plan_steps
                );
                crate::append_act_plan_log(
                    state,
                    task,
                    "completed",
                    estimated_plan_steps,
                    action_steps_executed,
                    tool_calls,
                    "task completed with final respond",
                );
                let image_goal =
                    intent_router::should_apply_image_tail_handling_with_llm(state, task, &routing_goal_seed).await;
                let content = if image_goal {
                    crate::normalize_delivery_tokens_to_file(&content)
                } else {
                    content
                };
                if !last_image_file_tokens.is_empty() {
                    return Ok(AskReply::non_llm(crate::build_hardcoded_image_saved_reply(
                        &last_image_file_tokens,
                    )));
                }
                if image_goal {
                    if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                        let file_tokens = crate::extract_delivery_file_tokens(last_out);
                        if !file_tokens.is_empty() {
                            return Ok(AskReply::non_llm(crate::build_hardcoded_image_saved_reply(
                                &file_tokens,
                            )));
                        }
                    }
                }
                if image_goal && !crate::contains_delivery_file_token(&content) {
                    if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                        let normalized_last_out = crate::normalize_delivery_tokens_to_file(last_out);
                        let file_tokens = crate::extract_delivery_file_tokens(last_out);
                        if !file_tokens.is_empty() {
                            if content.trim().is_empty() {
                                return Ok(AskReply::non_llm(normalized_last_out));
                            }
                            let mut merged = content.trim().to_string();
                            if !merged.is_empty() {
                                merged.push('\n');
                            }
                            merged.push_str(&file_tokens.join("\n"));
                            return Ok(AskReply::non_llm(merged));
                        }
                    }
                }
                let mut content = content;
                if subtask_results.len() > 1 {
                    let numbered = build_numbered_subtask_summary(&subtask_results);
                    let mut prefix = crate::i18n_t_with_default(
                        state,
                        "clawd.msg.multi_subtask_prefix",
                        "Executed multiple instructions in order. Itemized results:\n",
                    );
                    prefix.push_str(&numbered);
                    if content.trim().is_empty() {
                        content = prefix;
                    } else {
                        content = format!("{prefix}\n\n{}", content.trim());
                    }
                }
                if awaiting_trade_confirmation {
                    return Ok(AskReply::llm(String::new()).with_messages(delivery_messages));
                }
                if delivery_messages
                    .last()
                    .map(|m| m.trim() == content.trim())
                    .unwrap_or(false)
                {
                    return Ok(AskReply::llm(String::new()).with_messages(delivery_messages));
                }
                if !should_send_respond_to_user {
                    let has_content = !content.trim().is_empty();
                    let has_progress = !delivery_messages.is_empty();
                    // For chat_act/multi-step requests, keep NEW conversational content
                    // (e.g. joke) while still suppressing redundant single-step summaries.
                    if has_content && (!has_progress || estimated_plan_steps > 1) {
                        if has_progress {
                            return Ok(AskReply::llm(content).with_messages(delivery_messages));
                        }
                        return Ok(AskReply::llm(content));
                    }
                    return Ok(AskReply::llm(String::new()).with_messages(delivery_messages));
                }
                if delivery_messages.len() > 1 {
                    return Ok(AskReply::llm(content).with_messages(delivery_messages));
                }
                return Ok(AskReply::llm(content));
            }
            AgentAction::CallSkill { skill, args } => {
                let canonical_skill = crate::canonical_skill_name(&skill);
                let is_crypto_market_query =
                    is_crypto_market_query_call(canonical_skill, &args, &loop_guard_policy);
                let is_crypto_trade_preview =
                    is_crypto_trade_preview_call(canonical_skill, &args, &loop_guard_policy);
                let is_crypto_trade_submit = is_crypto_trade_submit_call(canonical_skill, &args);
                if is_crypto_trade_submit && has_successful_trade_submit {
                    crate::append_agent_trace_log(
                        state,
                        task,
                        step,
                        "crypto_trade_submit_single_task_guard",
                        &json!({
                            "reason": "blocked repeated trade_submit in the same task",
                            "policy": "one_trade_submit_per_task",
                        }),
                    );
                    // Safety hard limit: one task can execute at most one trade submit.
                    // If already delivered progress messages, return them as final output directly.
                    if !delivery_messages.is_empty() {
                        return Ok(AskReply::non_llm(String::new())
                            .with_messages(delivery_messages.clone()));
                    }
                    return Ok(AskReply::non_llm(
                        "Trade submit already executed once in this task; repeated submit was blocked."
                            .to_string(),
                    ));
                }
                if canonical_skill == "image_generate" && image_generate_success_count >= 1 {
                    let fallback_tokens = if last_image_file_tokens.is_empty() {
                        if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                            crate::extract_delivery_file_tokens(last_out)
                        } else {
                            Vec::new()
                        }
                    } else {
                        last_image_file_tokens.clone()
                    };
                    crate::append_agent_trace_log(
                        state,
                        task,
                        step,
                        "image_generate_loop_short_circuit",
                        &json!({
                            "image_generate_success_count": image_generate_success_count,
                            "reason": "skip repeated image_generate after successful result",
                        }),
                    );
                    if !fallback_tokens.is_empty() {
                        return Ok(AskReply::non_llm(crate::build_hardcoded_image_saved_reply(
                            &fallback_tokens,
                        )));
                    }
                    if let Some(last_out) = last_tool_or_skill_output.as_deref() {
                        let normalized_last_out = crate::normalize_delivery_tokens_to_file(last_out);
                        if !normalized_last_out.trim().is_empty() {
                            return Ok(AskReply::non_llm(normalized_last_out));
                        }
                    }
                    return Ok(AskReply::non_llm(crate::i18n_t_with_default(
                        state,
                        "clawd.msg.image_loop_stopped",
                        "Image generation succeeded. Repeated generation has been stopped to avoid task loops.",
                    )));
                }
                consume_tool_call_budget(&mut tool_calls)?;
                subtask_index += 1;
                let current_subtask = subtask_index;
                let skill_out = match execution_adapters::run_skill(state, task, &skill, args).await {
                    Ok(v) => v,
                    Err(err) => {
                        crate::append_subtask_result(
                            &mut subtask_results,
                            current_subtask,
                            &format!("skill({skill})"),
                            false,
                            &err,
                        );
                        crate::append_agent_trace_log(
                            state,
                            task,
                            step,
                            "skill_error",
                            &json!({
                                "skill": skill,
                                "error": crate::truncate_for_agent_trace(&err),
                            }),
                        );
                        let resume_err = build_resume_context_error(
                            &plan_steps,
                            user_request,
                            goal,
                            &subtask_results,
                            &delivery_messages,
                            current_subtask,
                            &format!("skill({skill})"),
                            &err,
                        );
                        return Err(resume_err);
                    }
                };
                crate::append_subtask_result(
                    &mut subtask_results,
                    current_subtask,
                    &format!("skill({skill})"),
                    true,
                    &skill_out,
                );
                let has_delivery_file_token =
                    !crate::extract_delivery_file_tokens(&skill_out).is_empty();
                // Strategy B: media outputs are delivered only once at terminal success stage.
                // Do not publish image/file tokens as running progress messages.
                let should_publish_progress_now = !matches!(canonical_skill, "image_generate" | "image_edit")
                    && !has_delivery_file_token;
                if should_publish_progress_now && !skill_out.trim().is_empty() {
                    append_and_publish_progress_message(
                        state,
                        task,
                        &mut delivery_messages,
                        skill_out.clone(),
                    );
                }
                last_tool_or_skill_output = Some(skill_out.clone());
                if is_crypto_market_query {
                    if !successful_crypto_market_query_signatures.is_empty()
                        && !successful_crypto_market_query_signatures.contains(&action_sig)
                    {
                        crypto_market_query_param_retry_used = true;
                    }
                    successful_crypto_market_query_signatures.insert(action_sig.clone());
                }
                if is_crypto_trade_submit {
                    has_successful_trade_submit = true;
                }
                awaiting_trade_confirmation = is_crypto_trade_preview;
                if canonical_skill == "image_generate" {
                    image_generate_success_count += 1;
                }
                if canonical_skill == "image_generate" || canonical_skill == "image_edit" {
                    let tokens = crate::extract_delivery_file_tokens(&skill_out);
                    if !tokens.is_empty() {
                        last_image_file_tokens = tokens;
                    }
                }
                action_steps_executed += 1;
                last_action_signature = Some(action_sig.clone());
                crate::append_agent_trace_log(
                    state,
                    task,
                    step,
                    "skill_ok",
                    &json!({
                        "skill": skill,
                        "output_preview": crate::truncate_for_agent_trace(&skill_out),
                    }),
                );
                history.push(format!("skill({}): {}", skill, skill_out));
            }
            AgentAction::CallTool { tool, args } => {
                awaiting_trade_confirmation = false;
                let run_cmd_command = if tool == "run_cmd" {
                    args.as_object()
                        .and_then(|m| m.get("command"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                } else {
                    None
                };
                consume_tool_call_budget(&mut tool_calls)?;
                subtask_index += 1;
                let current_subtask = subtask_index;
                let out = match execution_adapters::run_tool(state, &tool, &args).await {
                    Ok(v) => v,
                    Err(err) => {
                        crate::append_subtask_result(
                            &mut subtask_results,
                            current_subtask,
                            &format!("tool({tool})"),
                            false,
                            &err,
                        );
                        crate::append_agent_trace_log(
                            state,
                            task,
                            step,
                            "tool_error",
                            &json!({
                                "tool": tool,
                                "error": crate::truncate_for_agent_trace(&err),
                            }),
                        );
                        let mut final_err = err.clone();
                        if tool == "run_cmd" {
                            let command = args
                                .as_object()
                                .and_then(|m| m.get("command"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let suggest_prompt = crate::COMMAND_FAILURE_SUGGEST_PROMPT_TEMPLATE
                                .replace("__COMMAND__", command)
                                .replace("__ERROR__", &err);
                            if let Ok(suggestion) =
                                llm_gateway::run_with_fallback(state, task, &suggest_prompt).await
                            {
                                let suggestion = suggestion.trim();
                                if !suggestion.is_empty() {
                                    let suggest_title = crate::i18n_t_with_default(
                                        state,
                                        "clawd.msg.suggestion_title",
                                        "Suggestion:",
                                    );
                                    final_err.push_str("\n\n");
                                    final_err.push_str(&suggest_title);
                                    final_err.push('\n');
                                    final_err.push_str(suggestion);
                                }
                            }
                        }
                        let resume_err = build_resume_context_error(
                            &plan_steps,
                            user_request,
                            goal,
                            &subtask_results,
                            &delivery_messages,
                            current_subtask,
                            &format!("tool({tool})"),
                            &final_err,
                        );
                        return Err(resume_err);
                    }
                };
                crate::append_subtask_result(
                    &mut subtask_results,
                    current_subtask,
                    &format!("tool({tool})"),
                    true,
                    &out,
                );
                let _ = repo::insert_audit_log(
                    state,
                    Some(task.user_id),
                    "run_tool",
                    Some(&json!({"tool": tool, "task_id": task.task_id}).to_string()),
                    None,
                );
                crate::append_agent_trace_log(
                    state,
                    task,
                    step,
                    "tool_ok",
                    &json!({
                        "tool": tool,
                        "output_preview": crate::truncate_for_agent_trace(&out),
                    }),
                );
                if tool == "run_cmd" {
                    if let Some(command) = run_cmd_command {
                        // run_tool returned Ok for run_cmd, so command already exited successfully.
                        // Mark it as succeeded even when stdout is non-empty (common case),
                        // so duplicate loop actions can be short-circuited safely.
                        last_success_run_cmd = Some(command);
                    }
                }
                last_tool_or_skill_output = Some(out.clone());
                action_steps_executed += 1;
                last_action_signature = Some(action_sig.clone());
                history.push(format!("tool({}): {}", tool, out));
            }
        }
    }

    let history_tail = history
        .iter()
        .rev()
        .take(6)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    crate::append_agent_trace_log(
        state,
        task,
        crate::AGENT_MAX_STEPS,
        "max_steps_abort",
        &json!({
            "history_tail": history_tail,
            "tool_calls": tool_calls,
            "max_steps": crate::AGENT_MAX_STEPS,
        }),
    );
    info!(
        "run_agent_with_tools: task_id={} step_limit_reached action_steps={} tool_calls={} planned_steps={} max_steps={}",
        task.task_id, action_steps_executed, tool_calls, estimated_plan_steps, crate::AGENT_MAX_STEPS
    );
    crate::append_act_plan_log(
        state,
        task,
        "step_limit_reached",
        estimated_plan_steps,
        action_steps_executed,
        tool_calls,
        &format!("max_steps={}", crate::AGENT_MAX_STEPS),
    );
    let has_explicit_task_requirements = false;
    if tool_calls == 0 && !has_explicit_task_requirements {
        if let Ok(chat_reply) = llm_gateway::run_with_fallback(state, task, &routing_goal_seed).await {
            if !chat_reply.trim().is_empty() {
                return Ok(AskReply::llm(chat_reply));
            }
        }
    }

    let mut message = format!(
        "Task exceeded step limit. Executed only the first {} step(s); remaining steps were discarded.",
        crate::AGENT_MAX_STEPS
    );
    if let Some(last) = last_tool_or_skill_output {
        let last_trimmed = last.trim();
        if !last_trimmed.is_empty() {
            message.push_str("\n\nLast completed step output:\n");
            message.push_str(&crate::truncate_for_log(last_trimmed));
        }
    }
    Ok(AskReply::non_llm(message))
}
