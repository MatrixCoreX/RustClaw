use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

const DEFAULT_CHANNEL_COMMANDS_TOML: &str = include_str!("../../../configs/channel_commands.toml");

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCommandKind {
    Core,
    Skill,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreCommandAction {
    Start,
    Ask,
    BindKey,
    AgentMode,
    Status,
    Cancel,
    Skills,
    RunSkill,
    SendFile,
    VoiceMode,
    RustclawConfig,
    CryptoApi,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelCommandDef {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub kind: ChannelCommandKind,
    #[serde(default)]
    pub core_action: Option<CoreCommandAction>,
    #[serde(default)]
    pub skill_name: Option<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub menu_channels: Vec<String>,
    #[serde(default)]
    pub description_key: Option<String>,
    #[serde(default)]
    pub allow_unbound: bool,
    #[serde(default)]
    pub admin_only: bool,
    #[serde(default)]
    pub order: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ChannelCommandCatalogFile {
    #[serde(default)]
    commands: Vec<ChannelCommandDef>,
}

#[derive(Debug, Clone)]
pub struct ChannelCommandCatalog {
    commands: Vec<ChannelCommandDef>,
}

#[derive(Debug, Clone)]
pub struct ChannelCommandMatch {
    pub definition: ChannelCommandDef,
    pub tail: String,
    pub raw_name: String,
}

impl ChannelCommandMatch {
    pub fn invoked_name_matches(&self, raw_name: &str) -> bool {
        normalize_command_name(&self.raw_name) == normalize_command_name(raw_name)
    }
}

impl ChannelCommandDef {
    pub fn core_action(&self) -> Option<CoreCommandAction> {
        self.core_action
    }

    pub fn skill_name(&self) -> Option<&str> {
        self.skill_name.as_deref()
    }

    pub fn description_key(&self) -> Option<&str> {
        self.description_key.as_deref()
    }

    pub fn matches_name(&self, raw_name: &str) -> bool {
        let normalized = normalize_command_name(raw_name);
        if normalized.is_empty() {
            return false;
        }
        normalize_command_name(&self.name) == normalized
            || self
                .aliases
                .iter()
                .any(|alias| normalize_command_name(alias) == normalized)
    }

    pub fn supports_channel(&self, channel: &str) -> bool {
        supports_channel(&self.channels, channel)
    }

    pub fn menu_visible_for_channel(&self, channel: &str) -> bool {
        supports_channel(&self.menu_channels, channel)
    }
}

impl Default for ChannelCommandCatalog {
    fn default() -> Self {
        Self::from_toml_str(DEFAULT_CHANNEL_COMMANDS_TOML).unwrap_or_else(|_| Self {
            commands: Vec::new(),
        })
    }
}

impl ChannelCommandCatalog {
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| format!("read channel command config failed: {err}"))?;
        Self::from_toml_str(&raw)
    }

    pub fn load_or_default(path: &Path) -> Self {
        match Self::load_from_path(path) {
            Ok(catalog) => catalog,
            Err(err) => {
                tracing::warn!(
                    "channel command catalog fallback to default: path={} err={}",
                    path.display(),
                    err
                );
                Self::default()
            }
        }
    }

    pub fn from_toml_str(raw: &str) -> Result<Self, String> {
        let parsed: ChannelCommandCatalogFile = toml::from_str(raw)
            .map_err(|err| format!("parse channel command config failed: {err}"))?;
        validate_commands(&parsed.commands)?;
        Ok(Self {
            commands: parsed.commands,
        })
    }

    pub fn commands(&self) -> &[ChannelCommandDef] {
        &self.commands
    }

    pub fn menu_commands_for_channel(&self, channel: &str) -> Vec<&ChannelCommandDef> {
        let mut commands = self
            .commands
            .iter()
            .filter(|command| command.menu_visible_for_channel(channel))
            .collect::<Vec<_>>();
        commands.sort_by_key(|command| command.order);
        commands
    }

    pub fn match_command(&self, text: &str, channel: &str) -> Option<ChannelCommandMatch> {
        let trimmed = text.trim_start();
        if !trimmed.starts_with('/') {
            return None;
        }
        let without_slash = &trimmed[1..];
        let mut parts = without_slash.splitn(2, char::is_whitespace);
        let raw_name = parts.next().unwrap_or_default().trim();
        if raw_name.is_empty() {
            return None;
        }
        let tail = parts.next().unwrap_or_default().trim().to_string();
        self.commands
            .iter()
            .find(|command| command.supports_channel(channel) && command.matches_name(raw_name))
            .cloned()
            .map(|definition| ChannelCommandMatch {
                definition,
                tail,
                raw_name: raw_name.to_string(),
            })
    }

    pub fn allows_unbound_command(&self, text: &str, channel: &str) -> bool {
        self.match_command(text, channel)
            .map(|command| command.definition.allow_unbound)
            .unwrap_or(false)
    }
}

fn supports_channel(channels: &[String], channel: &str) -> bool {
    if channels.is_empty() {
        return false;
    }
    let normalized_channel = channel.trim().to_ascii_lowercase();
    channels.iter().any(|candidate| {
        let normalized = candidate.trim().to_ascii_lowercase();
        normalized == "*" || normalized == "all" || normalized == normalized_channel
    })
}

fn channels_overlap(left: &[String], right: &[String]) -> bool {
    if left.iter().any(|channel| is_wildcard_channel(channel))
        || right.iter().any(|channel| is_wildcard_channel(channel))
    {
        return true;
    }
    let left_channels = left
        .iter()
        .map(|channel| normalize_channel_name(channel))
        .collect::<HashSet<_>>();
    right
        .iter()
        .map(|channel| normalize_channel_name(channel))
        .any(|channel| left_channels.contains(&channel))
}

fn normalize_channel_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn is_wildcard_channel(raw: &str) -> bool {
    matches!(normalize_channel_name(raw).as_str(), "*" | "all")
}

fn normalize_command_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn normalized_command_names(command: &ChannelCommandDef) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for raw_name in
        std::iter::once(command.name.as_str()).chain(command.aliases.iter().map(String::as_str))
    {
        let normalized = normalize_command_name(raw_name);
        if normalized.is_empty() {
            return Err(format!(
                "channel command `{}` has an empty name or alias",
                command.name
            ));
        }
        if !seen.insert(normalized.clone()) {
            return Err(format!(
                "channel command `{}` has duplicate name or alias `{}`",
                command.name, raw_name
            ));
        }
        names.push(normalized);
    }
    Ok(names)
}

