use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
                "check service/process/task status, read logs, run health checks, and inspect safe config state"
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
            CapabilityDomain::AudioMedia => {
                "transcribe audio and synthesize spoken output"
            }
            CapabilityDomain::Publishing => {
                "draft or send outbound social content"
            }
            CapabilityDomain::GeneralChat => {
                "provide conversational explanation, rewriting, and smalltalk when external retrieval is not needed"
            }
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
        "process_basic" | "docker_basic" | "health_check" | "log_analyze"
        | "service_control" | "task_control" | "config_guard" => CapabilityDomain::OpsStatus,
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
    let mut snippets = vec![skill.to_string()];
    if let Some(registry) = state.get_skills_registry() {
        if let Some(entry) = registry.get(skill) {
            if !entry.aliases.is_empty() {
                snippets.push(entry.aliases.join(" "));
            }
            if let Some(prompt_text) = load_skill_prompt_text(state, entry) {
                snippets.push(prompt_text);
            }
        }
    }

    let haystack = snippets.join("\n").to_lowercase();
    infer_domain_from_text(&haystack)
}

fn load_skill_prompt_text(state: &AppState, entry: &SkillRegistryEntry) -> Option<String> {
    let prompt_file = entry.prompt_file.trim();
    if prompt_file.is_empty() {
        return None;
    }
    let prompt_path = resolve_prompt_path(&state.workspace_root, prompt_file)?;
    if !prompt_path.starts_with(&state.workspace_root) {
        return None;
    }
    std::fs::read_to_string(prompt_path).ok()
}

fn resolve_prompt_path(workspace_root: &Path, prompt_file: &str) -> Option<PathBuf> {
    let path = Path::new(prompt_file);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        Some(workspace_root.join(path))
    }
}

fn infer_domain_from_text(haystack: &str) -> Option<CapabilityDomain> {
    let matches_any = |terms: &[&str]| terms.iter().any(|term| haystack.contains(term));

    if matches_any(&[
        "a股",
        "股票",
        "stock",
        "crypto",
        "coin",
        "token",
        "market",
        "quote",
        "行情",
        "个股",
        "板块",
        "k线",
        "candles",
        "portfolio",
        "position",
        "order status",
        "交易策略",
        "trading-related",
    ]) {
        return Some(CapabilityDomain::MarketData);
    }

    if matches_any(&[
        "rss",
        "news",
        "headline",
        "feed",
        "web content",
        "资讯",
        "新闻",
        "网页",
    ]) {
        return Some(CapabilityDomain::NewsContent);
    }

    if matches_any(&[
        "image",
        "ocr",
        "vision",
        "photo",
        "图片",
        "图像",
        "视觉",
        "生成图片",
    ]) {
        return Some(CapabilityDomain::ImageMedia);
    }

    if matches_any(&[
        "audio",
        "speech",
        "voice",
        "tts",
        "transcribe",
        "音频",
        "语音",
        "转写",
        "朗读",
    ]) {
        return Some(CapabilityDomain::AudioMedia);
    }

    if matches_any(&[
        "twitter",
        "x.com",
        "tweet",
        "publish",
        "post to x",
        "社交发布",
        "发帖",
        "推文",
    ]) {
        return Some(CapabilityDomain::Publishing);
    }

    if matches_any(&[
        "log",
        "service",
        "process",
        "docker",
        "health",
        "task status",
        "配置",
        "进程",
        "服务状态",
        "日志",
    ]) {
        return Some(CapabilityDomain::OpsStatus);
    }

    if matches_any(&[
        "file",
        "directory",
        "filesystem",
        "path",
        "read_file",
        "write_file",
        "目录",
        "文件",
        "路径",
    ]) {
        return Some(CapabilityDomain::Filesystem);
    }

    if matches_any(&[
        "chat",
        "smalltalk",
        "conversation",
        "rewrite",
        "summarize",
        "闲聊",
        "改写",
        "对话",
    ]) {
        return Some(CapabilityDomain::GeneralChat);
    }

    if matches_any(&[
        "shell",
        "command",
        "http",
        "database",
        "sql",
        "git",
        "archive",
        "package",
        "system",
        "命令",
        "系统",
        "数据库",
        "压缩",
    ]) {
        return Some(CapabilityDomain::System);
    }

    None
}

pub(crate) fn build_capability_map_for_task(state: &AppState, task: &ClaimedTask) -> String {
    let visible = state.planner_visible_skills_for_task(task);
    if visible.is_empty() {
        return "Current tool capabilities are unavailable; use chat only when no external retrieval or execution is needed.".to_string();
    }

    let mut grouped: BTreeMap<CapabilityDomain, BTreeSet<String>> = BTreeMap::new();
    for skill in visible {
        grouped
            .entry(classify_skill(state, &skill))
            .or_default()
            .insert(skill);
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
