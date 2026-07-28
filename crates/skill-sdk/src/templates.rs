use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::manifest::validate_safe_name;
use crate::{SkillSdkError, SkillSdkResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationLanguage {
    Rust,
    Python,
    Node,
    Go,
    Prebuilt,
}

impl ImplementationLanguage {
    pub fn parse(value: &str) -> SkillSdkResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "rust" | "cargo" => Ok(Self::Rust),
            "python" | "python3" => Ok(Self::Python),
            "node" | "javascript" | "typescript" | "js" | "ts" => Ok(Self::Node),
            "go" | "golang" => Ok(Self::Go),
            "prebuilt" | "native" => Ok(Self::Prebuilt),
            other => Err(SkillSdkError::new(
                "implementation_language_unsupported",
                format!("language={other}"),
            )),
        }
    }

    pub fn as_token(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Node => "node",
            Self::Go => "go",
            Self::Prebuilt => "prebuilt",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScaffoldRequest {
    pub destination: PathBuf,
    pub skill_name: String,
    pub capability_summary: String,
    pub actions: Vec<String>,
    pub implementation_language: ImplementationLanguage,
    /// Manifest source root relative to the workspace used by the installer.
    pub source_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaffoldOutcome {
    pub skill_name: String,
    pub implementation_language: ImplementationLanguage,
    pub manifest_path: PathBuf,
    pub written_files: Vec<PathBuf>,
}

pub fn scaffold_skill(request: &ScaffoldRequest) -> SkillSdkResult<ScaffoldOutcome> {
    validate_safe_name(&request.skill_name, "skill_name")?;
    validate_actions(&request.actions)?;
    if request.capability_summary.trim().is_empty() {
        return Err(SkillSdkError::new(
            "scaffold_summary_required",
            "capability_summary is empty",
        ));
    }
    if request.destination.exists()
        && fs::read_dir(&request.destination)?
            .next()
            .transpose()?
            .is_some()
    {
        return Err(SkillSdkError::new(
            "scaffold_destination_not_empty",
            request.destination.display().to_string(),
        ));
    }
    fs::create_dir_all(&request.destination)?;
    let package_name = format!("{}-skill", request.skill_name.replace('_', "-"));
    let actions = if request.actions.is_empty() {
        vec!["run".to_string()]
    } else {
        request.actions.clone()
    };
    let replacements = [
        ("__SKILL_NAME__", request.skill_name.as_str()),
        ("__PACKAGE_NAME__", package_name.as_str()),
        ("__SUMMARY__", request.capability_summary.trim()),
        ("__SOURCE_ROOT__", request.source_root.as_str()),
        ("__FIRST_ACTION__", actions[0].as_str()),
    ];
    let mut files = vec![
        ("README.md", README_TEMPLATE),
        ("INTERFACE.md", INTERFACE_TEMPLATE),
    ];
    files.extend(language_files(request.implementation_language));
    let mut written_files = Vec::new();
    for (relative, template) in files {
        let path = request.destination.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut content = template.to_string();
        for (from, to) in replacements {
            content = content.replace(from, to);
        }
        if relative == "INTERFACE.md" {
            content = content.replace("__ACTIONS__", &render_actions(&actions));
        }
        fs::write(&path, content)?;
        written_files.push(path);
    }
    Ok(ScaffoldOutcome {
        skill_name: request.skill_name.clone(),
        implementation_language: request.implementation_language,
        manifest_path: request.destination.join("skill.toml"),
        written_files,
    })
}

fn validate_actions(actions: &[String]) -> SkillSdkResult<()> {
    for action in actions {
        validate_safe_name(action, "action")?;
    }
    Ok(())
}

fn render_actions(actions: &[String]) -> String {
    actions
        .iter()
        .map(|action| {
            format!(
                "### `{action}`\n\n- Required args: none.\n- Optional args: action-specific values documented by the developer.\n"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn language_files(language: ImplementationLanguage) -> Vec<(&'static str, &'static str)> {
    match language {
        ImplementationLanguage::Rust => vec![
            ("skill.toml", include_str!("../templates/rust/skill.toml")),
            ("Cargo.toml", include_str!("../templates/rust/Cargo.toml")),
            ("Cargo.lock", include_str!("../templates/rust/Cargo.lock")),
            ("src/main.rs", include_str!("../templates/rust/src/main.rs")),
            (
                "tests/protocol.rs",
                include_str!("../templates/rust/tests/protocol.rs"),
            ),
        ],
        ImplementationLanguage::Python => vec![
            ("skill.toml", include_str!("../templates/python/skill.toml")),
            (
                "requirements.lock",
                "# Add hash-pinned dependencies only.\n",
            ),
            (
                "src/main.py",
                include_str!("../templates/python/src/main.py"),
            ),
            (
                "tests/test_protocol.py",
                include_str!("../templates/python/tests/test_protocol.py"),
            ),
        ],
        ImplementationLanguage::Node => vec![
            ("skill.toml", include_str!("../templates/node/skill.toml")),
            (
                "package.json",
                include_str!("../templates/node/package.json"),
            ),
            (
                "package-lock.json",
                include_str!("../templates/node/package-lock.json"),
            ),
            (
                "src/main.mjs",
                include_str!("../templates/node/src/main.mjs"),
            ),
            (
                "test/protocol.test.mjs",
                include_str!("../templates/node/test/protocol.test.mjs"),
            ),
        ],
        ImplementationLanguage::Go => vec![
            ("skill.toml", include_str!("../templates/go/skill.toml")),
            ("go.mod", include_str!("../templates/go/go.mod")),
            ("go.sum", ""),
            ("main.go", include_str!("../templates/go/main.go")),
            ("main_test.go", include_str!("../templates/go/main_test.go")),
        ],
        ImplementationLanguage::Prebuilt => vec![
            (
                "skill.toml",
                include_str!("../templates/prebuilt/skill.toml"),
            ),
            (
                "artifacts/README.md",
                include_str!("../templates/prebuilt/artifacts/README.md"),
            ),
        ],
    }
}

const README_TEMPLATE: &str = "# __SKILL_NAME__\n\n__SUMMARY__\n\nThis package uses `rustclaw-jsonl-v1`. Run `rustclaw-skill validate skill.toml` before installation.\n";

const INTERFACE_TEMPLATE: &str = "# __SKILL_NAME__ Interface\n\n## Capability Summary\n\n__SUMMARY__\n\n## Actions\n\n__ACTIONS__\n## Error Contract\n\nErrors return `status=error`, readable `error_text`, and stable `extra.error_code` / `extra.message_key`.\n\n## Config Entry Points\n\nNo configuration is required by the starter template.\n";

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