fn validate_commands(commands: &[ChannelCommandDef]) -> Result<(), String> {
    if commands.is_empty() {
        return Err("channel command config has no commands".to_string());
    }
    let mut normalized_names_by_command = Vec::with_capacity(commands.len());
    for command in commands {
        let normalized_names = normalized_command_names(command)?;
        if command.channels.is_empty() {
            return Err(format!(
                "channel command `{}` must declare at least one channel",
                command.name
            ));
        }
        for menu_channel in &command.menu_channels {
            if !supports_channel(&command.channels, menu_channel) {
                return Err(format!(
                    "channel command `{}` declares menu channel `{}` outside supported channels",
                    command.name, menu_channel
                ));
            }
        }
        match command.kind {
            ChannelCommandKind::Core => {
                if command.core_action.is_none() {
                    return Err(format!(
                        "core channel command `{}` is missing core_action",
                        command.name
                    ));
                }
            }
            ChannelCommandKind::Skill => {
                if command
                    .skill_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return Err(format!(
                        "skill channel command `{}` is missing skill_name",
                        command.name
                    ));
                }
            }
        }
        normalized_names_by_command.push(normalized_names);
    }

    for (idx, command) in commands.iter().enumerate() {
        for (other_idx, other) in commands.iter().enumerate().skip(idx + 1) {
            if !channels_overlap(&command.channels, &other.channels) {
                continue;
            }
            let other_names = normalized_names_by_command[other_idx]
                .iter()
                .collect::<HashSet<_>>();
            if let Some(duplicate_name) = normalized_names_by_command[idx]
                .iter()
                .find(|name| other_names.contains(name))
            {
                return Err(format!(
                    "channel commands `{}` and `{}` both bind `{}` on overlapping channels",
                    command.name, other.name, duplicate_name
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ChannelCommandCatalog, CoreCommandAction};

    const SAMPLE: &str = r#"
[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["telegram", "whatsapp"]
menu_channels = ["telegram"]
allow_unbound = true
order = 10

[[commands]]
name = "crypto"
kind = "skill"
skill_name = "crypto"
channels = ["telegram"]
menu_channels = ["telegram"]
order = 20
"#;

    #[test]
    fn match_command_supports_bot_suffix_and_tail() {
        let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");
        let matched = catalog
            .match_command("/start@demo_bot hello", "telegram")
            .expect("match command");
        assert_eq!(
            matched.definition.core_action(),
            Some(CoreCommandAction::Start)
        );
        assert_eq!(matched.tail, "hello");
    }

    #[test]
    fn allows_unbound_command_follows_catalog() {
        let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");
        assert!(catalog.allows_unbound_command("/start", "telegram"));
        assert!(!catalog.allows_unbound_command("/crypto price btc", "telegram"));
    }

    #[test]
    fn menu_commands_filter_by_channel() {
        let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");
        let telegram = catalog.menu_commands_for_channel("telegram");
        assert_eq!(telegram.len(), 2);
        let whatsapp = catalog.menu_commands_for_channel("whatsapp");
        assert!(whatsapp.is_empty());
    }

    #[test]
    fn duplicate_alias_on_overlapping_channels_is_rejected() {
        let raw = r#"
[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["telegram"]

[[commands]]
name = "begin"
aliases = ["start"]
kind = "core"
core_action = "cancel"
channels = ["telegram", "whatsapp"]
"#;

        let err = ChannelCommandCatalog::from_toml_str(raw).expect_err("duplicate should fail");
        assert!(err.contains("both bind `start`"));
    }

    #[test]
    fn menu_channel_must_be_supported_by_command_channel_set() {
        let raw = r#"
[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["whatsapp"]
menu_channels = ["telegram"]
"#;

        let err = ChannelCommandCatalog::from_toml_str(raw).expect_err("menu channel should fail");
        assert!(err.contains("menu channel `telegram` outside supported channels"));
    }

    #[test]
    fn slash_prefixed_paths_and_non_whitespace_suffixes_are_not_commands() {
        let raw = r#"
[[commands]]
name = "run"
kind = "core"
core_action = "run_skill"
channels = ["telegram", "whatsapp"]

[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["telegram", "whatsapp"]
"#;

        let catalog = ChannelCommandCatalog::from_toml_str(raw).expect("parse catalog");
        assert!(catalog
            .match_command("/home/testuser/project", "telegram")
            .is_none());
        assert!(catalog.match_command("/run/logs", "telegram").is_none());
        assert!(catalog.match_command("/start/docs", "telegram").is_none());
        assert!(catalog.match_command("/run logs", "telegram").is_some());
    }
}
