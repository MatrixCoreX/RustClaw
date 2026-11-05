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
    root: &Path,
    require_main: bool,
) -> anyhow::Result<[ModelSettingsFile; 5]> {
    let mut files = Vec::new();
    for filename in [
        "config.toml",
        "image.toml",
        "audio.toml",
        "video.toml",
        "music.toml",
    ] {
        let original = match fs::read(root.join("configs").join(filename)) {
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

struct ModelSettingsEdit {
    name: &'static str,
    original: Option<Vec<u8>>,
    updated: Vec<u8>,
}

fn save_model_settings_files(root: &Path, files: [ModelSettingsFile; 5]) -> anyhow::Result<()> {
    // Finish all parsing, structural updates and serialization before touching
    // any file. Validation failures therefore cannot leave a partial save.
    let edits = files
        .into_iter()
        .filter(|file| file.changed)
        .map(|file| {
            let updated = toml::to_string_pretty(&file.value).map_err(|_| {
                model_settings_failure(file.name, "model_settings_serialize_failed")
            })?;
            Ok(ModelSettingsEdit {
                name: file.name,
                original: file.original,
                updated: updated.into_bytes(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    commit_model_settings_edits(root, &edits, |path, bytes| {
        crate::secure_workspace_fs::atomic_write_workspace_file(root, path, bytes)
    })
}

fn commit_model_settings_edits(
    root: &Path,
    edits: &[ModelSettingsEdit],
    mut publish: impl FnMut(&Path, &[u8]) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    // Detect edits made since this request read its inputs. The HTTP service
    // serializes this family of requests; out-of-band editors are not locked.
    for edit in edits {
        let current = read_model_settings_bytes(root, edit.name)?;
        if current != edit.original {
            return Err(model_settings_failure(
                edit.name,
                "model_settings_changed_during_save",
            ));
        }
    }
    for (index, edit) in edits.iter().enumerate() {
        let path = root.join("configs").join(edit.name);
        if publish(&path, &edit.updated).is_err() {
            // Atomic publication can report a directory-sync error after its
            // rename. Include that file when restoring, not just earlier ones.
            let mut restored = true;
            for previous in edits[..=index].iter().rev() {
                let current = read_model_settings_bytes(root, previous.name);
                if matches!(current, Ok(ref bytes) if *bytes == previous.original) {
                    continue;
                }
                let path = root.join("configs").join(previous.name);
                let result = if let Some(original) = previous.original.as_deref() {
                    publish(&path, original)
                } else {
                    crate::secure_workspace_fs::remove_workspace_file(root, &path)
                };
                restored &= result.is_ok()
                    || previous.original.is_none()
                        && result
                            .as_ref()
                            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
            }
            return Err(model_settings_failure(
                edit.name,
                if restored {
                    "model_settings_write_failed"
                } else {
                    "model_settings_rollback_failed"
                },
            ));
        }
    }
    // Each file is atomically replaced. This is not a crash-safe multi-file
    // transaction. Other configuration endpoints and out-of-process writers
    // do not share this request family's lock.
    Ok(())
}

fn read_model_settings_bytes(root: &Path, name: &'static str) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(root.join("configs").join(name)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(model_settings_failure(name, "model_settings_read_failed")),
    }
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
