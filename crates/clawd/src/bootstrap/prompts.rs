use std::path::Path;

use claw_core::{
    config::PersonaConfig,
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
        "prompts/chat_skill_system_prompt.md",
        "chat_skill_system (skills.builtin.chat)",
    ),
    (
        "prompts/chat_skill_joke_system_prompt.md",
        "chat_skill_joke_system (skills.builtin.chat)",
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
        vendor: vendor.clone(),
        ..Default::default()
    };
    for (logical_path, label) in CORE_PROMPT_REGISTRY {
        report.checked += 1;
        let resolved =
            prompt_layers::load_prompt_template_for_vendor_with_meta(workspace_root, &vendor, logical_path, "");
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
    if report.missing.is_empty() {
        info!(
            "prompt_validation: all {} core prompts resolved from disk/manifest (vendor={})",
            report.checked, report.vendor
        );
        return;
    }
    for issue in &report.missing {
        warn!(
            "prompt_validation: fallback to embedded include_str! template (\
            disk/manifest empty) logical_path={} label={} resolved_disk_path={} vendor={}",
            issue.logical_path, issue.label, issue.resolved_disk_path, report.vendor
        );
    }
    warn!(
        "prompt_validation: {} of {} core prompts fell back to embedded constants (vendor={}); \
        production deployments should ship the prompts/ tree alongside the binary",
        report.missing.len(),
        report.checked,
        report.vendor
    );
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

pub(crate) fn load_prompt_template_for_state(
    state: &AppState,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = active_prompt_vendor_name(state);
    prompt_layers::load_prompt_template_for_vendor(
        &state.skill_rt.workspace_root,
        &vendor,
        rel_path,
        default_template,
    )
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
