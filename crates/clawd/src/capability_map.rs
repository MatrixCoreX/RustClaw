use std::collections::{BTreeMap, BTreeSet};

use crate::{skill_availability, AppState, ClaimedTask};
use claw_core::skill_registry::{
    Capability, OutputKind, PlannerCapabilityKind, SkillRegistryEntry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CapabilityDomain {
    Filesystem,
    System,
    OpsStatus,
    MarketData,
    NewsContent,
    ImageMedia,
    AudioMedia,
    Publishing,
    GeneralChat,
}

impl CapabilityDomain {
    fn title(self) -> &'static str {
        match self {
            CapabilityDomain::Filesystem => "filesystem",
            CapabilityDomain::System => "system",
            CapabilityDomain::OpsStatus => "ops/status",
            CapabilityDomain::MarketData => "market/data",
            CapabilityDomain::NewsContent => "news/web",
            CapabilityDomain::ImageMedia => "image",
            CapabilityDomain::AudioMedia => "audio",
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
            CapabilityDomain::Publishing => "draft or send outbound social content",
            CapabilityDomain::GeneralChat => {
                "provide conversational explanation, rewriting, and smalltalk when external retrieval is not needed"
            }
        }
    }

    fn from_registry_group(group: &str) -> Option<Self> {
        match group.trim().to_ascii_lowercase().as_str() {
            "filesystem" | "file" | "files" | "fs" => Some(Self::Filesystem),
            "system" | "developer" | "dev" | "shell" | "http" | "database" | "db" | "package"
            | "archive" | "transform" | "workflow" | "flows" | "orchestration" => {
                Some(Self::System)
            }
            "ops" | "status" | "ops/status" | "service" | "runtime" | "config" => {
                Some(Self::OpsStatus)
            }
            "market" | "market/data" | "finance" => Some(Self::MarketData),
            "news" | "web" | "news/web" => Some(Self::NewsContent),
            "image" | "vision" => Some(Self::ImageMedia),
            "audio" | "voice" => Some(Self::AudioMedia),
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
    legacy_domain_from_skill_name(canonical)
}

fn legacy_domain_from_skill_name(skill: &str) -> Option<CapabilityDomain> {
    match skill.trim().to_ascii_lowercase().as_str() {
        "stock" | "crypto" => Some(CapabilityDomain::MarketData),
        "rss_fetch" | "web_search_extract" | "browser_web" => Some(CapabilityDomain::NewsContent),
        "image_vision" | "image_generate" | "image_edit" => Some(CapabilityDomain::ImageMedia),
        "audio_transcribe" | "audio_synthesize" => Some(CapabilityDomain::AudioMedia),
        "x" => Some(CapabilityDomain::Publishing),
        "chat" => Some(CapabilityDomain::GeneralChat),
        "fs_basic" | "read_file" | "write_file" | "list_dir" | "make_dir" | "remove_file"
        | "fs_search" => Some(CapabilityDomain::Filesystem),
        "process_basic" | "docker_basic" | "health_check" | "log_analyze" | "service_control"
        | "task_control" | "config_guard" => Some(CapabilityDomain::OpsStatus),
        "run_cmd" | "system_basic" | "http_basic" | "git_basic" | "install_module"
        | "package_manager" | "archive_basic" | "db_basic" => Some(CapabilityDomain::System),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_core::skill_registry::SkillsRegistry;

    fn registry_entry_from(toml: &str, name: &str) -> SkillRegistryEntry {
        let path = std::env::temp_dir().join(format!("capability_map_{name}.toml"));
        std::fs::write(&path, toml).unwrap();
        let registry = SkillsRegistry::load_from_path(&path).unwrap();
        let entry = registry.get(name).unwrap().clone();
        let _ = std::fs::remove_file(path);
        entry
    }

    #[test]
    fn registry_group_controls_capability_domain() {
        let entry = registry_entry_from(
            r#"
[[skills]]
name = "custom_web_tool"
enabled = true
planner_kind = "tool"
group = "news/web"
capabilities = ["net"]
"#,
            "custom_web_tool",
        );
        assert_eq!(
            infer_domain_from_registry_entry(&entry),
            Some(CapabilityDomain::NewsContent)
        );
    }

    #[test]
    fn filesystem_capability_infers_domain_without_skill_name() {
        let entry = registry_entry_from(
            r#"
[[skills]]
name = "custom_reader"
enabled = true
planner_kind = "tool"
capabilities = ["fs.read"]
"#,
            "custom_reader",
        );
        assert_eq!(
            infer_domain_from_registry_entry(&entry),
            Some(CapabilityDomain::Filesystem)
        );
    }
}

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
            if !validation_actions.is_empty() {
                parts.push(format!(
                    "validation_actions: {}",
                    validation_actions.join(", ")
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
