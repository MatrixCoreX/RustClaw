use std::collections::{BTreeMap, BTreeSet};

use crate::{skill_availability, AppState, ClaimedTask};
use claw_core::skill_registry::{
    Capability, OutputKind, PlannerCapabilityKind, PlannerCapabilityMapping, SkillRegistryEntry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CapabilityDomain {
    Filesystem,
    System,
    Config,
    Git,
    Process,
    Service,
    TaskControl,
    OpsStatus,
    MarketData,
    NewsContent,
    ImageMedia,
    AudioMedia,
    VideoMedia,
    MusicMedia,
    Publishing,
    GeneralChat,
}

impl CapabilityDomain {
    fn title(self) -> &'static str {
        match self {
            CapabilityDomain::Filesystem => "filesystem",
            CapabilityDomain::System => "system",
            CapabilityDomain::Config => "config",
            CapabilityDomain::Git => "git",
            CapabilityDomain::Process => "process",
            CapabilityDomain::Service => "service",
            CapabilityDomain::TaskControl => "task_control",
            CapabilityDomain::OpsStatus => "ops/status",
            CapabilityDomain::MarketData => "market/data",
            CapabilityDomain::NewsContent => "news/web",
            CapabilityDomain::ImageMedia => "image",
            CapabilityDomain::AudioMedia => "audio",
            CapabilityDomain::VideoMedia => "video",
            CapabilityDomain::MusicMedia => "music",
            CapabilityDomain::Publishing => "publishing",
            CapabilityDomain::GeneralChat => "chat",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            CapabilityDomain::Filesystem => {
                "inspect directories, read files, create/write files, remove files, and search the filesystem"
            }
            CapabilityDomain::System => {
                "run shell commands and inspect local system, developer, package, archive, git, HTTP, and database information"
            }
            CapabilityDomain::Config
            | CapabilityDomain::Git
            | CapabilityDomain::Process
            | CapabilityDomain::Service
            | CapabilityDomain::TaskControl => self.title(),
            CapabilityDomain::OpsStatus => {
                "check service/process/runtime task status, list or cancel the current chat's queued/running tasks, read logs, run health checks, and inspect safe config state"
            }
            CapabilityDomain::MarketData => {
                "retrieve stock and crypto quotes, market indicators, portfolio/position data, order status, and trading-related facts"
            }
            CapabilityDomain::NewsContent => {
                "fetch RSS feeds, news, and web content from external sources"
            }
            CapabilityDomain::ImageMedia => {
                "analyze images and perform image generation or editing"
            }
            CapabilityDomain::AudioMedia => "transcribe audio and synthesize spoken output",
            CapabilityDomain::VideoMedia | CapabilityDomain::MusicMedia => self.title(),
            CapabilityDomain::Publishing => "draft or send outbound social content",
            CapabilityDomain::GeneralChat => {
                "provide conversational explanation, rewriting, and smalltalk when external retrieval is not needed"
            }
        }
    }

    fn from_registry_group(group: &str) -> Option<Self> {
        match group.trim().to_ascii_lowercase().as_str() {
            "filesystem" | "file" | "files" | "fs" => Some(Self::Filesystem),
            "config" | "configuration" => Some(Self::Config),
            "git" | "vcs" | "repository" => Some(Self::Git),
            "process" | "processes" => Some(Self::Process),
            "service" | "services" => Some(Self::Service),
            "task" | "tasks" | "task_control" => Some(Self::TaskControl),
            "system" | "developer" | "dev" | "shell" | "http" | "database" | "db" | "package"
            | "archive" | "transform" | "workflow" | "flows" | "orchestration" => {
                Some(Self::System)
            }
            "ops" | "status" | "ops/status" | "runtime" => Some(Self::OpsStatus),
            "market" | "market/data" | "finance" => Some(Self::MarketData),
            "news" | "web" | "news/web" => Some(Self::NewsContent),
            "image" | "vision" => Some(Self::ImageMedia),
            "audio" | "voice" => Some(Self::AudioMedia),
            "video" => Some(Self::VideoMedia),
            "music" => Some(Self::MusicMedia),
            "publishing" | "social" => Some(Self::Publishing),
            "chat" | "general_chat" => Some(Self::GeneralChat),
            _ => None,
        }
    }
}

