use super::*;
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    registry: PathBuf,
    service: SkillAdmissionService,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-state-mutation-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir(&root).expect("exclusively create fixture");
        let root = root.canonicalize().expect("canonical fixture root");
        let registry = root.join("configs/skills_registry.toml");
        let service = SkillAdmissionService::for_test(&root, &registry);
        let fixture = Self {
            root,
            registry,
            service,
        };
        fs::create_dir(fixture.root.join("configs")).unwrap();
        let names = ["switch_probe_alpha", "switch_probe_beta"];
        let text = names.iter().map(|name| format!(
            "[[skills]]\nname='{name}'\nenabled=false\nkind='runner'\nplanner_kind='skill'\npackage_manifest='sources/{name}/skill.toml'\ninstall_mode='on_demand'\n"
        )).collect::<Vec<_>>().join("\n");
        fs::write(&fixture.registry, text).unwrap();
        let mutations = names
            .iter()
            .map(|name| {
                let manifest = install_fixture(&fixture.root, name);
                AdmissionMutation {
                    metadata: ExternalSkillMetadata {
                        name: (*name).to_string(),
                        source: SkillAdmissionSource::BundledBase,
                        package_manifest_path: format!("sources/{name}/skill.toml"),
                        description: "Owned state mutation fixture".to_string(),
                        aliases: Vec::new(),
                        group: "extensions".to_string(),
                    },
                    prompt: format!("# {name}\n"),
                    state: AdmissionState::Enabled,
                    grant: Some(fixture_grant(&manifest, ApprovalSource::ReleaseBaseline)),
                }
            })
            .collect();
        fixture.service.admit_bundled_batch(mutations).unwrap();
        fixture
    }

    fn other_operation(&self) -> SkillAdmissionService {
        SkillAdmissionService::for_test(&self.root, &self.registry)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn failed_operation_rollback_must_not_reenable_an_unrelated_operation_disabled_skill() {
    let fixture = Fixture::new();
    let initial = fixture.service.snapshot().unwrap();
    let own = fixture
        .service
        .set_state(
            "switch_probe_alpha",
            AdmissionState::InstalledDisabled,
            None,
        )
        .unwrap();
    let other = fixture
        .other_operation()
        .set_state("switch_probe_beta", AdmissionState::InstalledDisabled, None)
        .unwrap();
    assert!(other.generation > own.generation);
    // Exact old automatic-recovery flow used by the multi-entry UI update:
    // one operation commits, another succeeds, then the first operation fails.
    fixture
        .service
        .rollback_to_generation(initial.generation)
        .unwrap();
    let after = fixture.service.snapshot().unwrap();
    eprintln!(
        "interleaved_state_rollback unrelated_disable_preserved={}",
        after.state("switch_probe_beta") == Some(AdmissionState::InstalledDisabled)
    );
    assert_eq!(
        after.state("switch_probe_beta"),
        Some(AdmissionState::InstalledDisabled),
        "automatic recovery must not undo another operation's successful disable"
    );
}
