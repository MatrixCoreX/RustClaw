use std::path::Path;
use std::time::Instant;

use claw_core::{
    config::{AppConfig, PersonaConfig},
    prompt_layers::{self, ResolvedPromptTemplate},
};
use tracing::{info, warn};

use crate::{llm_vendor_name, AppState};

/// §3.5b: clawd 主 crate 启动期校验所覆盖的 prompt 逻辑路径清单。
///
/// 每一项形式为 `(logical_path, label)`：
/// - `logical_path` 是 `crate::load_prompt_template_for_state(...)` 调用面上喂入的相对路径，
///   会经过 [`prompt_layers::resolve_prompt_rel_path_for_vendor`] 解析到具体磁盘文件 / manifest 拼层。
/// - `label` 是日志/告警里给运维看的人类可读名字。
///
/// 列表与 `crates/clawd/src/{main.rs,intent_router.rs,agent_engine.rs,semantic_judge.rs,
/// agent_engine/observed_output.rs,skills/builtin.rs,memory/service.rs,schedule_service.rs,
/// ask_flow.rs}` 里 `include_str!` 的兜底常量一一对应；若新增 prompt 兜底常量请同步追加这里
/// 或显式标注「不参与启动校验」。
const CORE_PROMPT_REGISTRY: &[(&str, &str)] = &[
    (
        "prompts/intent_normalizer_prompt.md",
        "intent_normalizer (routing)",
    ),
    (
        "prompts/contract_repair_judge_prompt.md",
        "contract_repair_judge (routing)",
    ),
    (
        "prompts/direct_answer_gate_prompt.md",
        "direct_answer_gate (ask_flow)",
    ),
    (
        "prompts/clarify_question_prompt.md",
        "clarify_question (routing)",
    ),
    (
        "prompts/chat_response_prompt.md",
        "chat_response (ask_flow)",
    ),
    (
        "prompts/resume_continue_execute_prompt.md",
        "resume_continue_execute (ask_flow)",
    ),
    (
        "prompts/resume_followup_discussion_prompt.md",
        "resume_followup_discussion (ask_flow / worker.ask_pipeline)",
    ),
    (
        "prompts/long_term_summary_prompt.md",
        "long_term_summary (memory.service)",
    ),
    (
        "prompts/schedule_intent_prompt.md",
        "schedule_intent (schedule_service)",
    ),
    (
        "prompts/delivery_text_classifier_prompt.md",
        "delivery_text_classifier (semantic_judge.finalize)",
    ),
    (
        "prompts/agent_tool_spec.md",
        "agent_tool_spec (agent_engine)",
    ),
    (
        "prompts/single_plan_execution_prompt.md",
        "single_plan_execution (agent_engine.planning)",
    ),
    (
        "prompts/lightweight_execution_prompt.md",
        "lightweight_execution (agent_engine.planning)",
    ),
    (
        "prompts/loop_incremental_plan_prompt.md",
        "loop_incremental_plan (agent_engine.planning)",
    ),
    (
        "prompts/plan_repair_prompt.md",
        "plan_repair (agent_engine.planning)",
    ),
    (
        "prompts/observed_answer_fallback_prompt.md",
        "observed_answer_fallback (finalize.observed)",
    ),
    (
        "prompts/user_response_composer_prompt.md",
        "user_response_composer (fallback.response)",
    ),
    (
        "prompts/user_response_contract_validator_prompt.md",
        "user_response_contract_validator (fallback.response)",
    ),
    (
        "prompts/answer_verifier_prompt.md",
        "answer_verifier (finalize.answer)",
    ),
];

/// §3.5b: 启动期 prompt 校验单条记录。
#[derive(Debug, Clone)]
pub(crate) struct PromptValidationIssue {
    pub logical_path: String,
    pub label: String,
    pub resolved_disk_path: String,
}

/// §3.5b: 启动期校验汇总 —— `checked` 总数 + `missing` 命中 fallback 的清单。
#[derive(Debug, Clone, Default)]
pub(crate) struct PromptValidationReport {
    pub checked: usize,
    pub missing: Vec<PromptValidationIssue>,
    pub active_llm_vendor: Option<String>,
    pub vendor: String,
}

