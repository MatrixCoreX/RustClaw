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

fn self_extension_t(
    state: Option<&AppState>,
    language: ReplyLanguage,
    key: &str,
    default_zh: &str,
    default_en: &str,
) -> String {
    match language {
        ReplyLanguage::ZhCn => state.map_or_else(
            || default_zh.to_string(),
            |state| {
                crate::app_helpers::bilingual_t_with_default(
                    state, key, default_zh, default_en, false,
                )
            },
        ),
        ReplyLanguage::En => state.map_or_else(
            || default_en.to_string(),
            |state| {
                crate::app_helpers::bilingual_t_with_default(
                    state, key, default_zh, default_en, true,
                )
            },
        ),
    }
}

fn self_extension_t_with_vars(
    state: Option<&AppState>,
    language: ReplyLanguage,
    key: &str,
    default_zh: &str,
    default_en: &str,
    vars: &[(&str, &str)],
) -> String {
    match language {
        ReplyLanguage::ZhCn => state.map_or_else(
            || {
                let mut text = default_zh.to_string();
                for (name, value) in vars {
                    text = text.replace(&format!("{{{name}}}"), value);
                }
                text
            },
            |state| {
                crate::bilingual_t_with_default_vars(
                    state, key, default_zh, default_en, false, vars,
                )
            },
        ),
        ReplyLanguage::En => state.map_or_else(
            || {
                let mut text = default_en.to_string();
                for (name, value) in vars {
                    text = text.replace(&format!("{{{name}}}"), value);
                }
                text
            },
            |state| {
                crate::bilingual_t_with_default_vars(state, key, default_zh, default_en, true, vars)
            },
        ),
    }
}

fn text_contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        )
    })
}

fn text_contains_ascii_alpha(text: &str) -> bool {
    text.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn request_language(state: &AppState, request: &str) -> ReplyLanguage {
    let trimmed = request.trim();
    if trimmed.is_empty() {
        return if state.policy.command_intent.default_locale.starts_with("en") {
            ReplyLanguage::En
        } else {
            ReplyLanguage::ZhCn
        };
    }
    match (
        text_contains_cjk(trimmed),
        text_contains_ascii_alpha(trimmed),
    ) {
        (true, false) => ReplyLanguage::ZhCn,
        (false, true) => ReplyLanguage::En,
        (true, true) => {
            if state.policy.command_intent.default_locale.starts_with("en") {
                ReplyLanguage::En
            } else {
                ReplyLanguage::ZhCn
            }
        }
        (false, false) => {
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
    route: &crate::RouteResult,
    request: &str,
    execution_user_request: &str,
) -> bool {
    !matches!(
        crate::execution_recipe::detect_execution_recipe(
            Some(route),
            request,
            execution_user_request
        )
        .kind,
        crate::execution_recipe::ExecutionRecipeKind::None
    )
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
        .unwrap_or("Temporary fix plan generated.")
        .to_string()
}

fn localized_plan_reply(
    state: Option<&AppState>,
    language: ReplyLanguage,
    plan: &Value,
    will_execute: bool,
    allow_package_install: bool,
) -> String {
    let (files, commands, packages) = plan_counts(plan);
    let summary = plan_summary(plan);
    let files_text = files.to_string();
    let commands_text = commands.to_string();
    let packages_text = packages.to_string();
    let package_note = if packages > 0 && !allow_package_install {
        self_extension_t(
            state,
            language,
            "clawd.msg.self_extension.package_install_disabled_note",
            " 方案包含依赖安装，但当前配置未允许自动安装。",
            " The plan includes package installation, but automatic package install is currently disabled.",
        )
    } else {
        String::new()
    };
    let text = if will_execute {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.temporary_plan_execute",
            "当前没有合适的现成技能完全覆盖这个请求。我已生成临时方案并准备执行：{summary}",
            "No existing skill cleanly covers this request. I generated a bounded temporary plan and am executing it: {summary}",
            &[("summary", summary.as_str())],
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.temporary_plan_pending",
            "当前没有合适的现成技能完全覆盖这个请求。我已生成一份受控临时方案，暂未执行：{summary} 预计会写入 {files} 个临时文件、运行 {commands} 条命令、涉及 {packages} 组依赖。{package_note}",
            "No existing skill cleanly covers this request. I created a bounded temporary plan but did not execute it yet: {summary} It would write {files} temporary file(s), run {commands} command(s), and involve {packages} package group(s).{package_note}",
            &[
                ("summary", summary.as_str()),
                ("files", files_text.as_str()),
                ("commands", commands_text.as_str()),
                ("packages", packages_text.as_str()),
                ("package_note", package_note.as_str()),
            ],
        )
    };
    text.trim().to_string()
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
    state: Option<&AppState>,
    language: ReplyLanguage,
    detail: &str,
) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        self_extension_t(
            state,
            language,
            "clawd.msg.self_extension.failure",
            "我尝试走受控自扩展链，但这次没有成功。",
            "I tried the controlled self-extension path, but it did not succeed.",
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.failure_with_detail",
            "我尝试走受控自扩展链，但这次没有成功：{detail}",
            "I tried the controlled self-extension path, but it did not succeed: {detail}",
            &[("detail", trimmed)],
        )
    }
}

