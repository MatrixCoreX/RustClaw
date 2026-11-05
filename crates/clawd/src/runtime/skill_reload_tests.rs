use super::*;
use skill_sdk::AdmissionState;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[path = "skill_reload_fixture_tests.rs"]
mod fixture;
use fixture::*;

#[test]
fn skill_reload_absolute_config_preserves_the_verified_binding() {
    let fixture = Fixture::new();
    reload_skill_views(&fixture.state).unwrap();
    let overlay = fixture.service.snapshot().unwrap();
    let view = fixture.state.get_skill_views_snapshot();
    assert_eq!(view.binding.registry_generation, overlay.generation);
    assert_eq!(
        view.binding.overlay_generation_digest,
        overlay.generation_digest
    );
    assert!(view.skills_list.contains(PROBE));
    assert_eq!(
        view.binding.admission_bindings.get(PROBE),
        overlay.execution_bindings.get(PROBE)
    );
}

#[test]
fn skill_reload_relative_config_uses_workspace_instead_of_process_cwd() {
    let mut fixture = Fixture::new();
    fixture.state.reload_ctx.config_path_for_reload = fixture.relative_config.clone();
    let result = reload_skill_views(&fixture.state);
    assert!(
        result.is_ok(),
        "workspace-relative config must reload: {result:?}"
    );
    assert_eq!(
        fixture.state.get_skills_list().as_ref(),
        &HashSet::from([PROBE.to_string()])
    );
}

#[test]
fn skill_reload_empty_config_uses_the_workspace_default() {
    let mut fixture = Fixture::new();
    fs::copy(&fixture.config, fixture.root.join("configs/config.toml")).unwrap();
    fixture.state.reload_ctx.config_path_for_reload.clear();
    let result = reload_skill_views(&fixture.state);
    assert!(
        result.is_ok(),
        "empty config selection must use workspace default: {result:?}"
    );
    assert!(fixture.state.get_skills_list().contains(PROBE));
}

#[test]
fn skill_reload_publication_coordinates_with_the_real_admission_lock() {
    let fixture = Fixture::new();
    let mut held = false;
    prepare_and_publish_views(&fixture.state, || {
        held = admission_boundary_is_held(&fixture.root)
    })
    .unwrap();
    assert!(
        held,
        "preparation through publication must be inside the admission boundary"
    );
}

#[test]
fn skill_reload_old_preparation_cannot_overwrite_a_newer_disable() {
    let fixture = Fixture::new();
    let old = PausedReload::start(fixture.state.clone());
    let boundary_held = admission_boundary_is_held(&fixture.root);
    let state = fixture.state.clone();
    let service = fixture.service.clone();
    let (finished, waiting) = mpsc::channel();
    let newer = thread::spawn(move || {
        let snapshot = service
            .set_state(PROBE, AdmissionState::InstalledDisabled, None)
            .map_err(|error| error.to_string())?;
        reload_skill_views(&state)?;
        let _ = finished.send(());
        Ok::<_, String>(snapshot.generation)
    });
    // On the unfixed path, deliberately finish the newer publication first.
    // A coordinated implementation blocks it until the old publication ends.
    let ordering = if boundary_held {
        Ok(())
    } else {
        waiting.recv_timeout(Duration::from_secs(30))
    };
    let old_result = old.finish();
    let newer_result = newer.join().expect("newer mutation worker must not panic");
    ordering.expect("uncoordinated path must exercise the late old publication");
    old_result.expect("old reload completes");
    let generation = newer_result.expect("newer disable and reload complete");
    let current = fixture.service.snapshot().unwrap();
    let view = fixture.state.get_skill_views_snapshot();
    assert_eq!(current.generation, generation);
    assert_eq!(
        view.binding.registry_generation, generation,
        "late preparation must not restore an old runtime generation"
    );
    assert_eq!(
        view.binding.overlay_generation_digest,
        current.generation_digest
    );
    assert!(
        !view.skills_list.contains(PROBE),
        "a successful disable must remain disabled"
    );
    assert!(!view.binding.admission_bindings.contains_key(PROBE));
}

#[test]
fn skill_reload_legitimate_owned_rollback_may_publish_an_earlier_generation() {
    let fixture = Fixture::new();
    let original = fixture.service.snapshot().unwrap();
    let disabled = fixture
        .service
        .set_state(PROBE, AdmissionState::InstalledDisabled, None)
        .unwrap();
    reload_skill_views(&fixture.state).unwrap();
    assert!(!fixture.state.get_skills_list().contains(PROBE));
    let restored = fixture
        .service
        .rollback_generation(disabled.generation)
        .unwrap();
    assert!(restored.generation < disabled.generation);
    reload_skill_views(&fixture.state).unwrap();
    let view = fixture.state.get_skill_views_snapshot();
    assert_eq!(view.binding.registry_generation, original.generation);
    assert_eq!(
        view.binding.overlay_generation_digest,
        original.generation_digest
    );
    assert!(view.skills_list.contains(PROBE));
}

#[test]
fn skill_reload_tampered_artifact_cannot_replace_the_last_verified_view() {
    let fixture = Fixture::new();
    let before = fixture.state.get_skill_views_snapshot();
    fs::write(fixture.program_path(), b"tampered fixture artifact").unwrap();
    assert!(reload_skill_views(&fixture.state).is_err());
    assert!(Arc::ptr_eq(
        &before,
        &fixture.state.get_skill_views_snapshot()
    ));
}

#[test]
fn skill_reload_invalid_configuration_keeps_the_previous_verified_view() {
    let fixture = Fixture::new();
    let before = fixture.state.get_skill_views_snapshot();
    fs::write(&fixture.config, "[unterminated fixture").unwrap();
    assert!(reload_skill_views(&fixture.state).is_err());
    assert!(Arc::ptr_eq(
        &before,
        &fixture.state.get_skill_views_snapshot()
    ));
}