/// §3.5b: 校验每个核心 prompt 是否能从磁盘 / manifest 拼层加载到非空内容。
///
/// 实现策略：把 `default_template` 喂成空串调一次 `load_prompt_template_for_vendor_with_meta`，
/// 若返回的 `template` 仍为空字符串，则说明 disk + manifest 都没货，运行时会跌回 `include_str!`
/// 兜底常量。这种隐式兜底在生产应被运维感知，所以列入 `missing`。
///
/// 仅做日志告警（WARN），**不**阻塞启动 —— 编译期 `include_str!` 已确保最坏情况下也有可用模板。
pub(crate) fn validate_core_prompts(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
) -> PromptValidationReport {
    let vendor = prompt_vendor_name_from_selected_vendor(selected_vendor);
    let mut report = PromptValidationReport {
        active_llm_vendor: selected_vendor
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        vendor: vendor.clone(),
        ..Default::default()
    };
    for (logical_path, label) in CORE_PROMPT_REGISTRY {
        report.checked += 1;
        let resolved = prompt_layers::load_prompt_template_for_vendor_with_meta(
            workspace_root,
            &vendor,
            logical_path,
            "",
        );
        if resolved.template.trim().is_empty() {
            report.missing.push(PromptValidationIssue {
                logical_path: (*logical_path).to_string(),
                label: (*label).to_string(),
                resolved_disk_path: resolved.source,
            });
        }
    }
    report
}

/// §3.5b: 把 [`validate_core_prompts`] 的结果按运维语义打印到日志。
///
/// - 全部命中磁盘：单行 INFO 总结。
/// - 有 fallback 命中：每条 WARN，最后给一条汇总，便于排查打包/部署遗漏。
pub(crate) fn log_prompt_validation_report(report: &PromptValidationReport) {
    let active_llm_vendor = report.active_llm_vendor.as_deref().unwrap_or("default");
    if report.missing.is_empty() {
        info!(
            "prompt_validation: all {} core prompts resolved from disk/manifest \
            (active_llm_vendor={} prompt_vendor_patch={})",
            report.checked, active_llm_vendor, report.vendor
        );
        return;
    }
    for issue in &report.missing {
        warn!(
            "prompt_validation: fallback to embedded include_str! template (\
            disk/manifest empty) logical_path={} label={} resolved_disk_path={} \
            active_llm_vendor={} prompt_vendor_patch={}",
            issue.logical_path,
            issue.label,
            issue.resolved_disk_path,
            active_llm_vendor,
            report.vendor
        );
    }
    warn!(
        "prompt_validation: {} of {} core prompts fell back to embedded constants \
        (active_llm_vendor={} prompt_vendor_patch={}); \
        production deployments should ship the prompts/ tree alongside the binary",
        report.missing.len(),
        report.checked,
        active_llm_vendor,
        report.vendor
    );
}

/// 严格模式下把启动期 prompt 校验结果转成拒启错误信息。
///
/// 只在 `report.missing` 非空时返回 Some；调用方可在 log 之后 `bail!`。
pub(crate) fn strict_prompt_validation_error(report: &PromptValidationReport) -> Option<String> {
    if report.missing.is_empty() {
        return None;
    }
    let details = report
        .missing
        .iter()
        .map(|issue| {
            format!(
                "{} ({}) -> {}",
                issue.label, issue.logical_path, issue.resolved_disk_path
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "prompt_validation strict mode blocked startup: {} of {} core prompts fell back to embedded templates (active_llm_vendor={} prompt_vendor_patch={}): {}",
        report.missing.len(),
        report.checked,
        report.active_llm_vendor.as_deref().unwrap_or("default"),
        report.vendor,
        details
    ))
}

fn builtin_persona_prompt(profile: &str) -> &'static str {
    match profile {
        "expert" => {
            "Persona profile: expert. Be rigorous and concise. Explain key trade-offs, assumptions, and verification steps. Prefer correctness and safety over speed."
        }
        "teacher" => {
            "Persona profile: teacher. Be patient, beginner-friendly, and clear. Explain in simple steps, define necessary terms briefly, and help the user build understanding without unnecessary jargon."
        }
        "advisor" => {
            "Persona profile: advisor. Be calm, balanced, and recommendation-oriented. Help the user choose a sensible default, explain the main trade-offs briefly, and optimize for practical decisions."
        }
        "reviewer" => {
            "Persona profile: reviewer. Be critical, precise, and risk-aware. Surface the most important issues first, distinguish severity clearly, and avoid softening concrete problems."
        }
        "companion" => {
            "Persona profile: companion. Be friendly and supportive while staying practical. Keep responses clear and encouraging, but still action-oriented."
        }
        _ => {
            "Persona profile: executor. Be direct and efficient. Give conclusion first, then minimal actionable details. Prioritize execution quality and safety."
        }
    }
}

