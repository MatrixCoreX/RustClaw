#[derive(Debug, Clone, Copy)]
struct ManagedRuntimeAssetDefinition {
    id: &'static str,
    provider: &'static str,
    source: &'static str,
    /// Provider-supported selector passed to the download API. ModelScope does
    /// not accept an arbitrary Git commit hash as `revision`.
    selector: &'static str,
    /// Exact upstream ref resolved immediately before and after the download.
    remote_ref: &'static str,
    expected_commit: &'static str,
    /// Small structural sentinels used to detect an incomplete or manually
    /// damaged snapshot without re-hashing multi-gigabyte model weights on
    /// every repair/update request.
    required_files: &'static [&'static str],
}

#[derive(Debug)]
struct ManagedRuntimeAssetError {
    asset_id: String,
    code: &'static str,
    detail: String,
}

const MODELSCOPE_INSTALL_SCRIPT: &str = r#"import sys
from modelscope.hub.snapshot_download import snapshot_download
path = snapshot_download(model_id=sys.argv[1], revision=sys.argv[2], cache_dir=sys.argv[3])
print(path)
"#;

fn managed_runtime_asset_catalog() -> &'static [ManagedRuntimeAssetDefinition] {
    &[
        ManagedRuntimeAssetDefinition {
            id: "modelscope_sensevoice_small",
            provider: "modelscope",
            source: "iic/SenseVoiceSmall",
            selector: "master",
            remote_ref: "refs/heads/master",
            expected_commit: "7bf452403abd7353a300cd760f7adae7701c92c1",
            required_files: &["model.pt", "config.yaml"],
        },
        ManagedRuntimeAssetDefinition {
            id: "modelscope_fsmn_vad",
            provider: "modelscope",
            source: "iic/speech_fsmn_vad_zh-cn-16k-common-pytorch",
            selector: "v2.0.4",
            remote_ref: "refs/tags/v2.0.4^{}",
            expected_commit: "662fc7a38813d81305085696d59eb5b1141a204a",
            required_files: &["model.pt", "config.yaml"],
        },
    ]
}

fn resolve_declared_runtime_assets(
    asset_ids: &[String],
) -> Result<Vec<&'static ManagedRuntimeAssetDefinition>, ManagedRuntimeAssetError> {
    let catalog = managed_runtime_asset_catalog();
    asset_ids
        .iter()
        .map(|asset_id| {
            catalog
                .iter()
                .find(|definition| definition.id == asset_id)
                .ok_or_else(|| ManagedRuntimeAssetError {
                    asset_id: asset_id.clone(),
                    code: "runtime_asset_unknown",
                    detail: "reason=unknown_host_runtime_asset_id".to_string(),
                })
        })
        .collect()
}