fn localized_permanent_plan_reply(
    state: Option<&AppState>,
    language: ReplyLanguage,
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
    let actions_text = actions.to_string();
    match (language, materialized) {
        (_, false) => {
            let mut text = self_extension_t_with_vars(
                state,
                language,
                "clawd.msg.self_extension.permanent_plan_pending",
                "这个请求更适合做成可复用能力。我已生成外部技能脚手架方案，但当前未自动落地：建议技能名 `{skill_name}`，摘要是“{capability_summary}”，包含 {actions} 个动作。",
                "This request looks better as a reusable capability. I generated an external-skill scaffold plan but did not materialize it yet: suggested skill name `{skill_name}`, summary \"{capability_summary}\", with {actions} action(s).",
                &[
                    ("skill_name", skill_name),
                    ("capability_summary", capability_summary),
                    ("actions", actions_text.as_str()),
                ],
            );
            if !rationale.is_empty() {
                text.push_str(&self_extension_t_with_vars(
                    state,
                    language,
                    "clawd.msg.self_extension.permanent_plan_rationale_suffix",
                    " 原因：{rationale}",
                    " Rationale: {rationale}",
                    &[("rationale", rationale)],
                ));
            }
            text
        }
        (_, true) => self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.permanent_plan_materialized",
            "我已按开发态流程生成并填充 `external_skills/{skill_name}` 的初始实现，并完成文档同步、编译检查和协议级 smoke test。它仍未注册、未启用；接下来只需要人工复核后显式启用。",
            "I scaffolded and filled the first implementation for `external_skills/{skill_name}` through the developer extension flow, and I already synced docs, ran cargo check, and passed a protocol smoke test. It is still neither registered nor enabled; the next step is explicit human review and enablement.",
            &[("skill_name", skill_name)],
        ),
    }
}

fn localized_permanent_runtime_enabled_reply(
    state: Option<&AppState>,
    language: ReplyLanguage,
    skill_name: &str,
) -> String {
    self_extension_t_with_vars(
        state,
        language,
        "clawd.msg.self_extension.permanent_enabled",
        "我已按开发态流程完成 `external_skills/{skill_name}` 的生成、验证、注册、启用，并已重载技能视图。它现在可以被运行时识别，但仍建议先人工复核再正常使用。",
        "I completed generation, validation, registration, enablement, and skill-view reload for `external_skills/{skill_name}` through the developer extension flow. It is now visible to the runtime, but it should still be reviewed before normal use.",
        &[("skill_name", skill_name)],
    )
}

fn localized_permanent_materialization_failure(
    state: Option<&AppState>,
    language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.materialization_failure",
            "我已生成 `external_skills/{skill_name}` 的脚手架，但自动填充初始实现没有完成。该技能仍未注册、未启用。",
            "I scaffolded `external_skills/{skill_name}`, but the initial implementation generation did not finish. The skill is still not registered or enabled.",
            &[("skill_name", skill_name)],
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.materialization_failure_with_detail",
            "我已生成 `external_skills/{skill_name}` 的脚手架，但自动填充初始实现没有完成：{detail}。该技能仍未注册、未启用。",
            "I scaffolded `external_skills/{skill_name}`, but the initial implementation generation did not finish: {detail}. The skill is still not registered or enabled.",
            &[("skill_name", skill_name), ("detail", trimmed)],
        )
    }
}

