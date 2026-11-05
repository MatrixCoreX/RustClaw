//! Deterministic generation interleavings; no production state or process is touched.
use super::*;
use std::sync::Arc;

fn disabled_view(original: &crate::SkillViewsSnapshot) -> Arc<crate::SkillViewsSnapshot> {
    let mut view = crate::SkillViewsSnapshot {
        registry: original.registry.clone(),
        skills_list: Arc::new(Default::default()),
        binding: Default::default(),
    };
    view.binding.registry_generation = original.binding.registry_generation + 1;
    Arc::new(view)
}

fn install_view(fixture: &SurfaceFixture, view: Arc<crate::SkillViewsSnapshot>) {
    *fixture.state.core.skill_views_snapshot.write().unwrap() = view;
}

#[test]
fn aipp_metadata_resolution_uses_only_the_captured_registry_view() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let install = fixture.admitted_bundle();
    let captured = fixture.state.get_skill_views_snapshot();
    install_view(&fixture, Arc::new(crate::SkillViewsSnapshot::default()));

    let active = active_aipp_package_in_view(&fixture.state, "fixture_view", &captured)
        .unwrap()
        .unwrap();
    assert_eq!(active.package_root, install);
    assert_eq!(
        active.admission_binding.as_ref(),
        captured.binding.admission_bindings.get("fixture_view")
    );
    assert_eq!(
        active_aipp_package(&fixture.state, "fixture_view")
            .err()
            .as_deref(),
        Some("aipp_registry_unavailable")
    );
}

#[test]
fn aipp_disabled_captured_view_cannot_borrow_a_new_views_enable_state() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    fixture.admitted_bundle();
    let enabled = fixture.state.get_skill_views_snapshot();
    let disabled = disabled_view(&enabled);
    assert!(
        active_aipp_package_in_view(&fixture.state, "fixture_view", &disabled)
            .unwrap()
            .is_none()
    );
    assert!(
        active_aipp_package(&fixture.state, "fixture_view")
            .unwrap()
            .is_some()
    );
}

#[test]
fn aipp_catalog_projects_one_captured_generation_and_refreshes_next_request() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    fixture.admitted_bundle();
    let captured = fixture.state.get_skill_views_snapshot();
    let disabled = disabled_view(&captured);
    install_view(&fixture, disabled.clone());

    let apps = project_aipp_catalog(&fixture.state, &captured);
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].skill_name, "fixture_view");
    assert!(project_aipp_catalog(&fixture.state, &disabled).is_empty());
    let headers = auth_headers(Some("aipp-view-admin"));
    let (status, Json(response)) = read_aipp_catalog(&fixture.state, &headers);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response.data.unwrap()["apps"], json!([]));
    install_view(&fixture, captured);
    let (status, Json(response)) = read_aipp_catalog(&fixture.state, &headers);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response.data.unwrap()["apps"].as_array().unwrap().len(), 1);
}

#[test]
fn aipp_bundle_binding_removal_cannot_downgrade_pinned_artifact_verification() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let install = fixture.admitted_bundle();
    let active = active_aipp_package(&fixture.state, "fixture_view")
        .unwrap()
        .unwrap();
    let original = fixture.state.get_skill_views_snapshot();
    fs::write(install.join("aipp/index.html"), b"tampered-after-metadata").unwrap();
    install_view(&fixture, disabled_view(&original));

    // The old current-view lookup would now take its no-binding trusted-source branch.
    assert!(
        fixture
            .state
            .get_skill_views_snapshot()
            .binding
            .admission_bindings
            .get("fixture_view")
            .is_none()
    );
    assert_eq!(
        verify_aipp_bundle_artifacts(&fixture.state, "fixture_view", &active),
        Err("aipp_install_receipt_invalid".into())
    );
    assert!(
        active_aipp_package(&fixture.state, "fixture_view")
            .unwrap()
            .is_none()
    );
}

#[test]
fn aipp_bundle_pointer_removal_cannot_be_hidden_by_a_new_unbound_view() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let install = fixture.admitted_bundle();
    let active = active_aipp_package(&fixture.state, "fixture_view")
        .unwrap()
        .unwrap();
    let original = fixture.state.get_skill_views_snapshot();
    let store = skill_sdk::InstallReceiptStore::new(skill_package_root(&fixture.state));
    assert!(store.deactivate_current("fixture_view").unwrap().is_some());
    install_view(&fixture, disabled_view(&original));

    assert_eq!(
        fs::read(install.join("aipp/index.html"))
            .unwrap()
            .as_slice(),
        BUNDLE
    );
    assert_eq!(
        verify_aipp_bundle_artifacts(&fixture.state, "fixture_view", &active),
        Err("aipp_install_receipt_invalid".into())
    );
}

#[test]
fn aipp_bundle_new_binding_cannot_repin_an_existing_metadata_result() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let install = fixture.admitted_bundle();
    let active = active_aipp_package(&fixture.state, "fixture_view")
        .unwrap()
        .unwrap();
    let original = fixture.state.get_skill_views_snapshot();
    let store = skill_sdk::InstallReceiptStore::new(skill_package_root(&fixture.state));
    let mut receipt = store
        .verified_current_install("fixture_view")
        .unwrap()
        .receipt;
    // Rotate only private fixture receipt evidence; no installation code or process runs.
    receipt.installed_at_unix += 1;
    store.write_receipt(&install, &receipt).unwrap();
    store.activate(&install, &receipt).unwrap();
    let mut bindings = original.binding.admission_bindings.as_ref().clone();
    bindings
        .get_mut("fixture_view")
        .unwrap()
        .install_receipt_digest = receipt.digest().unwrap();
    let mut replacement = crate::SkillViewsSnapshot {
        registry: original.registry.clone(),
        skills_list: original.skills_list.clone(),
        binding: Default::default(),
    };
    replacement.binding.registry_generation = original.binding.registry_generation + 1;
    replacement.binding.admission_bindings = Arc::new(bindings);
    install_view(&fixture, Arc::new(replacement));

    assert_eq!(
        verify_aipp_bundle_artifacts(&fixture.state, "fixture_view", &active),
        Err("aipp_generation_binding_mismatch".into())
    );
    let fresh = active_aipp_package(&fixture.state, "fixture_view")
        .unwrap()
        .unwrap();
    assert_eq!(
        verify_aipp_bundle_artifacts(&fixture.state, "fixture_view", &fresh),
        Ok(())
    );
}
