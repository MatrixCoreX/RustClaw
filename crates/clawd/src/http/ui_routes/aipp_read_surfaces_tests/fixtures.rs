//! Private data fixtures; never included by production code.
use super::super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum Surface {
    Catalog,
    Preview,
    Bundle,
}

pub(super) const SURFACES: [Surface; 3] = [Surface::Catalog, Surface::Preview, Surface::Bundle];
pub(super) const PREVIEW: &[u8] = b"fixture-preview";
pub(super) const BUNDLE: &[u8] = b"<!doctype html><title>Fixture</title>";

pub(super) struct SurfaceFixture {
    pub(super) state: AppState,
    pub(super) workspace: Workspace,
    storage: Option<Workspace>,
}

impl SurfaceFixture {
    pub(super) fn new(surface: Surface) -> Self {
        let (state, workspace) = fixture();
        let mut value = Self {
            state,
            workspace,
            storage: None,
        };
        let mut manifest = skill_sdk::PackageManifest::load(&value.manifest_path()).unwrap();
        let aipp = manifest.aipp.as_mut().unwrap();
        match surface {
            Surface::Catalog => {}
            Surface::Preview => {
                aipp.renderer = "collection_feed_v1".into();
                aipp.data_contract = "media_collection_v1".into();
                aipp.task_channel_scope = None;
                let root = Workspace(
                    value
                        .state
                        .core
                        .skill_storage
                        .directory_path("fixture_view")
                        .unwrap(),
                );
                fs::create_dir_all(root.0.join("records")).unwrap();
                fs::create_dir_all(root.0.join("exports/images")).unwrap();
                fs::write(root.0.join("exports/images/preview.png"), PREVIEW).unwrap();
                write_record(
                    &root.0,
                    1,
                    json!({
                        "global_sequence": 1, "kind": "image",
                        "image_screenshot_path": "images/preview.png",
                    }),
                );
                value.storage = Some(root);
            }
            Surface::Bundle => {
                aipp.renderer = "sandbox_bundle_v1".into();
                aipp.data_contract = "capability_bridge_v1".into();
                aipp.task_channel_scope = None;
                aipp.asset_root = Some("aipp".into());
                aipp.entrypoint = Some("aipp/index.html".into());
                fs::create_dir_all(value.bundle_root()).unwrap();
                fs::write(value.bundle_root().join("index.html"), BUNDLE).unwrap();
            }
        }
        fs::write(value.manifest_path(), manifest.to_toml_string().unwrap()).unwrap();
        value
    }

    pub(super) fn manifest_path(&self) -> PathBuf {
        self.workspace.0.join("sources/fixture_view/skill.toml")
    }

    pub(super) fn bundle_root(&self) -> PathBuf {
        self.workspace.0.join("sources/fixture_view/aipp")
    }

    pub(super) fn storage_root(&self) -> &Path {
        &self.storage.as_ref().unwrap().0
    }

    /// Construct receipt evidence only; no compiler, process launch or real admission occurs.
    pub(super) fn admitted_bundle(&self) -> PathBuf {
        use skill_sdk::receipt::{LaunchProgramScope, ReceiptLaunch};
        let manifest = skill_sdk::PackageManifest::load(&self.manifest_path()).unwrap();
        let manifest_digest = manifest.digest().unwrap();
        let store = skill_sdk::InstallReceiptStore::new(skill_package_root(&self.state));
        let install = store
            .version_dir("fixture_view", &manifest.package.version, &manifest_digest)
            .unwrap();
        fs::create_dir_all(install.join("runtime/bin")).unwrap();
        fs::create_dir_all(install.join("aipp")).unwrap();
        fs::write(
            install.join("runtime/bin/fixture"),
            b"unexecuted-fixture-binary",
        )
        .unwrap();
        fs::write(install.join("aipp/index.html"), BUNDLE).unwrap();
        fs::write(
            install.join("skill.toml"),
            manifest.to_toml_string().unwrap(),
        )
        .unwrap();
        let artifacts = ["runtime/bin/fixture", "aipp/index.html"]
            .into_iter()
            .map(|name| {
                let path = install.join(name);
                skill_sdk::ArtifactReceipt {
                    path: name.into(),
                    sha256: skill_sdk::digest_file(&path).unwrap(),
                    size_bytes: fs::metadata(path).unwrap().len(),
                    executable: name.starts_with("runtime/"),
                }
            })
            .collect();
        let receipt = skill_sdk::InstallReceipt {
            schema_version: skill_sdk::INSTALL_RECEIPT_SCHEMA_VERSION,
            skill_name: "fixture_view".into(),
            version: manifest.package.version.clone(),
            manifest_digest,
            semantic_contract_digest: Some(manifest.capability_request_digest().unwrap()),
            source_digest: "a".repeat(64),
            lockfile_digests: BTreeMap::new(),
            adapter: skill_sdk::BuildAdapter::Cargo,
            adapter_version: "fixture".into(),
            platform: skill_sdk::HostPlatform::current(),
            artifacts,
            launch: ReceiptLaunch {
                launcher: skill_sdk::LauncherKind::Native,
                program: "runtime/bin/fixture".into(),
                program_scope: LaunchProgramScope::Package,
                args: vec![],
                working_directory: "runtime".into(),
                environment: BTreeMap::new(),
                environment_allowlist: vec![],
                trusted_runtime_sha256: None,
                trusted_runtime_version: None,
                remote_endpoint: None,
            },
            sandbox_profile: skill_sdk::SandboxProfile::Required,
            runtime_network: false,
            protocol_smoke: skill_sdk::ProtocolSmokeReceipt {
                protocol: "agent-jsonl-v1".into(),
                passed: true,
                request_id: "fixture-only".into(),
                checked_at_unix: 1,
            },
            installed_at_unix: 1,
        };
        store.write_receipt(&install, &receipt).unwrap();
        store.activate(&install, &receipt).unwrap();
        let verified = store.verified_current_install("fixture_view").unwrap();
        let current = self.state.get_skill_views_snapshot();
        let mut replacement = crate::SkillViewsSnapshot {
            registry: current.registry.clone(),
            skills_list: current.skills_list.clone(),
            binding: Default::default(),
        };
        replacement.binding.admission_bindings = Arc::new(BTreeMap::from([(
            "fixture_view".into(),
            crate::skill_admission::AdmissionExecutionBinding {
                version: receipt.version.clone(),
                manifest_digest: receipt.manifest_digest.clone(),
                install_receipt_digest: receipt.digest().unwrap(),
                policy_digest: None,
                admission_receipt_digest: "b".repeat(64),
            },
        )]));
        *self.state.core.skill_views_snapshot.write().unwrap() = Arc::new(replacement);
        verified.install_dir
    }
}
