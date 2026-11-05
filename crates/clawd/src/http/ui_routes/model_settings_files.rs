#[derive(Debug)]
struct ModelSettingsFailure {
    file: &'static str,
    error_code: &'static str,
}

impl std::fmt::Display for ModelSettingsFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.error_code)
    }
}

impl std::error::Error for ModelSettingsFailure {}

fn model_settings_failure(file: &'static str, error_code: &'static str) -> anyhow::Error {
    anyhow::Error::new(ModelSettingsFailure { file, error_code })
}

struct ModelSettingsFile {
    name: &'static str,
    original: Option<Vec<u8>>,
    value: toml::Value,
    changed: bool,
}

fn load_model_settings_files(
    state: &AppState,
    require_main: bool,
) -> anyhow::Result<[ModelSettingsFile; 5]> {
    let mut files = Vec::new();
    for (filename, path) in [
        "config.toml",
        "image.toml",
        "audio.toml",
        "video.toml",
        "music.toml",
    ]
    .into_iter()
    .zip(model_settings_paths(state))
    {
        let original = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && !(require_main && filename == "config.toml") =>
            {
                None
            }
            Err(_) => {
                return Err(model_settings_failure(
                    filename,
                    "model_settings_read_failed",
                ))
            }
        };
        // Missing optional files retain first-use defaults. An unreadable or
        // malformed existing file is never projected as an empty configuration.
        let value = match original.as_deref() {
            None => toml::Value::Table(toml::map::Map::new()),
            Some(raw) => {
                let raw = std::str::from_utf8(raw)
                    .map_err(|_| model_settings_failure(filename, "model_settings_parse_failed"))?;
                toml::from_str(raw)
                    .map_err(|_| model_settings_failure(filename, "model_settings_parse_failed"))?
            }
        };
        files.push(ModelSettingsFile {
            name: filename,
            original,
            value,
            changed: false,
        });
    }
    files
        .try_into()
        .map_err(|_| anyhow::anyhow!("model_settings_file_inventory_invalid"))
}

#[cfg(test)]
struct ModelSettingsEdit {
    name: &'static str,
    original: Option<Vec<u8>>,
    updated: Vec<u8>,
}

fn model_settings_paths(state: &AppState) -> [PathBuf; 5] {
    let main = active_runtime_config_path(state);
    let parent = main.parent().expect("active configuration has a parent");
    [
        main.clone(),
        parent.join("image.toml"),
        parent.join("audio.toml"),
        parent.join("video.toml"),
        parent.join("music.toml"),
    ]
}

fn model_settings_lock_paths(state: &AppState) -> Vec<PathBuf> {
    let mut paths = model_settings_paths(state).to_vec();
    paths.extend(crate::configuration_files::persistence_path(
        &state.skill_rt.workspace_root,
    ));
    paths.push(claw_core::secrets::model_environment::path(
        &state.skill_rt.workspace_root,
    ));
    paths
}

fn save_model_settings_files(
    state: &AppState,
    files: [ModelSettingsFile; 5],
) -> anyhow::Result<()> {
    save_model_settings_files_with_persistence(
        state,
        files,
        crate::configuration_files::persistence_path(&state.skill_rt.workspace_root),
    )
}

fn save_model_settings_files_with_persistence(
    state: &AppState,
    files: [ModelSettingsFile; 5],
    persistence: Option<PathBuf>,
) -> anyhow::Result<()> {
    // Finish all parsing, structural updates and serialization before touching
    // any file. Validation failures therefore cannot leave a partial save.
    let root = &state.skill_rt.workspace_root;
    let paths = model_settings_paths(state);
    let mut changes = Vec::new();
    let mut main_updated = None;
    for (index, (file, path)) in files.into_iter().zip(paths).enumerate() {
        if !file.changed {
            continue;
        }
        let updated = toml::to_string_pretty(&file.value)
            .map_err(|_| model_settings_failure(file.name, "model_settings_serialize_failed"))?
            .into_bytes();
        if index == 0 {
            main_updated = Some(updated.clone());
        }
        let target = crate::configuration_files::Target::configured(root, path)
            .map_err(|_| model_settings_failure(file.name, "model_settings_write_failed"))?;
        changes.push(crate::configuration_files::Edit {
            target,
            original: file.original,
            updated,
        });
    }
    if let Some(updated) = main_updated {
        if let Some(path) = persistence {
            let target =
                crate::configuration_files::Target::configured(root, path).map_err(|_| {
                    model_settings_failure("config.toml", "model_settings_write_failed")
                })?;
            let original = target.read().map_err(|_| {
                model_settings_failure("config.toml", "model_settings_write_failed")
            })?;
            if !target
                .same_destination(&changes[0].target)
                .map_err(|_| model_settings_failure("config.toml", "model_settings_write_failed"))?
            {
                changes.push(crate::configuration_files::Edit {
                    target,
                    original,
                    updated,
                });
            }
        }
    }
    commit_model_settings_changes(&changes, |target, bytes| target.publish(bytes))
}

#[cfg(test)]
fn commit_model_settings_edits(
    root: &Path,
    edits: &[ModelSettingsEdit],
    mut publish: impl FnMut(&Path, &[u8]) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    let changes = edits
        .iter()
        .map(|edit| crate::configuration_files::Edit {
            target: crate::configuration_files::Target::workspace(
                root,
                root.join("configs").join(edit.name),
            ),
            original: edit.original.clone(),
            updated: edit.updated.clone(),
        })
        .collect::<Vec<_>>();
    commit_model_settings_changes(&changes, |target, bytes| publish(&target.path, bytes))
}

fn commit_model_settings_changes(
    changes: &[crate::configuration_files::Edit],
    publish: impl FnMut(&crate::configuration_files::Target, &[u8]) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    use crate::configuration_files::{commit_with, FailureKind};
    commit_with(changes, publish).map_err(|failure| {
        model_settings_failure(
            "configuration",
            match failure.kind {
                FailureKind::Changed => "model_settings_changed_during_save",
                FailureKind::Write => "model_settings_write_failed",
                FailureKind::Recovery => "model_settings_rollback_failed",
            },
        )
    })
}

fn update_model_provider_key(
    provider: &mut toml::map::Map<String, toml::Value>,
    key: &str,
) -> bool {
    if key.is_empty()
        || provider
            .get("api_key")
            .and_then(toml::Value::as_str)
            .is_some_and(|current| current == key || mask_secret(current) == key)
    {
        return false;
    }
    provider.insert("api_key".to_string(), toml::Value::String(key.to_string()));
    true
}
