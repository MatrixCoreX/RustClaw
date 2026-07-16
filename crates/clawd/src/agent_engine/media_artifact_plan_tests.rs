use std::path::Path;

use serde_json::{json, Value};

use super::{strip_media_artifact_text_overwrites, AgentAction};

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
