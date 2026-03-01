use std::collections::HashMap;

use serde_json::{Value, json};
use tracing::{info, warn};

use crate::{execution_adapters, intent_router, llm_gateway, repo, AgentAction, AppState, AskReply, ClaimedTask};

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
    let mut tool_calls = 0usize;
    let mut repeat_actions: HashMap<String, usize> = HashMap::new();
    let mut last_tool_or_skill_output: Option<String> = None;
    let mut last_image_file_tokens: Vec<String> = Vec::new();
    let routing_goal_seed = user_request.trim().to_string();
    let mut action_steps_executed = 0usize;
    let mut subtask_index = 0usize;
    let mut subtask_results: Vec<String> = Vec::new();
    let mut last_success_run_cmd: Option<String> = None;
    let estimated_plan_steps = 1usize;
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

    for step in 1..=crate::AGENT_MAX_STEPS {
        let tool_spec = "Tools: read_file(path), write_file(path,content), list_dir(path), run_cmd(command). Skills: image_vision(action=describe|extract|compare|screenshot_summary, images=[{path|url|base64}]), image_generate(prompt,size?,style?,quality?,n?,output_path?), image_edit(action=edit|outpaint|restyle|add_remove, image?, instruction, mask?, output_path?), x(text, dry_run?, send?). Return exactly one action JSON per turn. For simple save-a-file tasks, prefer write_file directly (use run_cmd mkdir -p once only when target folder is missing). For image generation requests, prefer call_skill image_generate directly. For image edit requests that reference an earlier image without explicit path, still call image_edit with instruction; backend may resolve the image from memory/history. For X posting requests, call_skill x with text first; keep dry_run=true unless user explicitly asks to publish and set send=true.";
        let hist_text = if history.is_empty() {
            "(empty)".to_string()
        } else {
            history.join("\n")
        };

        let prompt = crate::AGENT_RUNTIME_PROMPT_TEMPLATE
            .replace("__PERSONA_PROMPT__", &state.persona_prompt)
            .replace("__TOOL_SPEC__", tool_spec)
            .replace("__GOAL__", goal)
            .replace("__STEP__", &step.to_string())
            .replace("__HISTORY__", &hist_text);
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
            AgentAction::Think { content } => history.push(format!("think: {}", content)),
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
                return Ok(AskReply::llm(content));
            }
            AgentAction::CallSkill { skill, args } => {
                if tool_calls >= crate::AGENT_MAX_TOOL_CALLS {
                    return Err("agent tool call limit exceeded".to_string());
                }
                tool_calls += 1;
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
                        let prefix = crate::i18n_t_with_default(
                            state,
                            "clawd.msg.skill_exec_error_prefix",
                            "技能执行错误：",
                        );
                        return Err(format!("{prefix}{err}"));
                    }
                };
                crate::append_subtask_result(
                    &mut subtask_results,
                    current_subtask,
                    &format!("skill({skill})"),
                    true,
                    &skill_out,
                );
                last_tool_or_skill_output = Some(skill_out.clone());
                let canonical_skill = crate::canonical_skill_name(&skill);
                if canonical_skill == "image_generate" || canonical_skill == "image_edit" {
                    let tokens = crate::extract_delivery_file_tokens(&skill_out);
                    if !tokens.is_empty() {
                        last_image_file_tokens = tokens;
                    }
                }
                action_steps_executed += 1;
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
                let run_cmd_command = if tool == "run_cmd" {
                    args.as_object()
                        .and_then(|m| m.get("command"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                } else {
                    None
                };
                if tool_calls >= crate::AGENT_MAX_TOOL_CALLS {
                    return Err("agent tool call limit exceeded".to_string());
                }
                tool_calls += 1;
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
                        let mut final_err = if tool == "run_cmd" {
                            let prefix = crate::i18n_t_with_default(
                                state,
                                "clawd.msg.command_exec_error_prefix",
                                "命令执行错误：",
                            );
                            format!("{prefix}{err}")
                        } else {
                            let prefix = crate::i18n_t_with_default(
                                state,
                                "clawd.msg.tool_exec_error_prefix",
                                "工具执行错误：",
                            );
                            format!("{prefix}{err}")
                        };
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
                                        "建议：",
                                    );
                                    final_err.push_str("\n\n");
                                    final_err.push_str(&suggest_title);
                                    final_err.push('\n');
                                    final_err.push_str(suggestion);
                                }
                            }
                        }
                        return Err(final_err);
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