fn localized_permanent_validation_failure(
    state: Option<&AppState>,
    language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.validation_failure",
            "我已生成并填充 `external_skills/{skill_name}`，但后续的文档同步、编译检查或 smoke test 没有全部通过。该技能仍未注册、未启用。",
            "I scaffolded and filled `external_skills/{skill_name}`, but the follow-up doc sync, compile check, or smoke test did not fully pass. The skill is still not registered or enabled.",
            &[("skill_name", skill_name)],
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.validation_failure_with_detail",
            "我已生成并填充 `external_skills/{skill_name}`，但后续的文档同步、编译检查或 smoke test 没有全部通过：{detail}。该技能仍未注册、未启用。",
            "I scaffolded and filled `external_skills/{skill_name}`, but the follow-up doc sync, compile check, or smoke test did not fully pass: {detail}. The skill is still not registered or enabled.",
            &[("skill_name", skill_name), ("detail", trimmed)],
        )
    }
}

fn localized_permanent_registration_failure(
    state: Option<&AppState>,
    language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.registration_failure",
            "我已生成并验证 `external_skills/{skill_name}`，但注册到工作区和技能配置的步骤没有完成。该技能仍未进入运行时。",
            "I generated and validated `external_skills/{skill_name}`, but the workspace/registry registration step did not complete. The skill is still not in the runtime.",
            &[("skill_name", skill_name)],
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.registration_failure_with_detail",
            "我已生成并验证 `external_skills/{skill_name}`，但注册到工作区和技能配置的步骤没有完成：{detail}。该技能仍未进入运行时。",
            "I generated and validated `external_skills/{skill_name}`, but the workspace/registry registration step did not complete: {detail}. The skill is still not in the runtime.",
            &[("skill_name", skill_name), ("detail", trimmed)],
        )
    }
}

fn localized_permanent_enable_failure(
    state: Option<&AppState>,
    language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.enable_failure",
            "我已生成、验证并注册 `external_skills/{skill_name}`，但启用或 release 构建没有完成。该技能还不能正常进入运行时。",
            "I generated, validated, and registered `external_skills/{skill_name}`, but enablement or release build did not complete. The skill is not ready for runtime use yet.",
            &[("skill_name", skill_name)],
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.enable_failure_with_detail",
            "我已生成、验证并注册 `external_skills/{skill_name}`，但启用或 release 构建没有完成：{detail}。该技能还不能正常进入运行时。",
            "I generated, validated, and registered `external_skills/{skill_name}`, but enablement or release build did not complete: {detail}. The skill is not ready for runtime use yet.",
            &[("skill_name", skill_name), ("detail", trimmed)],
        )
    }
}

