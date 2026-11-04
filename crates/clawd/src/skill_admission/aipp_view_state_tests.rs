use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use super::{AippViewState, SkillAdmissionService};

struct Workspace(PathBuf);

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (SkillAdmissionService, Workspace) {
    let root =
        Workspace(std::env::temp_dir().join(format!("aipp-view-state-{}", uuid::Uuid::new_v4())));
    fs::create_dir_all(&root.0).unwrap();
    let service = SkillAdmissionService::for_test(&root.0, &root.0.join("registry.toml"));
    (service, root)
}

fn empty_view() -> AippViewState {
    AippViewState {
        installed: true,
        cleared_through_event_ms: 0,
        hidden_task_ids: BTreeSet::new(),
        hidden_record_sequences: BTreeSet::new(),
    }
}

#[test]
fn aipp_view_state_defaults_are_read_only_and_validate_names() {
    let (service, root) = fixture();
    assert_eq!(
        service.aipp_view_state("fixture_app").unwrap(),
        empty_view()
    );
    for invalid in ["", "../outside", "app/child"] {
        assert_eq!(
            service.aipp_view_state(invalid).unwrap_err().code,
            "aipp_skill_name_invalid"
        );
    }
    assert!(!root.0.join("data").exists());
}

#[test]
fn aipp_view_state_is_scoped_and_does_not_mutate_the_shared_file() {
    let (service, _root) = fixture();
    for (name, watermark, task, sequence) in [
        ("fixture_app", 42, "task-one", 7),
        ("other_app", 99, "foreign-task", 81),
    ] {
        service
            .set_aipp_cleared_through_event_ms(name, watermark)
            .unwrap();
        service
            .hide_aipp_task_ids(name, &[task.to_string()])
            .unwrap();
        service
            .hide_aipp_record_sequences(name, &[sequence])
            .unwrap();
    }
    service.set_aipp_installed("other_app", false).unwrap();
    let path = service.root().join("aipp-install-state.json");
    let before = fs::read(&path).unwrap();
    let expected = AippViewState {
        installed: true,
        cleared_through_event_ms: 42,
        hidden_task_ids: BTreeSet::from(["task-one".into()]),
        hidden_record_sequences: BTreeSet::from([7]),
    };
    assert_eq!(service.aipp_view_state("fixture_app").unwrap(), expected);
    assert!(!service.aipp_view_state("other_app").unwrap().installed);
    assert_eq!(service.aipp_view_state("new_app").unwrap(), empty_view());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn aipp_view_state_is_owned_but_subsequent_reads_are_fresh() {
    let (service, _root) = fixture();
    let first = service.aipp_view_state("fixture_app").unwrap();
    service.set_aipp_installed("fixture_app", false).unwrap();
    service
        .set_aipp_cleared_through_event_ms("fixture_app", 123)
        .unwrap();
    service
        .hide_aipp_task_ids("fixture_app", &["hidden".into()])
        .unwrap();
    service
        .hide_aipp_record_sequences("fixture_app", &[9])
        .unwrap();
    let second = service.aipp_view_state("fixture_app").unwrap();
    assert_eq!(first, empty_view());
    assert!(!second.installed);
    assert_eq!(second.cleared_through_event_ms, 123);
    assert_eq!(second.hidden_task_ids, BTreeSet::from(["hidden".into()]));
    assert_eq!(second.hidden_record_sequences, BTreeSet::from([9]));
    service.set_aipp_installed("fixture_app", true).unwrap();
    assert!(service.aipp_view_state("fixture_app").unwrap().installed);
    fs::remove_file(service.root().join("aipp-install-state.json")).unwrap();
    assert_eq!(
        service.aipp_view_state("fixture_app").unwrap(),
        empty_view()
    );
    assert!(!second.installed);
}

#[test]
fn aipp_view_state_preserves_schema_validation_and_existing_watermark_alias() {
    let (service, _root) = fixture();
    fs::create_dir_all(service.root()).unwrap();
    let path = service.root().join("aipp-install-state.json");
    for invalid in [
        "not JSON",
        r#"{"schema_version":2}"#,
        r#"{"schema_version":1,"unexpected":true}"#,
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(service.aipp_view_state("fixture_app").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }
    fs::write(
        &path,
        r#"{"schema_version":1,"cleared_through_sequence":{"fixture_app":42}}"#,
    )
    .unwrap();
    assert_eq!(
        service
            .aipp_view_state("fixture_app")
            .unwrap()
            .cleared_through_event_ms,
        42
    );
}