async fn install_declared_runtime_assets(
    asset_ids: &[String],
    install_outcome: &skill_sdk::InstallOutcome,
    storage_directory: &Path,
    control: &skill_sdk::InstallControl,
) -> Result<Vec<String>, ManagedRuntimeAssetError> {
    if asset_ids.is_empty() {
        return Ok(Vec::new());
    }
    let definitions = resolve_declared_runtime_assets(asset_ids)?;
    control
        .phase("runtime_assets")
        .map_err(|error| ManagedRuntimeAssetError {
            asset_id: String::new(),
            code: "runtime_asset_install_cancelled",
            detail: error.detail,
        })?;
    let python = install_outcome.install_root.join("runtime/venv/bin/python");
    if !python.is_file() {
        return Err(ManagedRuntimeAssetError {
            asset_id: String::new(),
            code: "runtime_asset_adapter_unsupported",
            detail: format!("python_runtime_missing path={}", python.display()),
        });
    }
    let cache_directory = storage_directory.join("modelscope");
    let marker_directory = storage_directory.join("runtime-assets");
    let install_home = storage_directory.join("install-home");
    let install_temp = storage_directory.join("install-tmp");
    fs::create_dir_all(&cache_directory).map_err(|error| ManagedRuntimeAssetError {
        asset_id: String::new(),
        code: "runtime_asset_storage_unavailable",
        detail: error.to_string(),
    })?;
    fs::create_dir_all(&marker_directory).map_err(|error| ManagedRuntimeAssetError {
        asset_id: String::new(),
        code: "runtime_asset_storage_unavailable",
        detail: error.to_string(),
    })?;
    fs::create_dir_all(&install_home).map_err(|error| ManagedRuntimeAssetError {
        asset_id: String::new(),
        code: "runtime_asset_storage_unavailable",
        detail: error.to_string(),
    })?;
    fs::create_dir_all(&install_temp).map_err(|error| ManagedRuntimeAssetError {
        asset_id: String::new(),
        code: "runtime_asset_storage_unavailable",
        detail: error.to_string(),
    })?;

    let mut installed = Vec::with_capacity(asset_ids.len());
    for definition in definitions {
        let asset_id = definition.id;
        if control.is_cancelled() {
            return Err(ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_install_cancelled",
                detail: "state=cancelled phase=runtime_asset_iteration".to_string(),
            });
        }
        let marker = marker_directory.join(format!("{}.json", definition.id));
        if runtime_asset_marker_is_valid(&marker, definition, &cache_directory) {
            installed.push(asset_id.to_string());
            continue;
        }
        verify_modelscope_remote_revision(definition, storage_directory, control).await?;
        let mut command = Command::new(&python);
        command
            .arg("-c")
            .arg(MODELSCOPE_INSTALL_SCRIPT)
            .arg(definition.source)
            .arg(definition.selector)
            .arg(&cache_directory)
            .env_clear()
            .env("PATH", dependency_install_path())
            .env("HOME", &install_home)
            .env("TMPDIR", &install_temp)
            .env("TMP", &install_temp)
            .env("TEMP", &install_temp)
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("MODELSCOPE_CACHE", &cache_directory)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONNOUSERSITE", "1")
            .stdin(StdProcessStdio::null())
            .kill_on_drop(true);
        let mut output = Box::pin(command.output());
        let output = loop {
            tokio::select! {
                result = &mut output => break result,
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    if control.is_cancelled() {
                        return Err(ManagedRuntimeAssetError {
                            asset_id: asset_id.to_string(),
                            code: "runtime_asset_install_cancelled",
                            detail: "state=cancelled phase=runtime_asset_download".to_string(),
                        });
                    }
                }
            }
        }
        .map_err(|error| ManagedRuntimeAssetError {
            asset_id: asset_id.to_string(),
            code: "runtime_asset_install_launch_failed",
            detail: error.to_string(),
        })?;
        if !output.status.success() {
            let mut log = String::new();
            append_dependency_log(&mut log, &output.stdout);
            append_dependency_log(&mut log, &output.stderr);
            return Err(ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_install_failed",
                detail: format!(
                    "provider={} exit_code={:?} log_tail={}",
                    definition.provider,
                    output.status.code(),
                    bounded_tail(&log, DEPENDENCY_INSTALL_LOG_LIMIT)
                ),
            });
        }
        let installed_path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .next_back()
            .map(PathBuf::from)
            .ok_or_else(|| ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_install_invalid",
                detail: "installer_did_not_return_asset_path".to_string(),
            })?;
        let canonical_cache =
            fs::canonicalize(&cache_directory).map_err(|error| ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_install_invalid",
                detail: error.to_string(),
            })?;
        let canonical_asset =
            fs::canonicalize(&installed_path).map_err(|error| ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_install_invalid",
                detail: format!("path={} error={error}", installed_path.display()),
            })?;
        if !canonical_asset.starts_with(&canonical_cache) || !canonical_asset.is_dir() {
            return Err(ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_install_invalid",
                detail: format!(
                    "asset_path_outside_private_cache path={}",
                    canonical_asset.display()
                ),
            });
        }
        // A branch/tag selector is necessarily mutable. Resolve the exact ref
        // again after the snapshot finishes so an upstream move cannot be
        // silently accepted during this installation transaction.
        verify_modelscope_remote_revision(definition, storage_directory, control).await?;
        write_runtime_asset_marker(&marker, definition, &canonical_asset).map_err(|error| {
            ManagedRuntimeAssetError {
                asset_id: asset_id.to_string(),
                code: "runtime_asset_marker_failed",
                detail: error.to_string(),
            }
        })?;
        installed.push(asset_id.to_string());
    }
    Ok(installed)
}

async fn verify_modelscope_remote_revision(
    definition: &ManagedRuntimeAssetDefinition,
    storage_directory: &Path,
    control: &skill_sdk::InstallControl,
) -> Result<(), ManagedRuntimeAssetError> {
    if control.is_cancelled() {
        return Err(ManagedRuntimeAssetError {
            asset_id: definition.id.to_string(),
            code: "runtime_asset_install_cancelled",
            detail: "state=cancelled phase=runtime_asset_revision_check".to_string(),
        });
    }
    let remote = format!("https://www.modelscope.cn/{}.git", definition.source);
    let mut command = Command::new("git");
    command
        .arg("ls-remote")
        .arg("--exit-code")
        .arg(&remote)
        .arg(definition.remote_ref)
        .env_clear()
        .env("PATH", dependency_install_path())
        .env("HOME", storage_directory)
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(StdProcessStdio::null())
        .kill_on_drop(true);
    let mut output = Box::pin(command.output());
    let output = loop {
        tokio::select! {
            result = &mut output => break result,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if control.is_cancelled() {
                    return Err(ManagedRuntimeAssetError {
                        asset_id: definition.id.to_string(),
                        code: "runtime_asset_install_cancelled",
                        detail: "state=cancelled phase=runtime_asset_revision_check".to_string(),
                    });
                }
            }
        }
    }
    .map_err(|error| ManagedRuntimeAssetError {
        asset_id: definition.id.to_string(),
        code: "runtime_asset_revision_check_failed",
        detail: format!("remote={remote} error={error}"),
    })?;
    if !output.status.success() {
        let mut log = String::new();
        append_dependency_log(&mut log, &output.stdout);
        append_dependency_log(&mut log, &output.stderr);
        return Err(ManagedRuntimeAssetError {
            asset_id: definition.id.to_string(),
            code: "runtime_asset_revision_check_failed",
            detail: format!(
                "remote={} remote_ref={} exit_code={:?} log_tail={}",
                remote,
                definition.remote_ref,
                output.status.code(),
                bounded_tail(&log, DEPENDENCY_INSTALL_LOG_LIMIT)
            ),
        });
    }
    let actual_commit = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual_commit != definition.expected_commit {
        return Err(ManagedRuntimeAssetError {
            asset_id: definition.id.to_string(),
            code: "runtime_asset_revision_mismatch",
            detail: format!(
                "remote={} remote_ref={} expected_commit={} actual_commit={}",
                remote, definition.remote_ref, definition.expected_commit, actual_commit
            ),
        });
    }
    Ok(())
}