fn classify_skill(state: &AppState, skill: &str) -> CapabilityDomain {
    infer_domain_from_skill_metadata(state, skill)
        .or_else(|| legacy_domain_from_skill_name(skill))
        .unwrap_or(CapabilityDomain::System)
}

fn infer_domain_from_skill_metadata(state: &AppState, skill: &str) -> Option<CapabilityDomain> {
    let registry = state.get_skills_registry()?;
    let entry = registry.get(skill)?;
    infer_domain_from_registry_entry(entry)
}

fn infer_domain_from_registry_entry(entry: &SkillRegistryEntry) -> Option<CapabilityDomain> {
    if let Some(domain) = legacy_domain_from_skill_name(entry.name.trim()) {
        return Some(domain);
    }
    if let Some(domain) = entry
        .group
        .as_deref()
        .and_then(CapabilityDomain::from_registry_group)
    {
        return Some(domain);
    }
    if entry.output_kind == OutputKind::Image {
        return Some(CapabilityDomain::ImageMedia);
    }
    if entry
        .resolved_capabilities
        .iter()
        .any(|cap| matches!(cap, Capability::FsRead | Capability::FsWrite))
        && !entry
            .resolved_capabilities
            .iter()
            .any(|cap| matches!(cap, Capability::Net | Capability::Llm))
    {
        return Some(CapabilityDomain::Filesystem);
    }
    if entry
        .resolved_capabilities
        .iter()
        .any(|cap| matches!(cap, Capability::Net))
    {
        return Some(CapabilityDomain::NewsContent);
    }
    if entry
        .resolved_capabilities
        .iter()
        .any(|cap| matches!(cap, Capability::Llm))
    {
        return Some(CapabilityDomain::GeneralChat);
    }
    let canonical = entry.name.trim();
    if canonical.is_empty() {
        return None;
    }
    None
}

fn legacy_domain_from_skill_name(skill: &str) -> Option<CapabilityDomain> {
    match skill.trim().to_ascii_lowercase().as_str() {
        "stock" | "crypto" => Some(CapabilityDomain::MarketData),
        "rss_fetch" | "web_search_extract" | "browser_web" => Some(CapabilityDomain::NewsContent),
        "image_vision" | "image_generate" | "image_edit" => Some(CapabilityDomain::ImageMedia),
        "audio_transcribe" | "audio_synthesize" => Some(CapabilityDomain::AudioMedia),
        "video_generate" => Some(CapabilityDomain::VideoMedia),
        "music_generate" => Some(CapabilityDomain::MusicMedia),
        "x" => Some(CapabilityDomain::Publishing),
        "chat" => Some(CapabilityDomain::GeneralChat),
        "fs_basic" | "read_file" | "write_file" | "list_dir" | "make_dir" | "remove_file"
        | "fs_search" => Some(CapabilityDomain::Filesystem),
        "config_basic" | "config_edit" | "config_guard" => Some(CapabilityDomain::Config),
        "git_basic" => Some(CapabilityDomain::Git),
        "process_basic" => Some(CapabilityDomain::Process),
        "service_control" | "health_check" | "log_analyze" => Some(CapabilityDomain::Service),
        "task_control" => Some(CapabilityDomain::TaskControl),
        "docker_basic" => Some(CapabilityDomain::OpsStatus),
        "run_cmd" | "system_basic" | "http_basic" | "install_module" | "package_manager"
        | "archive_basic" | "db_basic" => Some(CapabilityDomain::System),
        _ => None,
    }
}

