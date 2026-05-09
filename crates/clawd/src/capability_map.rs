use std::collections::{BTreeMap, BTreeSet};

use crate::{AppState, ClaimedTask};
use claw_core::skill_registry::SkillRegistryEntry;

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
            "system" | "developer" | "dev" => Some(Self::System),
            "ops" | "status" | "ops/status" | "service" | "runtime" => Some(Self::OpsStatus),
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
    match skill {
        "read_file" | "write_file" | "list_dir" | "make_dir" | "remove_file" | "fs_search" => {
            CapabilityDomain::Filesystem
        }
        "run_cmd" | "system_basic" | "http_basic" | "git_basic" | "install_module"
        | "package_manager" | "archive_basic" | "db_basic" => CapabilityDomain::System,
        "process_basic" | "docker_basic" | "health_check" | "log_analyze" | "service_control"
        | "task_control" | "config_guard" => CapabilityDomain::OpsStatus,
        "stock" | "crypto" => CapabilityDomain::MarketData,
        "rss_fetch" => CapabilityDomain::NewsContent,
        "image_vision" | "image_generate" | "image_edit" => CapabilityDomain::ImageMedia,
        "audio_transcribe" | "audio_synthesize" => CapabilityDomain::AudioMedia,
        "x" => CapabilityDomain::Publishing,
        "chat" => CapabilityDomain::GeneralChat,
        _ => infer_domain_from_skill_metadata(state, skill).unwrap_or(CapabilityDomain::System),
    }
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
    let canonical = entry.name.trim();
    if canonical.is_empty() {
        return None;
    }
    let lower = canonical.to_ascii_lowercase();
    match lower.as_str() {
        "stock" | "crypto" => Some(CapabilityDomain::MarketData),
        "rss_fetch" | "web_search_extract" | "browser_web" => Some(CapabilityDomain::NewsContent),
        "image_vision" | "image_generate" | "image_edit" => Some(CapabilityDomain::ImageMedia),
        "audio_transcribe" | "audio_synthesize" => Some(CapabilityDomain::AudioMedia),
        "x" => Some(CapabilityDomain::Publishing),
        "chat" => Some(CapabilityDomain::GeneralChat),
        "read_file" | "write_file" | "list_dir" | "make_dir" | "remove_file" | "fs_search" => {
            Some(CapabilityDomain::Filesystem)
        }
        "process_basic" | "docker_basic" | "health_check" | "log_analyze" | "service_control"
        | "task_control" | "config_guard" => Some(CapabilityDomain::OpsStatus),
        "run_cmd" | "system_basic" | "http_basic" | "git_basic" | "install_module"
        | "package_manager" | "archive_basic" | "db_basic" => Some(CapabilityDomain::System),
        _ => None,
    }
}

pub(crate) fn build_capability_map_for_task(state: &AppState, task: &ClaimedTask) -> String {
    let visible = state.planner_visible_skills_for_task(task);
    if visible.is_empty() {
        return "Current tool capabilities are unavailable; use chat only when no external retrieval or execution is needed.".to_string();
    }

    let mut grouped: BTreeMap<CapabilityDomain, BTreeSet<String>> = BTreeMap::new();
    for skill in &visible {
        grouped
            .entry(classify_skill(state, skill))
            .or_default()
            .insert(skill.clone());
    }

    let mut lines = vec![
        "Current capability map (derived from the currently enabled skills):".to_string(),
        "Use this as routing guidance, not as a full tool schema.".to_string(),
    ];

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
            if aliases.is_empty()
                && description.is_none()
                && semantic_tags.is_empty()
                && validation_actions.is_empty()
                && !entry.preferred_over_run_cmd
            {
                continue;
            }
            let mut parts = Vec::new();
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