fn runtime_asset_marker_is_valid(
    marker: &Path,
    definition: &ManagedRuntimeAssetDefinition,
    cache_directory: &Path,
) -> bool {
    let Ok(value) = fs::read_to_string(marker)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(std::io::Error::other))
    else {
        return false;
    };
    if value.get("asset_id").and_then(Value::as_str) != Some(definition.id)
        || value.get("provider").and_then(Value::as_str) != Some(definition.provider)
        || value.get("source").and_then(Value::as_str) != Some(definition.source)
        || value.get("selector").and_then(Value::as_str) != Some(definition.selector)
        || value.get("remote_ref").and_then(Value::as_str) != Some(definition.remote_ref)
        || value.get("expected_commit").and_then(Value::as_str) != Some(definition.expected_commit)
    {
        return false;
    }
    let Some(path) = value.get("path").and_then(Value::as_str).map(PathBuf::from) else {
        return false;
    };
    let (Ok(path), Ok(cache)) = (fs::canonicalize(path), fs::canonicalize(cache_directory)) else {
        return false;
    };
    path.is_dir()
        && path.starts_with(cache)
        && definition.required_files.iter().all(|relative| {
            let file = path.join(relative);
            file.is_file() && fs::metadata(file).is_ok_and(|metadata| metadata.len() > 0)
        })
}

fn write_runtime_asset_marker(
    marker: &Path,
    definition: &ManagedRuntimeAssetDefinition,
    asset_path: &Path,
) -> std::io::Result<()> {
    let temporary = marker.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4().simple()));
    let payload = serde_json::to_vec_pretty(&json!({
        "schema_version": 1,
        "asset_id": definition.id,
        "provider": definition.provider,
        "source": definition.source,
        "selector": definition.selector,
        "remote_ref": definition.remote_ref,
        "expected_commit": definition.expected_commit,
        "path": asset_path,
    }))
    .map_err(std::io::Error::other)?;
    fs::write(&temporary, payload)?;
    fs::rename(temporary, marker)
}

#[cfg(test)]
mod managed_runtime_asset_tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "managed-runtime-asset-test-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&path).expect("create asset test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn catalog_ids_are_unique_and_provider_revisions_are_pinned() {
        let catalog = managed_runtime_asset_catalog();
        let ids = catalog.iter().map(|item| item.id).collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), catalog.len());
        for item in catalog {
            assert!(!item.selector.is_empty());
            assert!(item.remote_ref.starts_with("refs/"));
            assert_eq!(item.expected_commit.len(), 40);
            assert!(item
                .expected_commit
                .chars()
                .all(|character| character.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn unknown_runtime_asset_is_rejected_before_installation() {
        let error = resolve_declared_runtime_assets(&["unknown_asset".to_string()])
            .expect_err("unknown asset must be rejected");
        assert_eq!(error.code, "runtime_asset_unknown");
        assert_eq!(error.asset_id, "unknown_asset");
    }

    #[test]
    fn marker_requires_the_pinned_revision_and_private_cache_path() {
        let root = TestDirectory::new();
        let cache = root.path().join("cache");
        let asset = cache.join("asset");
        let marker = root.path().join("asset.json");
        fs::create_dir_all(&asset).expect("private asset directory");
        let definition = managed_runtime_asset_catalog()[0];
        for relative in definition.required_files {
            fs::write(asset.join(relative), b"fixture").expect("required model file");
        }
        write_runtime_asset_marker(&marker, &definition, &asset).expect("write marker");
        assert!(runtime_asset_marker_is_valid(&marker, &definition, &cache));

        fs::remove_file(asset.join(definition.required_files[0])).expect("remove required file");
        assert!(!runtime_asset_marker_is_valid(
            &marker,
            &definition,
            &cache
        ));
        fs::write(asset.join(definition.required_files[0]), b"fixture")
            .expect("restore required file");

        let different_revision = ManagedRuntimeAssetDefinition {
            expected_commit: "0000000000000000000000000000000000000000",
            ..definition
        };
        assert!(!runtime_asset_marker_is_valid(
            &marker,
            &different_revision,
            &cache
        ));

        let outside = root.path().join("outside");
        fs::create_dir_all(&outside).expect("outside directory");
        write_runtime_asset_marker(&marker, &definition, &outside).expect("outside marker");
        assert!(!runtime_asset_marker_is_valid(&marker, &definition, &cache));
    }
}
