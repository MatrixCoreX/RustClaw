use std::future::Future;

use serde_json::{json, Value};

use crate::{
    runtime::state::reload_skill_views, worker::task_runtime_channel, AppState, AskReply,
    ClaimedTask, SelfExtensionMode, SelfExtensionTrigger,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyLanguage {
    ZhCn,
    En,
}

fn request_language(state: &AppState, request: &str) -> ReplyLanguage {
    match crate::language_policy::request_language_hint(request) {
        "zh-CN" => ReplyLanguage::ZhCn,
        "en" => ReplyLanguage::En,
        _ => {
            if state.policy.command_intent.default_locale.starts_with("en") {
                ReplyLanguage::En
            } else {
                ReplyLanguage::ZhCn
            }
        }
    }
}

fn effective_request(
    resolved_prompt: &str,
    execution_user_request: &str,
    route: &crate::RouteResult,
) -> String {
    let resolved = route.resolved_intent.trim();
    if !resolved.is_empty() {
        return resolved.to_string();
    }
    let execution = execution_user_request.trim();
    if !execution.is_empty() {
        return execution.to_string();
    }
    resolved_prompt.trim().to_string()
}

fn self_extension_enabled_for_route(
    enabled: bool,
    auto_on_capability_gap: bool,
    route: &crate::RouteResult,
) -> bool {
    if !enabled || route.needs_clarify {
        return false;
    }
    if route.ask_mode.is_clarify_only() {
        return false;
    }
    let directive = &route.output_contract.self_extension;
    if matches!(directive.mode, SelfExtensionMode::None) {
        return false;
    }
    if matches!(directive.trigger, SelfExtensionTrigger::CapabilityGap) && !auto_on_capability_gap {
        return false;
    }
    true
}

fn should_handle_self_extension(state: &AppState, route: &crate::RouteResult) -> bool {
    self_extension_enabled_for_route(
        state.policy.self_extension.enabled,
        state.policy.self_extension.auto_on_capability_gap,
        route,
    )
}

fn should_bypass_self_extension_for_execution_recipe(
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> bool {
    execution_recipe_hint.is_some_and(|spec| {
        !matches!(
            spec.kind,
            crate::execution_recipe::ExecutionRecipeKind::None
        )
    })
}

fn runtime_source(state: &AppState, task: &ClaimedTask) -> &'static str {
    match task_runtime_channel(state, task) {
        crate::RuntimeChannel::Whatsapp => "whatsapp",
        crate::RuntimeChannel::Telegram => "telegram",
        crate::RuntimeChannel::Wechat => "wechat",
        crate::RuntimeChannel::Feishu => "feishu",
        crate::RuntimeChannel::Lark => "lark",
    }
}

async fn run_extension_manager(
    state: &AppState,
    task: &ClaimedTask,
    args: &Value,
) -> Result<Value, String> {
    let _permit = state
        .skill_rt
        .skill_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| format!("skill semaphore closed: {err}"))?;
    let runner_name = state.runner_name_for_skill("extension_manager");
    crate::skills::run_skill_with_runner_once(
        state,
        task,
        "extension_manager",
        &runner_name,
        args,
        runtime_source(state, task),
        state.skill_rt.skill_timeout_seconds.max(30),
    )
    .await
}

fn skill_status_ok(value: &Value) -> bool {
    value.get("status").and_then(|v| v.as_str()) == Some("ok")
}

fn skill_error_text(value: &Value) -> String {
    value
        .get("error_text")
        .and_then(|v| v.as_str())
        .unwrap_or("extension_manager failed")
        .trim()
        .to_string()
}

fn plan_from_skill_output(value: &Value) -> Option<Value> {
    value.get("extra").and_then(|v| v.get("plan")).cloned()
}

fn plan_counts(plan: &Value) -> (usize, usize, usize) {
    let files = plan
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    let commands = plan
        .get("commands")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    let packages = plan
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    (files, commands, packages)
}