pub(crate) fn load_persona_prompt(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    cfg: &PersonaConfig,
) -> String {
    let raw_profile = cfg.profile.trim().to_ascii_lowercase();
    let profile = match raw_profile.as_str() {
        "expert" | "companion" | "executor" | "teacher" | "advisor" | "reviewer" => raw_profile,
        other => {
            warn!("unknown persona profile={}, fallback to executor", other);
            "executor".to_string()
        }
    };
    let dir = if cfg.dir.trim().is_empty() {
        "prompts/personas".to_string()
    } else {
        cfg.dir.trim().to_string()
    };
    let rel_path = format!("{dir}/{profile}.md");
    let (template, resolved_path) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        &rel_path,
        builtin_persona_prompt(&profile),
    );
    let text = template.trim();
    if text.is_empty() {
        warn!(
            "persona prompt source resolved empty, fallback to built-in: path={}",
            resolved_path
        );
        builtin_persona_prompt(&profile).to_string()
    } else {
        text.to_string()
    }
}

pub(crate) fn prompt_vendor_name_from_selected_vendor(selected_vendor: Option<&str>) -> String {
    selected_vendor
        .map(prompt_layers::normalize_prompt_vendor_name)
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) fn active_prompt_vendor_name(state: &AppState) -> String {
    if let Some(provider) = state.core.llm_providers.first() {
        return prompt_layers::normalize_prompt_vendor_name(llm_vendor_name(provider));
    }
    if let Some(active) = state.core.active_provider_type.as_deref() {
        return prompt_layers::normalize_prompt_vendor_name(active);
    }
    "default".to_string()
}

pub(crate) fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    prompt_layers::resolve_prompt_rel_path_for_vendor(workspace_root, vendor, rel_path)
}

pub(crate) fn load_prompt_template_for_vendor(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = prompt_vendor_name_from_selected_vendor(selected_vendor);
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        &vendor,
        rel_path,
        default_template,
    )
}

pub(crate) fn load_required_prompt_template_for_vendor(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    rel_path: &str,
) -> Result<(String, String), RequiredPromptLoadError> {
    let resolved = load_required_prompt_template_for_vendor_with_meta(
        workspace_root,
        selected_vendor,
        rel_path,
    )?;
    Ok((resolved.template, resolved.source))
}

pub(crate) fn load_prompt_template_for_state(
    state: &AppState,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let resolved = load_prompt_template_for_state_with_meta(state, rel_path, default_template);
    (resolved.template, resolved.source)
}

#[derive(Debug, Clone)]
pub(crate) struct RequiredPromptLoadError {
    pub logical_path: String,
    pub resolved_source: String,
    pub vendor: String,
}

impl std::fmt::Display for RequiredPromptLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "required prompt missing from disk/manifest: logical_path={} resolved_source={} vendor={}",
            self.logical_path, self.resolved_source, self.vendor
        )
    }
}

impl std::error::Error for RequiredPromptLoadError {}

pub(crate) fn load_required_prompt_template_for_state(
    state: &AppState,
    rel_path: &str,
) -> Result<(String, String), RequiredPromptLoadError> {
    let resolved = load_required_prompt_template_for_state_with_meta(state, rel_path)?;
    Ok((resolved.template, resolved.source))
}

pub(crate) fn load_required_prompt_template_for_vendor_with_meta(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    rel_path: &str,
) -> Result<ResolvedPromptTemplate, RequiredPromptLoadError> {
    let vendor = prompt_vendor_name_from_selected_vendor(selected_vendor);
    let resolved = prompt_layers::load_prompt_template_for_vendor_with_meta(
        workspace_root,
        &vendor,
        rel_path,
        "",
    );
    if resolved.template.trim().is_empty() {
        return Err(RequiredPromptLoadError {
            logical_path: rel_path.to_string(),
            resolved_source: resolved.source,
            vendor,
        });
    }
    Ok(resolved)
}

