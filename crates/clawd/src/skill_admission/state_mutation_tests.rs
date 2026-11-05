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
    // Recovery owns only its published generation, never a later operation.
    let error = fixture
        .service
        .rollback_generation(own.generation)
        .unwrap_err();
    assert_eq!(error.code, "skill_admission_generation_changed");
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
    assert_eq!(after.generation_digest, other.generation_digest);
}

fn change(name: &str, state: AdmissionState) -> crate::skill_admission::AdmissionStateChange {
    crate::skill_admission::AdmissionStateChange {
        skill_name: name.to_string(),
        state,
        grant: None,
    }
}

fn pointer(fixture: &Fixture) -> Vec<u8> {
    fs::read(fixture.service.root().join("current-generation.json")).unwrap()
}

#[test]
fn state_batch_publishes_one_generation_and_owned_rollback_restores_it_once() {
    let fixture = Fixture::new();
    let before = fixture.service.snapshot().unwrap();
    let after = fixture
        .service
        .set_states(
            vec![
                change("switch_probe_alpha", AdmissionState::InstalledDisabled),
                change("switch_probe_beta", AdmissionState::InstalledDisabled),
            ],
            Some(before.generation),
        )
        .unwrap();
    assert_eq!(after.generation, before.generation + 1);
    for name in ["switch_probe_alpha", "switch_probe_beta"] {
        assert_eq!(after.state(name), Some(AdmissionState::InstalledDisabled));
        assert!(!after.execution_bindings.contains_key(name));
    }
    assert_eq!(
        fixture
            .other_operation()
            .snapshot()
            .unwrap()
            .generation_digest,
        after.generation_digest
    );
    let restored = fixture
        .service
        .rollback_generation(after.generation)
        .unwrap();
    assert_eq!(restored.generation_digest, before.generation_digest);
    assert_eq!(restored.enabled, before.enabled);
}

#[test]
fn invalid_or_duplicate_state_batch_members_never_publish_partial_changes() {
    for name in ["missing_probe", "../outside", "switch_probe_alpha"] {
        let fixture = Fixture::new();
        let before = pointer(&fixture);
        let result = fixture.service.set_states(
            vec![
                change("switch_probe_alpha", AdmissionState::InstalledDisabled),
                change(name, AdmissionState::InstalledDisabled),
            ],
            None,
        );
        assert!(result.is_err(), "invalid member {name}");
        assert_eq!(pointer(&fixture), before);
        assert_eq!(fixture.service.snapshot().unwrap().enabled.len(), 2);
    }
}

#[test]
fn stale_state_batch_including_an_empty_batch_cannot_undo_a_later_disable() {
    let fixture = Fixture::new();
    let before = fixture.service.snapshot().unwrap();
    let other = fixture
        .other_operation()
        .set_state("switch_probe_beta", AdmissionState::InstalledDisabled, None)
        .unwrap();
    let saved = pointer(&fixture);
    for changes in [
        vec![],
        vec![change("switch_probe_beta", AdmissionState::Enabled)],
    ] {
        let error = fixture
            .service
            .set_states(changes, Some(before.generation))
            .unwrap_err();
        assert_eq!(error.code, "skill_admission_generation_changed");
        assert_eq!(pointer(&fixture), saved);
    }
    assert_eq!(
        fixture.service.snapshot().unwrap().generation_digest,
        other.generation_digest
    );
}

#[test]
fn state_batch_without_a_change_does_not_publish_or_modify_the_pointer() {
    let fixture = Fixture::new();
    let before = fixture.service.snapshot().unwrap();
    let saved = pointer(&fixture);
    let after = fixture
        .service
        .set_states(vec![], Some(before.generation))
        .unwrap();
    assert_eq!(pointer(&fixture), saved);
    assert_eq!(before.generation_digest, after.generation_digest);
}

#[test]
fn revoked_grant_cannot_be_restored_implicitly_by_a_state_batch() {
    let fixture = Fixture::new();
    let before = fixture
        .service
        .set_state(
            "switch_probe_beta",
            AdmissionState::AwaitingPolicyApproval,
            None,
        )
        .unwrap();
    let saved = pointer(&fixture);
    let error = fixture
        .service
        .set_states(
            vec![
                change("switch_probe_alpha", AdmissionState::InstalledDisabled),
                change("switch_probe_beta", AdmissionState::Enabled),
            ],
            Some(before.generation),
        )
        .unwrap_err();
    assert_eq!(error.code, "skill_policy_grant_missing");
    assert_eq!(pointer(&fixture), saved);
    let after = fixture.service.snapshot().unwrap();
    assert_eq!(
        after.state("switch_probe_alpha"),
        Some(AdmissionState::Enabled)
    );
    assert_eq!(
        after.state("switch_probe_beta"),
        Some(AdmissionState::AwaitingPolicyApproval)
    );
}

#[test]
fn state_changes_reject_tampered_current_metadata_instead_of_blessing_it() {
    for field in ["metadata", "prompts"] {
        let fixture = Fixture::new();
        let before = fixture.service.snapshot().unwrap();
        let saved = pointer(&fixture);
        let root = fixture.service.root().join("generations");
        let generation = fs::read_dir(root).unwrap().next().unwrap().unwrap().path();
        let path = generation.join(field).join(if field == "metadata" {
            "switch_probe_beta.json"
        } else {
            "switch_probe_beta.md"
        });
        if field == "metadata" {
            let mut metadata: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            metadata["description"] = "unverified change".into();
            fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        } else {
            fs::write(path, "# unverified change\n").unwrap();
        }
        let error = fixture
            .service
            .set_states(
                vec![change(
                    "switch_probe_beta",
                    AdmissionState::InstalledDisabled,
                )],
                Some(before.generation),
            )
            .unwrap_err();
        assert_eq!(error.code, "skill_admission_metadata_mismatch");
        assert_eq!(pointer(&fixture), saved);
    }
}

#[test]
fn concurrent_state_batches_with_the_same_generation_have_exactly_one_winner() {
    use std::sync::{Arc, Barrier};
    let fixture = Fixture::new();
    let before = fixture.service.snapshot().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let actors = ["switch_probe_alpha", "switch_probe_beta"].map(|name| {
        let barrier = barrier.clone();
        let service = fixture.other_operation();
        std::thread::spawn(move || {
            barrier.wait();
            service.set_states(
                vec![change(name, AdmissionState::InstalledDisabled)],
                Some(before.generation),
            )
        })
    });
    barrier.wait();
    let results = actors.map(|actor| actor.join().unwrap());
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .filter(|error| error.code == "skill_admission_generation_changed")
            .count(),
        1
    );
    let after = fixture.service.snapshot().unwrap();
    assert_eq!(after.generation, before.generation + 1);
    assert_eq!(after.disabled.len(), 1);
    assert_eq!(after.enabled.len(), 1);
}
