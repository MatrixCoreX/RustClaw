use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use tracing::info;

use crate::AgentAction;

const MEDIA_ARTIFACT_SKILLS: &[&str] = &[
    "audio_synthesize",
    "image_generate",
    "image_edit",
    "video_generate",
    "music_generate",
];

pub(super) fn strip_media_artifact_text_overwrites(
    workspace_root: &Path,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut protected_paths = HashSet::new();
    let mut rewritten = Vec::with_capacity(actions.len());
    let mut removed = 0usize;

    for action in actions {
        if should_strip_text_io_over_media_output(workspace_root, &protected_paths, &action) {
            removed += 1;
            continue;
        }
        collect_media_output_paths(workspace_root, &action, &mut protected_paths);
        rewritten.push(action);
    }

    if removed > 0 {
        info!("plan_strip_media_artifact_text_io removed={removed}");
    }
    rewritten
}

fn should_strip_text_io_over_media_output(
    workspace_root: &Path,
    protected_paths: &HashSet<String>,
    action: &AgentAction,
) -> bool {
    let Some((skill, args)) = tool_or_skill_args(action) else {
        return false;
    };
    let Some(path) = media_artifact_text_io_path(skill, args) else {
        return false;
    };
    if crate::media_artifact_paths::is_media_artifact_path(path) {
        return true;
    }
    normalized_path_key(workspace_root, path).is_some_and(|path| protected_paths.contains(&path))
}

fn media_artifact_text_io_path<'a>(skill: &str, args: &'a Value) -> Option<&'a str> {
    let path = action_path_arg(args)?;
    if skill.eq_ignore_ascii_case("fs_basic") {
        let action_name = args.get("action").and_then(Value::as_str)?;
        if media_artifact_fs_basic_action_is_text_io(action_name) {
            return Some(path);
        }
        return None;
    }
    if ["read_file", "read_text", "write_file", "append_file"]
        .iter()
        .any(|candidate| skill.eq_ignore_ascii_case(candidate))
    {
        return Some(path);
    }
    None
}

fn media_artifact_fs_basic_action_is_text_io(action_name: &str) -> bool {
    matches!(
        action_name.trim(),
        "append_text" | "read_file" | "read_range" | "read_text" | "read_text_range" | "write_text"
    )
}

fn action_path_arg(args: &Value) -> Option<&str> {
    ["path", "file_path", "target_path"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
}

fn collect_media_output_paths(
    workspace_root: &Path,
    action: &AgentAction,
    protected_paths: &mut HashSet<String>,
) {
    let Some((skill, args)) = tool_or_skill_args(action) else {
        return;
    };
    if !MEDIA_ARTIFACT_SKILLS
        .iter()
        .any(|candidate| skill.eq_ignore_ascii_case(candidate))
    {
        return;
    }
    for key in ["output_path", "path"] {
        if let Some(path) = args.get(key).and_then(Value::as_str) {
            if let Some(path) = normalized_path_key(workspace_root, path) {
                protected_paths.insert(path);
            }
        }
    }
}

fn tool_or_skill_args(action: &AgentAction) -> Option<(&str, &Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => None,
    }
}

fn normalized_path_key(workspace_root: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains("://") {
        return None;
    }
    let path = Path::new(raw);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    Some(normalize_pathbuf(joined).to_string_lossy().to_string())
}

fn normalize_pathbuf(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn strips_text_io_that_would_touch_audio_artifact() {
        let workspace_root = Path::new("/workspace");
        let actions = vec![
            AgentAction::CallSkill {
                skill: "audio_synthesize".to_string(),
                args: json!({
                    "text": "smoke",
                    "output_path": "document/skill_audio_smoke.mp3"
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "read_text_range",
                    "path": "/workspace/document/skill_audio_smoke.mp3",
                    "mode": "head",
                    "n": 20
                }),
            },
            AgentAction::Respond {
                content: "document/skill_audio_smoke.mp3".to_string(),
            },
        ];

        let rewritten = strip_media_artifact_text_overwrites(workspace_root, actions);
        assert_eq!(rewritten.len(), 2);
        assert!(!rewritten.iter().any(|action| matches!(
            action,
            AgentAction::CallTool { tool, args }
                if tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("read_text_range")
        )));
    }

    #[test]
    fn strips_text_io_to_media_extension_even_without_prior_media_step() {
        let workspace_root = Path::new("/workspace");
        let actions = vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "write_text",
                    "path": "document/skill_audio_smoke.mp3",
                    "content": "placeholder"
                }),
            },
            AgentAction::CallSkill {
                skill: "audio_synthesize".to_string(),
                args: json!({
                    "text": "smoke",
                    "output_path": "document/skill_audio_smoke.mp3"
                }),
            },
        ];

        let rewritten = strip_media_artifact_text_overwrites(workspace_root, actions);
        assert_eq!(rewritten.len(), 1);
        assert!(matches!(rewritten[0], AgentAction::CallSkill { .. }));
    }

    #[test]
    fn strips_legacy_read_file_to_media_artifact() {
        let workspace_root = Path::new("/workspace");
        let actions = vec![
            AgentAction::CallSkill {
                skill: "image_edit".to_string(),
                args: json!({
                    "image_url": "https://example.test/rust.png",
                    "output_path": "document/rust_icon_pixel_smoke.png"
                }),
            },
            AgentAction::CallSkill {
                skill: "read_file".to_string(),
                args: json!({
                    "path": "/workspace/document/rust_icon_pixel_smoke.png"
                }),
            },
            AgentAction::Respond {
                content: "document/rust_icon_pixel_smoke.png".to_string(),
            },
        ];

        let rewritten = strip_media_artifact_text_overwrites(workspace_root, actions);
        assert_eq!(rewritten.len(), 2);
        assert!(!rewritten.iter().any(
            |action| matches!(action, AgentAction::CallSkill { skill, .. } if skill == "read_file")
        ));
    }

    #[test]
    fn keeps_metadata_checks_for_media_artifacts() {
        let workspace_root = Path::new("/workspace");
        let actions = vec![
            AgentAction::CallSkill {
                skill: "image_generate".to_string(),
                args: json!({
                    "prompt": "smoke",
                    "output_path": "document/skill_generate_smoke.png"
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "stat_paths",
                    "paths": ["document/skill_generate_smoke.png"]
                }),
            },
        ];

        let rewritten = strip_media_artifact_text_overwrites(workspace_root, actions);
        assert_eq!(rewritten.len(), 2);
        assert!(matches!(
            &rewritten[1],
            AgentAction::CallTool { tool, args }
                if tool == "fs_basic"
                    && args.get("action").and_then(Value::as_str) == Some("stat_paths")
        ));
    }
}