fn planner_capability_hint(mapping: &PlannerCapabilityMapping) -> String {
    let mut parts = Vec::new();
    if let Some(action) = mapping.action.as_deref() {
        parts.push(format!("action={action}"));
    }
    if let Some(effect) = mapping.effect {
        parts.push(format!("effect={}", effect.as_token()));
    }
    if !mapping.required.is_empty() {
        parts.push(format!("required={}", mapping.required.join("|")));
    }
    if !mapping.optional.is_empty() {
        parts.push(format!("optional={}", mapping.optional.join("|")));
    }
    if let Some(risk_level) = mapping.risk_level {
        parts.push(format!("risk={}", risk_level_token(risk_level)));
    }
    if mapping.preferred {
        parts.push("preferred=true".to_string());
    }
    if let Some(once_per_task) = mapping.once_per_task {
        parts.push(format!("once_per_task={once_per_task}"));
    }
    if let Some(dedup_scope) = mapping.dedup_scope {
        parts.push(format!("dedup_scope={}", dedup_scope.as_token()));
    }
    if let Some(idempotent) = mapping.idempotent {
        parts.push(format!("idempotent={idempotent}"));
    }
    if let Some(execution_mode) = mapping.execution_mode {
        parts.push(format!("execution_mode={}", execution_mode.as_token()));
    }
    if let Some(async_adapter_kind) = mapping.async_adapter_kind.as_deref() {
        parts.push(format!("async_adapter_kind={async_adapter_kind}"));
    }
    if parts.is_empty() {
        mapping.name.clone()
    } else {
        format!("{}({})", mapping.name, parts.join(","))
    }
}

fn risk_level_token(risk_level: claw_core::skill_registry::SkillRiskLevel) -> &'static str {
    match risk_level {
        claw_core::skill_registry::SkillRiskLevel::Unknown => "unknown",
        claw_core::skill_registry::SkillRiskLevel::Low => "low",
        claw_core::skill_registry::SkillRiskLevel::Medium => "medium",
        claw_core::skill_registry::SkillRiskLevel::High => "high",
    }
}