fn localized_permanent_reload_failure(
    state: Option<&AppState>,
    language: ReplyLanguage,
    skill_name: &str,
    detail: &str,
) -> String {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.reload_failure",
            "我已生成、验证并启用 `external_skills/{skill_name}`，但重载技能视图没有完成。请手动 reload 或重启 clawd。",
            "I generated, validated, and enabled `external_skills/{skill_name}`, but skill-view reload did not finish. Please reload skills manually or restart clawd.",
            &[("skill_name", skill_name)],
        )
    } else {
        self_extension_t_with_vars(
            state,
            language,
            "clawd.msg.self_extension.reload_failure_with_detail",
            "我已生成、验证并启用 `external_skills/{skill_name}`，但重载技能视图没有完成：{detail}。请手动 reload 或重启 clawd。",
            "I generated, validated, and enabled `external_skills/{skill_name}`, but skill-view reload did not finish: {detail}. Please reload skills manually or restart clawd.",
            &[("skill_name", skill_name), ("detail", trimmed)],
        )
    }
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
        return Ok(AskReply::non_llm(localized_extension_failure(
            state,
            language,
            &skill_error_text(&plan_value),
        )));
    }
    let Some(plan) = plan_from_skill_output(&plan_value) else {
        let fallback = plan_value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        return Ok(AskReply::non_llm(if fallback.is_empty() {
            localized_extension_failure(state, language, "")
        } else {
            fallback
        }));
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
        return Ok(AskReply::non_llm(localized_extension_failure(
            state,
            language,
            &skill_error_text(&execute_value),
        )));
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
        return Ok(AskReply::non_llm(localized_extension_failure(
            state,
            language,
            &skill_error_text(&plan_value),
        )));
    }
    let Some(plan) = plan_from_skill_output(&plan_value) else {
        return Ok(AskReply::non_llm(localized_extension_failure(
            state,
            language,
            "missing permanent extension plan",
        )));
    };
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
        return Ok(AskReply::non_llm(localized_extension_failure(
            state,
            language,
            &skill_error_text(&scaffold_value),
        )));
    }

    let skill_name = plan
        .get("skill_name")
        .and_then(|v| v.as_str())
        .unwrap_or("generated_extension");
    let implement_args = json!({
        "action": "implement_external_skill",
        "request": request,
        "skill_name": skill_name,
        "capability_summary": plan.get("capability_summary").and_then(|v| v.as_str()).unwrap_or("Reusable capability scaffold."),
        "actions": plan.get("actions").cloned().unwrap_or_else(|| json!(["todo_action"])),
    });
    let implement_value = run(implement_args).await?;
    if !skill_status_ok(&implement_value) {
        return Ok(AskReply::non_llm(
            localized_permanent_materialization_failure(
                state,
                language,
                skill_name,
                &skill_error_text(&implement_value),
            ),
        ));
    }
    let validate_args = json!({
        "action": "validate_external_skill",
        "skill_name": skill_name,
        "actions": plan.get("actions").cloned().unwrap_or_else(|| json!(["todo_action"])),
    });
    let validate_value = run(validate_args).await?;
    if !skill_status_ok(&validate_value) {
        return Ok(AskReply::non_llm(localized_permanent_validation_failure(
            state,
            language,
            skill_name,
            &skill_error_text(&validate_value),
        )));
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
        return Ok(AskReply::non_llm(localized_permanent_registration_failure(
            state,
            language,
            skill_name,
            &skill_error_text(&register_value),
        )));
    }

    let enable_args = json!({
        "action": "enable_external_skill",
        "skill_name": skill_name,
        "confirm": true,
    });
    let enable_value = run(enable_args).await?;
    if !skill_status_ok(&enable_value) {
        return Ok(AskReply::non_llm(localized_permanent_enable_failure(
            state,
            language,
            skill_name,
            &skill_error_text(&enable_value),
        )));
    }
    if let Err(err) = reload() {
        return Ok(AskReply::non_llm(localized_permanent_reload_failure(
            state, language, skill_name, &err,
        )));
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
    if should_bypass_self_extension_for_execution_recipe(route, &request, execution_user_request) {
        tracing::info!(
            "{} self_extension bypassed for active execution recipe task_id={} route_mode={:?}",
            crate::highlight_tag("self_extension"),
            task.task_id,
            route.routed_mode
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
mod tests {
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
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
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
                self_extension: crate::SelfExtensionContract {
                    mode: crate::SelfExtensionMode::TemporaryFix,
                    trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                    execute_now: true,
                },
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(!self_extension_enabled_for_route(false, false, &route));
        assert!(self_extension_enabled_for_route(true, false, &route));
    }

    #[test]
    fn capability_gap_trigger_requires_auto_flag() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
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
                self_extension: crate::SelfExtensionContract {
                    mode: crate::SelfExtensionMode::TemporaryFix,
                    trigger: crate::SelfExtensionTrigger::CapabilityGap,
                    execute_now: false,
                },
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(!self_extension_enabled_for_route(true, false, &route));
        assert!(self_extension_enabled_for_route(true, true, &route));
    }

    #[test]
    fn ops_closed_loop_request_bypasses_self_extension() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "verify local http service; if verification fails, repair config and re-verify until passing".to_string(),
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
                self_extension: crate::SelfExtensionContract {
                    mode: crate::SelfExtensionMode::TemporaryFix,
                    trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                    execute_now: false,
                },
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(should_bypass_self_extension_for_execution_recipe(
            &route,
            "verify local http service; if verification fails, repair document/index.html and re-verify until passing",
            "Do not modify any file or process before the first verification attempt. First verify the local static HTTP service. If that fails, repair document/index.html and verify again until it passes."
        ));
    }

    #[test]
    fn plain_temporary_fix_request_does_not_bypass_self_extension() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "solve this with a temporary script".to_string(),
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
                self_extension: crate::SelfExtensionContract {
                    mode: crate::SelfExtensionMode::TemporaryFix,
                    trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                    execute_now: false,
                },
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        assert!(!should_bypass_self_extension_for_execution_recipe(
            &route,
            "solve this with a temporary script",
            "Write a small temporary script to process this file."
        ));
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
        let reply =
            localized_permanent_runtime_enabled_reply(None, ReplyLanguage::En, "pdf_compare");
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
        let reply = localized_permanent_reload_failure(
            None,
            ReplyLanguage::En,
            "pdf_compare",
            "reload failed",
        );
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
            routed_mode: crate::RoutedMode::Chat,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Chat),
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
}