fn plan_summary(plan: &Value) -> String {
    plan.get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn localized_plan_reply(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    plan: &Value,
    will_execute: bool,
    allow_package_install: bool,
) -> String {
    let (files, commands, packages) = plan_counts(plan);
    let summary = plan_summary(plan);
    let mut payload = json!({
        "message_key": "clawd.msg.self_extension.temporary_plan",
        "reason_code": if will_execute { "self_extension_temporary_plan_execute" } else { "self_extension_temporary_plan_pending" },
        "will_execute": will_execute,
        "files": files,
        "commands": commands,
        "packages": packages,
        "package_install_allowed": allow_package_install,
        "package_install_blocked": packages > 0 && !allow_package_install,
    });
    if !summary.is_empty() {
        payload["summary"] = json!(summary);
    }
    payload.to_string()
}

fn extract_best_execution_output(value: &Value) -> Option<String> {
    let runs = value
        .get("extra")
        .and_then(|v| v.get("command_runs"))
        .and_then(|v| v.as_array())?;
    let last_non_empty = runs.iter().rev().find_map(|run| {
        run.get("stdout")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|stdout| !stdout.is_empty())
            .map(ToString::to_string)
    });
    last_non_empty.or_else(|| {
        runs.iter().rev().find_map(|run| {
            run.get("stderr")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|stderr| !stderr.is_empty())
                .map(ToString::to_string)
        })
    })
}

fn localized_extension_failure(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    detail: &str,
) -> String {
    self_extension_failure_machine_payload(
        "clawd.msg.self_extension.failure",
        "self_extension_failure",
        "",
        "self_extension",
        detail,
    )
}

fn self_extension_message_key_from_reason(reason_code: &str) -> String {
    let suffix = reason_code
        .trim()
        .strip_prefix("self_extension_")
        .unwrap_or(reason_code.trim())
        .replace('_', ".");
    if suffix.is_empty() {
        "clawd.msg.self_extension.failure".to_string()
    } else {
        format!("clawd.msg.self_extension.{suffix}")
    }
}

fn self_extension_failure_machine_payload(
    message_key: &str,
    reason_code: &str,
    skill_name: &str,
    phase: &str,
    detail: &str,
) -> String {
    let mut payload = json!({
        "message_key": message_key,
        "reason_code": reason_code,
    });
    let skill_name = skill_name.trim();
    if !skill_name.is_empty() {
        payload["skill_name"] = json!(skill_name);
        payload["skill_path"] = json!(format!("external_skills/{skill_name}"));
    }
    let phase = phase.trim();
    if !phase.is_empty() {
        payload["phase"] = json!(phase);
    }
    let detail = detail.trim();
    if !detail.is_empty() {
        payload["detail"] = json!(detail);
    }
    payload.to_string()
}