/// §3.5a: 带 prompt version 元数据的加载入口。
///
/// 与 [`load_prompt_template_for_state`] 行为一致，额外返回 `Option<String>` 版本号，
/// 便于审计日志记录。逐步把关键审计点从 `load_prompt_template_for_state` 迁移到这里。
pub(crate) fn load_prompt_template_for_state_with_meta(
    state: &AppState,
    rel_path: &str,
    default_template: &str,
) -> ResolvedPromptTemplate {
    let vendor = active_prompt_vendor_name(state);
    prompt_layers::load_prompt_template_for_vendor_with_meta(
        &state.skill_rt.workspace_root,
        &vendor,
        rel_path,
        default_template,
    )
}

pub(crate) fn load_required_prompt_template_for_state_with_meta(
    state: &AppState,
    rel_path: &str,
) -> Result<ResolvedPromptTemplate, RequiredPromptLoadError> {
    load_required_prompt_template_for_vendor_with_meta(
        &state.skill_rt.workspace_root,
        Some(&active_prompt_vendor_name(state)),
        rel_path,
    )
}

/// §3.5d: prompt hot-reload 汇总报告。`persona/schedule_*` 字段记录 reload 前后的字符数；
/// `validation` 是重跑的 [`validate_core_prompts`] 结果；`elapsed_ms` 给运维看本次 reload 耗时。
///
/// 字段服务于单测和运行期 SIGHUP listener 摘要日志。
#[derive(Debug, Clone)]
pub(crate) struct PromptReloadReport {
    pub persona_chars_before: usize,
    pub persona_chars_after: usize,
    pub schedule_intent_chars_before: usize,
    pub schedule_intent_chars_after: usize,
    pub schedule_rules_chars_before: usize,
    pub schedule_rules_chars_after: usize,
    pub validation: PromptValidationReport,
    pub elapsed_ms: u64,
    pub config_reread_ok: bool,
}

impl PromptReloadReport {
    pub(crate) fn trace_summary(&self) -> String {
        format!(
            "elapsed_ms={} config_reread_ok={} persona_chars=({}->{}) schedule_intent_chars=({}->{}) schedule_rules_chars=({}->{}) validation_missing={}/{}",
            self.elapsed_ms,
            self.config_reread_ok,
            self.persona_chars_before,
            self.persona_chars_after,
            self.schedule_intent_chars_before,
            self.schedule_intent_chars_after,
            self.schedule_rules_chars_before,
            self.schedule_rules_chars_after,
            self.validation.missing.len(),
            self.validation.checked
        )
    }
}

/// §3.5d: prompt hot-reload 主入口。
///
/// 设计要点：
/// - 大部分 `load_prompt_template_for_state_*` 路径已经每次调用时从磁盘读 prompt，
///   编辑 `prompts/layers/**/*.md` 下次 LLM 调用就生效，**无须**任何 reload 操作。
/// - 真正在启动期被快照进内存的字段只有：
///     - `policy.persona_prompt`（`load_persona_prompt` 的产物）
///     - `policy.schedule.intent_prompt_template` / `intent_rules_template`
///       （`load_schedule_runtime` 的产物）
/// - 本函数只负责 swap 上面三个字段的内部内容，并复用 [`validate_core_prompts`]
///   把所有核心 prompt 再校验一次，便于运维感知"我刚才编辑的文件是不是写崩了"。
///
/// 行为：
/// 1. 重读 `configs/config.toml`（用以拿到最新 `persona.profile` / `schedule.*_path`
///    等可能被同步编辑的字段）；解析失败时跳过 reload 并返回 `config_reread_ok=false`，
///    避免半截配置污染运行时（fail-soft：不动现状）。
/// 2. 从最新 config + 当前 `workspace_root` 重新加载 persona / schedule，
///    用 `replace_persona_prompt` / `replace_intent_*_template` 写入 `state.policy`。
/// 3. 复用 `validate_core_prompts` 跑一遍所有核心 prompt 的"磁盘有内容吗"。
///
/// 不做：
/// - 不改 vendor / 不切换 provider / 不重建 LLM 连接池 —— 这些是另一个项目（重启级别）。
/// - 不清 `semantic_judge` 的 task-scoped cache —— key 含 `task_id`，新任务自然吃新 prompt；
///   旧任务继续用上一版判定是合理的"运行中事务隔离"行为。
pub(crate) fn reload_runtime_prompts(state: &AppState, config_path: &str) -> PromptReloadReport {
    reload_runtime_prompts_impl(&state.skill_rt.workspace_root, &state.policy, config_path)
}

