use std::env;
use std::path::Path;

use claw_core::skill_registry::{SkillManifest, SkillRegistryEntry};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkillRuntimeAvailability {
    pub(crate) current_os: String,
    pub(crate) unsupported_os: Option<Vec<String>>,
    pub(crate) missing_required_bins: Vec<String>,
    pub(crate) missing_optional_bins: Vec<String>,
}

impl SkillRuntimeAvailability {
    pub(crate) fn is_available(&self) -> bool {
        self.unsupported_os.is_none() && self.missing_required_bins.is_empty()
    }
}

pub(crate) fn evaluate_entry_availability(entry: &SkillRegistryEntry) -> SkillRuntimeAvailability {
    evaluate_runtime_availability(
        &entry.supported_os,
        &entry.required_bins,
        &entry.optional_bins,
    )
}

pub(crate) fn evaluate_manifest_availability(manifest: &SkillManifest) -> SkillRuntimeAvailability {
    evaluate_runtime_availability(
        &manifest.supported_os,
        &manifest.required_bins,
        &manifest.optional_bins,
    )
}

pub(crate) fn availability_metadata_parts(availability: &SkillRuntimeAvailability) -> Vec<String> {
    let mut parts = Vec::new();
    if availability.is_available() {
        parts.push("runtime_availability: available".to_string());
    } else {
        parts.push("runtime_availability: unavailable".to_string());
        if let Some(supported_os) = &availability.unsupported_os {
            parts.push(format!(
                "unsupported_os: current={} supported={}",
                availability.current_os,
                supported_os.join(", ")
            ));
        }
        if !availability.missing_required_bins.is_empty() {
            parts.push(format!(
                "missing_required_bins: {}",
                availability.missing_required_bins.join(", ")
            ));
        }
    }
    if !availability.missing_optional_bins.is_empty() {
        parts.push(format!(
            "missing_optional_bins: {}",
            availability.missing_optional_bins.join(", ")
        ));
    }
    parts
}

fn evaluate_runtime_availability(
    supported_os: &[String],
    required_bins: &[String],
    optional_bins: &[String],
) -> SkillRuntimeAvailability {
    let current_os = current_os_token();
    let supported_os_values = clean_values(supported_os);
    let normalized_supported_os = supported_os_values
        .iter()
        .filter_map(|os| normalize_os_token(os))
        .collect::<Vec<_>>();
    let unsupported_os = if normalized_supported_os.is_empty()
        || normalized_supported_os
            .iter()
            .any(|os| os == "any" || os == &current_os)
    {
        None
    } else {
        Some(supported_os_values)
    };
    let missing_required_bins = clean_values(required_bins)
        .into_iter()
        .filter(|bin| !bin_available(bin))
        .collect::<Vec<_>>();
    let missing_optional_bins = clean_values(optional_bins)
        .into_iter()
        .filter(|bin| !bin_available(bin))
        .collect::<Vec<_>>();
    SkillRuntimeAvailability {
        current_os,
        unsupported_os,
        missing_required_bins,
        missing_optional_bins,
    }
}

fn current_os_token() -> String {
    normalize_os_token(env::consts::OS).unwrap_or_else(|| env::consts::OS.to_ascii_lowercase())
}

fn normalize_os_token(raw: &str) -> Option<String> {
    let token = raw.trim().to_ascii_lowercase();
    if token.is_empty() {
        return None;
    }
    let normalized = token.replace(['_', ' '], "-");
    let os = match normalized.as_str() {
        "*" | "all" | "any" | "unix" => "any",
        "mac" | "macos" | "mac-os" | "osx" | "darwin" => "macos",
        "linux" | "gnu-linux" | "debian" | "ubuntu" | "raspbian" | "raspberry-pi-os" | "raspi" => {
            "linux"
        }
        "windows" | "win32" | "win64" => "windows",
        other => other,
    };
    Some(os.to_string())
}

fn clean_values(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || out.iter().any(|seen: &String| seen.as_str() == trimmed) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

fn bin_available(bin: &str) -> bool {
    let candidate = Path::new(bin);
    if candidate.components().count() > 1 {
        return is_executable_file(candidate);
    }
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| is_executable_file(&dir.join(bin)))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_core::skill_registry::SkillsRegistry;

    fn registry_entry_from(toml: &str, name: &str) -> SkillRegistryEntry {
        let path = std::env::temp_dir().join(format!("skill_availability_{name}.toml"));
        std::fs::write(&path, toml).unwrap();
        let registry = SkillsRegistry::load_from_path(&path).unwrap();
        let entry = registry.get(name).unwrap().clone();
        let _ = std::fs::remove_file(path);
        entry
    }

    #[test]
    fn missing_required_bin_marks_skill_unavailable() {
        let entry = registry_entry_from(
            r#"
[[skills]]
name = "needs_missing_bin"
enabled = true
supported_os = ["linux", "macos"]
required_bins = ["definitely_missing_rustclaw_test_bin_20260511"]
"#,
            "needs_missing_bin",
        );
        let availability = evaluate_entry_availability(&entry);
        assert!(!availability.is_available());
        assert_eq!(
            availability.missing_required_bins,
            vec!["definitely_missing_rustclaw_test_bin_20260511"]
        );
    }

    #[test]
    fn unsupported_os_marks_skill_unavailable() {
        let entry = registry_entry_from(
            r#"
[[skills]]
name = "wrong_os"
enabled = true
supported_os = ["definitely-not-this-os"]
"#,
            "wrong_os",
        );
        let availability = evaluate_entry_availability(&entry);
        assert!(!availability.is_available());
        assert_eq!(
            availability.unsupported_os,
            Some(vec!["definitely-not-this-os".to_string()])
        );
    }
}
