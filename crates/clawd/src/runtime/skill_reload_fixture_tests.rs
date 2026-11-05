use super::super::*;
use crate::skill_admission::{
    install_test_fixture, test_policy_grant, AdmissionMutation, ExternalSkillMetadata,
    SkillAdmissionSource,
};
use skill_sdk::{AdmissionState, ApprovalSource, InstallReceiptStore, PackageManifest};
use std::fs::{self, OpenOptions};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) const PROBE: &str = "runtime_reload_probe";

pub(super) struct Fixture {
    pub state: AppState,
    pub root: PathBuf,
    pub config: PathBuf,
    pub relative_config: String,
    pub service: SkillAdmissionService,
    manifest: PackageManifest,
}

impl Fixture {
    pub fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("agent-runtime-reload-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).expect("exclusively create fixture");
        let root = root.canonicalize().expect("canonical fixture root");
        fs::create_dir(root.join("configs")).unwrap();
        let relative_config = format!("configs/reload-{}.toml", uuid::Uuid::new_v4().simple());
        let config = root.join(&relative_config);
        fs::write(&config, "[server]\nrequest_timeout_seconds=30\n[database]\nsqlite_path='data/fixture.db'\nbusy_timeout_ms=1000\nskill_data_root='data/skills'\n[worker]\nconcurrency=1\n[skills]\nregistry_path='configs/skills_registry.toml'\n").unwrap();
        fs::write(root.join("configs/skills_registry.toml"), format!(
            "[[skills]]\nname='{PROBE}'\nenabled=false\nkind='runner'\nplanner_kind='skill'\npackage_manifest='sources/{PROBE}/skill.toml'\ninstall_mode='on_demand'\n"
        )).unwrap();
        let manifest = install_test_fixture(&root, PROBE);
        let parsed = AppConfig::load(config.to_str().unwrap()).expect("load isolated config");
        let service = SkillAdmissionService::from_config(&root, &parsed).unwrap();
        service
            .admit_bundled(AdmissionMutation {
                metadata: ExternalSkillMetadata {
                    name: PROBE.to_string(),
                    source: SkillAdmissionSource::BundledBase,
                    package_manifest_path: format!("sources/{PROBE}/skill.toml"),
                    description: "Owned runtime publication fixture".to_string(),
                    aliases: Vec::new(),
                    group: "extensions".to_string(),
                },
                prompt: "# Runtime publication probe\n".to_string(),
                state: AdmissionState::Enabled,
                grant: Some(test_policy_grant(
                    &manifest,
                    ApprovalSource::ReleaseBaseline,
                )),
            })
            .unwrap();
        let mut state = AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();
        state.reload_ctx.config_path_for_reload = config.to_string_lossy().into_owned();
        reload_skill_views(&state).expect("initialize isolated runtime view");
        Self {
            state,
            root,
            config,
            relative_config,
            service,
            manifest,
        }
    }

    pub fn program_path(&self) -> PathBuf {
        // This is the known dummy artifact written by install_test_fixture,
        // not runtime entrypoint inference.
        InstallReceiptStore::new(self.root.join("data/skill-packages"))
            .version_dir(
                PROBE,
                &self.manifest.package.version,
                &self.manifest.digest().unwrap(),
            )
            .unwrap()
            .join("runtime/skill")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn admission_boundary_is_held(root: &Path) -> bool {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("data/skills/.runtime-admission/mutation.lock"))
        .unwrap();
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {
            fs2::FileExt::unlock(&lock).unwrap();
            false
        }
        Err(error) => {
            assert_eq!(
                error.raw_os_error(),
                fs2::lock_contended_error().raw_os_error(),
                "only actual lock contention proves the publication boundary"
            );
            true
        }
    }
}

pub(super) struct PausedReload {
    release: Option<Sender<()>>,
    worker: Option<JoinHandle<Result<ReloadSkillViewsResult, String>>>,
}

impl PausedReload {
    pub fn start(state: AppState) -> Self {
        let (ready, waiting) = mpsc::sync_channel(0);
        let (release, gate) = mpsc::channel();
        let worker = thread::spawn(move || {
            prepare_and_publish_views(&state, || {
                ready.send(()).expect("test observer is alive");
                gate.recv().expect("test releases prepared reload");
            })
        });
        let paused = Self {
            release: Some(release),
            worker: Some(worker),
        };
        waiting
            .recv_timeout(Duration::from_secs(30))
            .expect("reload reaches publication boundary");
        paused
    }

    pub fn finish(mut self) -> Result<ReloadSkillViewsResult, String> {
        self.resume();
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("reload worker must not panic")
    }

    fn resume(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for PausedReload {
    fn drop(&mut self) {
        self.resume();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