/// §3.5d: testable inner — 只依赖 `workspace_root + PolicyConfig`，便于单测构造。
pub(crate) fn reload_runtime_prompts_impl(
    workspace_root: &Path,
    policy: &crate::PolicyConfig,
    config_path: &str,
) -> PromptReloadReport {
    let started = Instant::now();
    let persona_chars_before = policy.persona_prompt_string().chars().count();
    let schedule_intent_chars_before = policy
        .schedule
        .intent_prompt_template_string()
        .chars()
        .count();
    let schedule_rules_chars_before = policy
        .schedule
        .intent_rules_template_string()
        .chars()
        .count();

    let workspace_root = workspace_root.to_path_buf();

    let (
        persona_chars_after,
        schedule_intent_chars_after,
        schedule_rules_chars_after,
        config_reread_ok,
        vendor_for_validation,
    ) = match AppConfig::load(config_path) {
        Ok(new_config) => {
            let vendor = new_config.llm.selected_vendor.clone();
            let new_persona =
                load_persona_prompt(&workspace_root, vendor.as_deref(), &new_config.persona);
            match super::config_loaders::load_schedule_runtime(
                &workspace_root,
                &new_config.schedule,
                vendor.as_deref(),
            ) {
                Ok(new_schedule) => {
                    let new_intent_template = new_schedule.intent_prompt_template_string();
                    let new_rules_template = new_schedule.intent_rules_template_string();
                    let pa = new_persona.chars().count();
                    let sia = new_intent_template.chars().count();
                    let sra = new_rules_template.chars().count();
                    policy.replace_persona_prompt(new_persona);
                    policy
                        .schedule
                        .replace_intent_prompt_template(new_intent_template);
                    policy
                        .schedule
                        .replace_intent_rules_template(new_rules_template);
                    (pa, sia, sra, true, vendor)
                }
                Err(err) => {
                    warn!(
                        "prompt_hot_reload: schedule prompt reload failed, keeping current schedule prompts: err={}",
                        err
                    );
                    (
                        persona_chars_before,
                        schedule_intent_chars_before,
                        schedule_rules_chars_before,
                        false,
                        vendor,
                    )
                }
            }
        }
        Err(err) => {
            warn!(
                "prompt_hot_reload: re-read config failed, keeping current prompts: path={} err={}",
                config_path, err
            );
            (
                persona_chars_before,
                schedule_intent_chars_before,
                schedule_rules_chars_before,
                false,
                None,
            )
        }
    };

    let validation = validate_core_prompts(&workspace_root, vendor_for_validation.as_deref());
    log_prompt_validation_report(&validation);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    info!(
        "prompt_hot_reload: done elapsed_ms={} config_reread_ok={} \
        persona_chars=({}->{}) schedule_intent_chars=({}->{}) schedule_rules_chars=({}->{}) \
        validation_missing={}/{}",
        elapsed_ms,
        config_reread_ok,
        persona_chars_before,
        persona_chars_after,
        schedule_intent_chars_before,
        schedule_intent_chars_after,
        schedule_rules_chars_before,
        schedule_rules_chars_after,
        validation.missing.len(),
        validation.checked
    );

    PromptReloadReport {
        persona_chars_before,
        persona_chars_after,
        schedule_intent_chars_before,
        schedule_intent_chars_after,
        schedule_rules_chars_before,
        schedule_rules_chars_after,
        validation,
        elapsed_ms,
        config_reread_ok,
    }
}

#[cfg(test)]
#[path = "prompts_tests.rs"]
mod tests;