fn skill_permission_profile_hint(entry: &SkillRegistryEntry) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(risk_level) = entry.risk_level {
        parts.push(format!("risk={}", risk_level_token(risk_level)));
    }
    if let Some(requires_confirmation) = entry.requires_confirmation {
        parts.push(format!("requires_confirmation={requires_confirmation}"));
    }
    if let Some(side_effect) = entry.side_effect {
        parts.push(format!("side_effect={side_effect}"));
    }
    if let Some(auto_invocable) = entry.auto_invocable {
        parts.push(format!("auto_invocable={auto_invocable}"));
    }
    if let Some(once_per_task) = entry.once_per_task {
        parts.push(format!("once_per_task={once_per_task}"));
    }
    if let Some(dedup_scope) = entry.dedup_scope {
        parts.push(format!("dedup_scope={}", dedup_scope.as_token()));
    }
    if let Some(idempotent) = entry.idempotent {
        parts.push(format!("idempotent={idempotent}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

#[cfg(test)]
#[path = "capability_map_tests.rs"]
mod tests;
pub(crate) fn build_capability_map_for_task(state: &AppState, task: &ClaimedTask) -> String {
    let all_visible = state.planner_visible_skills_for_task(task);
    let visible = state.planner_available_skills_for_task(task);
    let available_set = visible.iter().cloned().collect::<BTreeSet<_>>();
    let unavailable_hints = unavailable_skill_hints(state, &all_visible, &available_set);
    if visible.is_empty() {
        let mut lines = vec![
            "Current runtime-available tool capabilities are unavailable; use chat only when no external retrieval or execution is needed.".to_string(),
        ];
        if !unavailable_hints.is_empty() {
            lines.push("Enabled but unavailable capabilities omitted from planning:".to_string());
            lines.extend(unavailable_hints);
        }
        return lines.join("\n");
    }

    let mut grouped: BTreeMap<CapabilityDomain, BTreeSet<String>> = BTreeMap::new();
    let mut layered: BTreeMap<PlannerCapabilityKind, BTreeSet<String>> = BTreeMap::new();
    for skill in &visible {
        grouped
            .entry(classify_skill(state, skill))
            .or_default()
            .insert(skill.clone());
        let planner_kind = state
            .get_skills_registry()
            .and_then(|registry| registry.planner_kind(skill))
            .unwrap_or(PlannerCapabilityKind::Skill);
        layered
            .entry(planner_kind)
            .or_default()
            .insert(skill.clone());
    }

    let mut lines = vec![
        "Current capability map (derived from the currently enabled skills):".to_string(),
        "Use this as routing guidance, not as a full tool schema.".to_string(),
        "Do not plan or call capabilities marked `runtime_availability: unavailable`; choose another available capability or explain the dependency gap.".to_string(),
        crate::agent_runtime_contract::runtime_protocol_hint_line(),
        crate::async_job_contract::async_job_protocol_hint_line(),
    ];

    if !layered.is_empty() {
        lines.push(
            "Capability layers: tools are low-level reusable actions, skills are domain capabilities, workflows are multi-step playbooks."
                .to_string(),
        );
        for (kind, skills) in layered {
            let label = match kind {
                PlannerCapabilityKind::Tool => "tools",
                PlannerCapabilityKind::Skill => "skills",
                PlannerCapabilityKind::Workflow => "workflows",
            };
            lines.push(format!(
                "- {label}: {}.",
                skills.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }

    for (domain, skills) in grouped {
        let skills_text = skills.into_iter().collect::<Vec<_>>().join(", ");
        lines.push(format!(
            "- {}: {}. Visible skills: {}.",
            domain.title(),
            domain.summary(),
            skills_text
        ));
    }

    if let Some(registry) = state.get_skills_registry() {
        let mut hints = Vec::new();
        for skill in &visible {
            let Some(entry) = registry.get(skill) else {
                continue;
            };
            let aliases = entry
                .aliases
                .iter()
                .map(|alias| alias.trim())
                .filter(|alias| !alias.is_empty())
                .take(6)
                .collect::<Vec<_>>();
            let description = entry
                .description
                .as_deref()
                .map(str::trim)
                .filter(|description| !description.is_empty());
            let semantic_tags = entry
                .semantic_tags
                .iter()
                .map(|tag| tag.trim())
                .filter(|tag| !tag.is_empty())
                .take(8)
                .collect::<Vec<_>>();
            let validation_actions = entry
                .validation_actions
                .iter()
                .map(|action| action.trim())
                .filter(|action| !action.is_empty())
                .take(6)
                .collect::<Vec<_>>();
            let capability_tokens = entry
                .resolved_capabilities
                .iter()
                .map(|capability| capability.as_token())
                .take(8)
                .collect::<Vec<_>>();
            let planner_capability_tokens = entry
                .planner_capabilities
                .iter()
                .map(planner_capability_hint)
                .take(12)
                .collect::<Vec<_>>();
            let supported_os = entry
                .supported_os
                .iter()
                .map(|os| os.trim())
                .filter(|os| !os.is_empty())
                .take(6)
                .collect::<Vec<_>>();
            let required_bins = entry
                .required_bins
                .iter()
                .map(|bin| bin.trim())
                .filter(|bin| !bin.is_empty())
                .take(8)
                .collect::<Vec<_>>();
            let optional_bins = entry
                .optional_bins
                .iter()
                .map(|bin| bin.trim())
                .filter(|bin| !bin.is_empty())
                .take(8)
                .collect::<Vec<_>>();
            let platform_notes = entry
                .platform_notes
                .iter()
                .map(|note| note.trim())
                .filter(|note| !note.is_empty())
                .take(2)
                .collect::<Vec<_>>();
            let planner_kind = registry
                .planner_kind(skill)
                .unwrap_or(PlannerCapabilityKind::Skill);
            if aliases.is_empty()
                && description.is_none()
                && semantic_tags.is_empty()
                && validation_actions.is_empty()
                && planner_capability_tokens.is_empty()
                && capability_tokens.is_empty()
                && supported_os.is_empty()
                && required_bins.is_empty()
                && optional_bins.is_empty()
                && platform_notes.is_empty()
                && entry.retryable.is_none()
                && entry.requires_confirmation.is_none()
                && !entry.preferred_over_run_cmd
                && planner_kind == PlannerCapabilityKind::Skill
            {
                continue;
            }
            let mut parts = Vec::new();
            parts.push(format!("planner_kind: {}", planner_kind.as_token()));
            if let Some(description) = description {
                parts.push(description.to_string());
            }
            if !semantic_tags.is_empty() {
                parts.push(format!("semantic_tags: {}", semantic_tags.join(", ")));
            }
            if entry.preferred_over_run_cmd {
                parts.push("prefer over run_cmd when semantics match".to_string());
            }
            if let Some(permission_profile) = skill_permission_profile_hint(entry) {
                parts.push(format!("permission_profile={permission_profile}"));
            }
            if !validation_actions.is_empty() {
                parts.push(format!(
                    "validation_actions: {}",
                    validation_actions.join(", ")
                ));
            }
            if !planner_capability_tokens.is_empty() {
                parts.push(format!(
                    "planner_capabilities: {}",
                    planner_capability_tokens.join("; ")
                ));
            }
            if let Some(retryable) = entry.retryable {
                parts.push(format!("retryable: {retryable}"));
            }
            if let Some(requires_confirmation) = entry.requires_confirmation {
                parts.push(format!("requires_confirmation: {requires_confirmation}"));
            }
            if !entry.confirmation_exempt_when.is_empty() {
                let exemptions = entry
                    .confirmation_exempt_when
                    .iter()
                    .take(4)
                    .map(|matcher| {
                        matcher
                            .iter()
                            .map(|(key, value)| {
                                format!("{key}={}", compact_toml_value_token(value))
                            })
                            .collect::<Vec<_>>()
                            .join("+")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                parts.push(format!("confirmation_exempt_when: {exemptions}"));
            }
            parts.extend(skill_availability::availability_metadata_parts(
                &skill_availability::evaluate_entry_availability(entry),
            ));
            if !capability_tokens.is_empty() {
                parts.push(format!("capabilities: {}", capability_tokens.join(", ")));
            }
            if !supported_os.is_empty() {
                parts.push(format!("supported_os: {}", supported_os.join(", ")));
            }
            if !required_bins.is_empty() {
                parts.push(format!("required_bins: {}", required_bins.join(", ")));
            }
            if !optional_bins.is_empty() {
                parts.push(format!("optional_bins: {}", optional_bins.join(", ")));
            }
            if !platform_notes.is_empty() {
                parts.push(format!("platform_notes: {}", platform_notes.join(" | ")));
            }
            if !aliases.is_empty() {
                parts.push(format!("aliases: {}", aliases.join(", ")));
            }
            hints.push(format!("  - {skill}: {}", parts.join("; ")));
        }
        if !hints.is_empty() {
            lines.push("Registry skill hints:".to_string());
            lines.extend(hints);
        }
    }

    if !unavailable_hints.is_empty() {
        lines.push("Enabled but unavailable capabilities omitted from planning:".to_string());
        lines.extend(unavailable_hints);
    }

    lines.push(
        "If the user is asking for current data, real system state, latest external information, or observable results, prefer `act` over `chat`."
            .to_string(),
    );
    lines.push(
        "Use `chat` only for explanation, advice, rewriting, or discussion that does not require external retrieval or execution."
            .to_string(),
    );

    lines.join("\n")
}

fn compact_toml_value_token(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Boolean(v) => v.to_string(),
        toml::Value::Integer(v) => v.to_string(),
        toml::Value::Float(v) => v.to_string(),
        toml::Value::Array(values) => values
            .iter()
            .map(compact_toml_value_token)
            .collect::<Vec<_>>()
            .join("|"),
        _ => value.to_string(),
    }
}

fn unavailable_skill_hints(
    state: &AppState,
    all_visible: &[String],
    available_set: &BTreeSet<String>,
) -> Vec<String> {
    let Some(registry) = state.get_skills_registry() else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    for skill in all_visible {
        if available_set.contains(skill) {
            continue;
        }
        let Some(entry) = registry.get(skill) else {
            continue;
        };
        let availability = skill_availability::evaluate_entry_availability(entry);
        if availability.is_available() {
            continue;
        }
        hints.push(format!(
            "  - {skill}: {}",
            skill_availability::availability_metadata_parts(&availability).join("; ")
        ));
    }
    hints
}