fn localized_permanent_plan_reply(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    plan: &Value,
    materialized: bool,
) -> String {
    let skill_name = plan
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or("generated_extension");
    let capability_summary = plan
        .get("capability_summary")
        .and_then(|v| v.as_str())
        .unwrap_or("Reusable capability scaffold.");
    let actions = plan
        .get("actions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    let rationale = plan
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    let mut payload = json!({
        "message_key": "clawd.msg.self_extension.permanent_plan",
        "reason_code": if materialized { "self_extension_permanent_plan_materialized" } else { "self_extension_permanent_plan_pending" },
        "skill_name": skill_name,
        "skill_path": format!("external_skills/{skill_name}"),
        "capability_summary": capability_summary,
        "actions": actions,
        "materialized": materialized,
        "registered": false,
        "enabled": false,
    });
    if !rationale.is_empty() {
        payload["rationale"] = json!(rationale);
    }
    payload.to_string()
}

fn localized_permanent_runtime_enabled_reply(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    skill_name: &str,
) -> String {
    json!({
        "message_key": "clawd.msg.self_extension.permanent_enabled",
        "reason_code": "self_extension_permanent_enabled",
        "skill_name": skill_name,
        "skill_path": format!("external_skills/{skill_name}"),
        "registered": true,
        "enabled": true,
        "reload_completed": true,
        "runtime_visible": true,
    })
    .to_string()
}

fn localized_permanent_materialization_failure(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    self_extension_failure_machine_payload(
        "clawd.msg.self_extension.materialization_failure",
        "self_extension_materialization_failure",
        skill_name,
        "materialization",
        detail,
    )
}

fn localized_permanent_validation_failure(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    self_extension_failure_machine_payload(
        "clawd.msg.self_extension.validation_failure",
        "self_extension_validation_failure",
        skill_name,
        "validation",
        detail,
    )
}

fn localized_permanent_registration_failure(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    self_extension_failure_machine_payload(
        "clawd.msg.self_extension.registration_failure",
        "self_extension_registration_failure",
        skill_name,
        "registration",
        detail,
    )
}

fn localized_permanent_enable_failure(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    self_extension_failure_machine_payload(
        "clawd.msg.self_extension.enable_failure",
        "self_extension_enable_failure",
        skill_name,
        "enable",
        detail,
    )
}

fn localized_permanent_reload_failure(
    _state: Option<&AppState>,
    _language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    self_extension_failure_machine_payload(
        "clawd.msg.self_extension.reload_failure",
        "self_extension_reload_failure",
        skill_name,
        "reload",
        detail,
    )
}

async fn compose_permanent_extension_failure_reply(
    state: Option<&AppState>,
    task: Option<&ClaimedTask>,
    language: ReplyLanguage,
    reason_code: &str,
    request: &str,
    skill_name: &str,
    phase: &str,
    detail: &str,
    _default_text: &str,
) -> String {
    let default_payload = self_extension_failure_machine_payload(
        &self_extension_message_key_from_reason(reason_code),
        reason_code,
        skill_name,
        phase,
        detail,
    );
    let (Some(state), Some(task)) = (state, task) else {
        return default_payload;
    };
    let language_hint = crate::language_policy::task_response_language_hint(state, task, request);
    let mut observed_facts = Vec::new();
    if !skill_name.trim().is_empty() {
        observed_facts.push(format!("skill_path: external_skills/{skill_name}"));
    }
    observed_facts.push(format!("failure_phase: {phase}"));
    if !detail.trim().is_empty() {
        observed_facts.push(format!("failure_detail: {}", detail.trim()));
    }
    let response_shape = match language {
        ReplyLanguage::ZhCn | ReplyLanguage::En => "brief_failure_with_next_step",
    };
    let contract = crate::fallback::UserResponseContract::tool_failure(
        reason_code,
        request,
        request,
        observed_facts,
        vec![
            "runtime_ready_claim_allowed=false".to_string(),
            "expose_internal_build_details=false".to_string(),
            "response_scope=blocked_phase_and_next_step".to_string(),
            "next_step_policy=one_concrete".to_string(),
        ],
        response_shape,
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_payload,
    )
    .await
}

async fn compose_temporary_extension_failure_reply(
    state: Option<&AppState>,
    task: Option<&ClaimedTask>,
    language: ReplyLanguage,
    reason_code: &str,
    request: &str,
    phase: &str,
    detail: &str,
    _default_text: &str,
) -> String {
    let default_payload = self_extension_failure_machine_payload(
        &self_extension_message_key_from_reason(reason_code),
        reason_code,
        "",
        phase,
        detail,
    );
    let (Some(state), Some(task)) = (state, task) else {
        return default_payload;
    };
    let language_hint = crate::language_policy::task_response_language_hint(state, task, request);
    let mut observed_facts = vec![
        "extension_type: temporary_fix".to_string(),
        format!("failure_phase: {phase}"),
    ];
    if !detail.trim().is_empty() {
        observed_facts.push(format!("failure_detail: {}", detail.trim()));
    }
    let response_shape = match language {
        ReplyLanguage::ZhCn | ReplyLanguage::En => "brief_failure_with_next_step",
    };
    let contract = crate::fallback::UserResponseContract::tool_failure(
        reason_code,
        request,
        request,
        observed_facts,
        vec![
            "temporary_fix_success_claim_allowed=false".to_string(),
            "expose_internal_build_details=false".to_string(),
            "response_scope=blocked_phase_and_next_step".to_string(),
            "next_step_policy=one_concrete".to_string(),
        ],
        response_shape,
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_payload,
    )
    .await
}

async fn handle_temporary_fix(
    state: &AppState,
    task: &ClaimedTask,
    request: &str,
    execute_now: bool,
    language: ReplyLanguage,
) -> Result<AskReply, String> {
    handle_temporary_fix_with(
        Some(state),
        Some(task),
        &state.policy.self_extension,
        request,
        execute_now,
        language,
        |args| async move { run_extension_manager(state, task, &args).await },
    )
    .await
}

async fn handle_temporary_fix_with<Run, Fut>(
    state: Option<&AppState>,
    task: Option<&ClaimedTask>,
    runtime: &claw_core::config::SelfExtensionConfig,
    request: &str,
    execute_now: bool,
    language: ReplyLanguage,
    mut run: Run,
) -> Result<AskReply, String>
where
    Run: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let plan_args = json!({
        "action": "temporary_fix_plan",
        "request": request,
    });
    let plan_value = run(plan_args).await?;
    if !skill_status_ok(&plan_value) {
        let detail = skill_error_text(&plan_value);
        let default_text = localized_extension_failure(state, language, &detail);
        let reply = compose_temporary_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_temporary_plan_failure",
            request,
            "temporary_fix_plan",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    let Some(plan) = plan_from_skill_output(&plan_value) else {
        let detail = "missing temporary fix plan";
        let default_text = localized_extension_failure(state, language, detail);
        let reply = compose_temporary_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_temporary_plan_missing",
            request,
            "temporary_fix_plan",
            detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    };

    let plan_requires_install = plan
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false);
    let can_execute = execute_now
        && runtime.allow_execute
        && (!plan_requires_install || runtime.allow_package_install);
    if !can_execute {
        return Ok(AskReply::non_llm(localized_plan_reply(
            state,
            language,
            &plan,
            false,
            runtime.allow_package_install,
        )));
    }

    let execute_args = json!({
        "action": "temporary_fix_execute",
        "confirm": true,
        "allow_package_install": runtime.allow_package_install,
        "plan": plan,
    });
    let execute_value = run(execute_args.clone()).await?;
    if !skill_status_ok(&execute_value) {
        let detail = skill_error_text(&execute_value);
        let default_text = localized_extension_failure(state, language, &detail);
        let reply = compose_temporary_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_temporary_execute_failure",
            request,
            "temporary_fix_execute",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    if let Some(output) = extract_best_execution_output(&execute_value) {
        return Ok(AskReply::non_llm(output));
    }
    let fallback = execute_value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if !fallback.is_empty() {
        return Ok(AskReply::non_llm(fallback.to_string()));
    }
    Ok(AskReply::non_llm(localized_plan_reply(
        state,
        language,
        &execute_args["plan"],
        true,
        runtime.allow_package_install,
    )))
}

async fn handle_permanent_extension(
    state: &AppState,
    task: &ClaimedTask,
    request: &str,
    execute_now: bool,
    language: ReplyLanguage,
) -> Result<AskReply, String> {
    handle_permanent_extension_with(
        Some(state),
        Some(task),
        &state.policy.self_extension,
        request,
        execute_now,
        language,
        |args| async move { run_extension_manager(state, task, &args).await },
        || reload_skill_views(state).map(|_| ()),
    )
    .await
}

async fn handle_permanent_extension_with<Run, Fut, Reload>(
    state: Option<&AppState>,
    task: Option<&ClaimedTask>,
    runtime: &claw_core::config::SelfExtensionConfig,
    request: &str,
    execute_now: bool,
    language: ReplyLanguage,
    mut run: Run,
    mut reload: Reload,
) -> Result<AskReply, String>
where
    Run: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
    Reload: FnMut() -> Result<(), String>,
{
    let plan_args = json!({
        "action": "permanent_extension_plan",
        "request": request,
    });
    let plan_value = run(plan_args).await?;
    if !skill_status_ok(&plan_value) {
        let detail = skill_error_text(&plan_value);
        let default_text = localized_extension_failure(state, language, &detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_permanent_plan_failure",
            request,
            "",
            "permanent_extension_plan",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    let Some(plan) = plan_from_skill_output(&plan_value) else {
        let detail = "missing permanent extension plan";
        let default_text = localized_extension_failure(state, language, detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_permanent_plan_missing",
            request,
            "",
            "permanent_extension_plan",
            detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    };
    let skill_name = plan
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or("generated_extension");
    if !(execute_now && runtime.allow_permanent_extension) {
        return Ok(AskReply::non_llm(localized_permanent_plan_reply(
            state, language, &plan, false,
        )));
    }

    let scaffold_args = json!({
        "action": "scaffold_external_skill",
        "skill_name": plan.get("skill_name").and_then(|v| v.as_str()).unwrap_or("generated_extension"),
        "capability_summary": plan.get("capability_summary").and_then(|v| v.as_str()).unwrap_or("Reusable capability scaffold."),
        "actions": plan.get("actions").cloned().unwrap_or_else(|| json!(["todo_action"])),
    });
    let scaffold_value = run(scaffold_args).await?;
    if !skill_status_ok(&scaffold_value) {
        let detail = skill_error_text(&scaffold_value);
        let default_text =
            localized_permanent_materialization_failure(state, language, skill_name, &detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_scaffold_failure",
            request,
            skill_name,
            "scaffold_external_skill",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }

    let implement_args = json!({
        "action": "implement_external_skill",
        "request": request,
        "skill_name": skill_name,
        "capability_summary": plan.get("capability_summary").and_then(|v| v.as_str()).unwrap_or("Reusable capability scaffold."),
        "actions": plan.get("actions").cloned().unwrap_or_else(|| json!(["todo_action"])),
    });
    let implement_value = run(implement_args).await?;
    if !skill_status_ok(&implement_value) {
        let detail = skill_error_text(&implement_value);
        let default_text =
            localized_permanent_materialization_failure(state, language, skill_name, &detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_materialization_failure",
            request,
            skill_name,
            "implement_external_skill",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    let validate_args = json!({
        "action": "validate_external_skill",
        "skill_name": skill_name,
        "actions": plan.get("actions").cloned().unwrap_or_else(|| json!(["todo_action"])),
    });
    let validate_value = run(validate_args).await?;
    if !skill_status_ok(&validate_value) {
        let detail = skill_error_text(&validate_value);
        let default_text =
            localized_permanent_validation_failure(state, language, skill_name, &detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_validation_failure",
            request,
            skill_name,
            "validate_external_skill",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    if !runtime.allow_runtime_enable {
        return Ok(AskReply::non_llm(localized_permanent_plan_reply(
            state, language, &plan, true,
        )));
    }

    let register_args = json!({
        "action": "register_external_skill",
        "skill_name": skill_name,
        "confirm": true,
    });
    let register_value = run(register_args).await?;
    if !skill_status_ok(&register_value) {
        let detail = skill_error_text(&register_value);
        let default_text =
            localized_permanent_registration_failure(state, language, skill_name, &detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_registration_failure",
            request,
            skill_name,
            "register_external_skill",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }

    let enable_args = json!({
        "action": "enable_external_skill",
        "skill_name": skill_name,
        "confirm": true,
    });
    let enable_value = run(enable_args).await?;
    if !skill_status_ok(&enable_value) {
        let detail = skill_error_text(&enable_value);
        let default_text = localized_permanent_enable_failure(state, language, skill_name, &detail);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_enable_failure",
            request,
            skill_name,
            "enable_external_skill",
            &detail,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    if let Err(err) = reload() {
        let default_text = localized_permanent_reload_failure(state, language, skill_name, &err);
        let reply = compose_permanent_extension_failure_reply(
            state,
            task,
            language,
            "self_extension_reload_failure",
            request,
            skill_name,
            "reload_skill_views",
            &err,
            &default_text,
        )
        .await;
        return Ok(AskReply::non_llm(reply));
    }
    return Ok(AskReply::non_llm(
        localized_permanent_runtime_enabled_reply(state, language, skill_name),
    ));
}

pub(crate) async fn maybe_handle_ask_self_extension(
    state: &AppState,
    task: &ClaimedTask,
    resolved_prompt: &str,
    execution_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Result<Option<AskReply>, String> {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return Ok(None);
    };
    if !should_handle_self_extension(state, route) {
        return Ok(None);
    }

    let request = effective_request(resolved_prompt, execution_user_request, route);
    if should_bypass_self_extension_for_execution_recipe(
        agent_run_context.and_then(|ctx| ctx.execution_recipe_hint),
    ) {
        tracing::info!(
            "{} self_extension bypassed for active execution recipe task_id={} ask_mode={} legacy_route_label={}",
            crate::highlight_tag("self_extension"),
            task.task_id,
            route.ask_mode.as_str(),
            route.legacy_route_label_for_trace()
        );
        return Ok(None);
    }
    let language = request_language(state, execution_user_request);
    let directive = &route.output_contract.self_extension;
    let reply = match directive.mode {
        SelfExtensionMode::TemporaryFix => {
            handle_temporary_fix(state, task, &request, directive.execute_now, language).await?
        }
        SelfExtensionMode::PermanentExtension => {
            handle_permanent_extension(state, task, &request, directive.execute_now, language)
                .await?
        }
        SelfExtensionMode::None => return Ok(None),
    };
    Ok(Some(reply))
}

#[cfg(test)]
#[path = "self_extension_tests.rs"]
mod tests;
