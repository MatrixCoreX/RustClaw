use super::*;
use skill_sdk::AdmissionState;

#[path = "skill_switch_update_offload_tests.rs"]
mod offload;

#[path = "skill_switch_revision_tests.rs"]
mod revision;

const ALPHA: &str = "switch_api_alpha";
const BETA: &str = "switch_api_beta";
const USER: &str = "switch-api-user";

struct Fixture {
    state: AppState,
    root: PathBuf,
    admission_root: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let (mut state, root) = isolated_skill_store_state();
        let root = root.canonicalize().unwrap();
        state.skill_rt.workspace_root = root.clone();
        state.reload_ctx.config_path_for_reload = root
            .join("configs/config.toml")
            .to_string_lossy()
            .into_owned();
        let admission_root = admission_service(&state).unwrap().root().to_path_buf();
        let fixture = Self {
            state,
            root,
            admission_root,
        };
        fixture.state.seed_test_auth_identity(USER, "user");
        for name in [ALPHA, BETA] {
            register_external_skill_fixture(&fixture.state, &fixture.root, name).await;
        }
        fixture
    }

    fn service(&self) -> crate::skill_admission::SkillAdmissionService {
        admission_service(&self.state).unwrap()
    }

    fn config(&self) -> Vec<u8> {
        std::fs::read(self.root.join("configs/config.toml")).unwrap()
    }

    async fn save(&self, key: &str, switches: Value) -> (StatusCode, Value) {
        let router = axum::Router::new()
            .nest("/v1", build_ui_router())
            .with_state(self.state.clone());
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/skills/config")
            .header("content-type", "application/json");
        if !key.is_empty() {
            request = request.header(claw_core::product_identity::AUTH_KEY_HEADER, key);
        }
        let response = router
            .oneshot(
                request
                    .body(Body::from(
                        serde_json::json!({"skill_switches":switches}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn skill_switch_update_tests_batch_commits_once_without_touching_startup_config_or_packages()
{
    let fixture = Fixture::new().await;
    let before = fixture.service().snapshot().unwrap();
    let config = fixture.config();
    let registry = std::fs::read(fixture.root.join("configs/skills_registry.toml")).unwrap();
    let packages = [ALPHA, BETA]
        .map(|name| std::fs::read(installed_package_pointer(&fixture.root, name)).unwrap());
    let response = fixture
        .save(STORE_TEST_KEY, serde_json::json!({ALPHA:false, BETA:false}))
        .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(
        response.1["data"]["registry_generation"],
        before.generation + 1
    );
    assert_eq!(response.1["data"]["restart_required"], false);
    assert_eq!(fixture.config(), config);
    assert_eq!(
        std::fs::read(fixture.root.join("configs/skills_registry.toml")).unwrap(),
        registry
    );
    for (index, name) in [ALPHA, BETA].into_iter().enumerate() {
        assert_eq!(response.1["data"]["skill_switches"][name], false);
        assert!(!fixture.state.get_skills_list().contains(&name.to_string()));
        assert_eq!(
            std::fs::read(installed_package_pointer(&fixture.root, name)).unwrap(),
            packages[index]
        );
    }
    reload_skill_views(&fixture.state).unwrap();
    assert_eq!(
        fixture.service().snapshot().unwrap().disabled,
        BTreeSet::from([ALPHA.to_string(), BETA.to_string()])
    );
}

#[tokio::test]
async fn skill_switch_update_tests_partial_request_preserves_unrelated_switches_and_state() {
    let fixture = Fixture::new().await;
    let config = fixture.config();
    let parsed: toml::Value = toml::from_str(std::str::from_utf8(&config).unwrap()).unwrap();
    let previous = super::super::collect_skill_switches(&parsed, &fixture.state);
    let response = fixture
        .save(STORE_TEST_KEY, serde_json::json!({ALPHA:false}))
        .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(fixture.config(), config);
    for (name, enabled) in previous {
        if name != ALPHA && name != BETA {
            assert_eq!(response.1["data"]["skill_switches"][&name], enabled);
        }
    }
    let after = fixture.service().snapshot().unwrap();
    assert_eq!(after.state(ALPHA), Some(AdmissionState::InstalledDisabled));
    assert_eq!(after.state(BETA), Some(AdmissionState::Enabled));
}

#[tokio::test]
async fn skill_switch_update_tests_authorization_precedes_private_file_parsing() {
    let fixture = Fixture::new().await;
    let private = "fixture_switch_private_secret";
    std::fs::write(
        fixture.root.join("configs/config.toml"),
        format!("key = {private}\n"),
    )
    .unwrap();
    let pointer = std::fs::read(fixture.service_root().join("current-generation.json")).unwrap();
    for (key, status) in [
        ("", StatusCode::UNAUTHORIZED),
        ("unknown", StatusCode::UNAUTHORIZED),
        (USER, StatusCode::FORBIDDEN),
    ] {
        let response = fixture.save(key, serde_json::json!({ALPHA:false})).await;
        assert_eq!(response.0, status);
        assert!(!response.1.to_string().contains(private));
    }
    assert_eq!(
        std::fs::read(fixture.service_root().join("current-generation.json")).unwrap(),
        pointer
    );
}

#[tokio::test]
async fn skill_switch_update_tests_configuration_and_admission_failures_remain_private() {
    for shape in ["malformed", "missing_registry", "bad_generation"] {
        let fixture = Fixture::new().await;
        let private = "fixture_switch_private_secret";
        let pointer =
            std::fs::read(fixture.service_root().join("current-generation.json")).unwrap();
        match shape {
            "malformed" => std::fs::write(
                fixture.root.join("configs/config.toml"),
                format!("key = {private}\n"),
            )
            .unwrap(),
            "missing_registry" => {
                std::fs::remove_file(fixture.root.join("configs/skills_registry.toml")).unwrap()
            }
            "bad_generation" => std::fs::write(
                fixture.service_root().join("current-generation.json"),
                format!("{{{private}"),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        let before = fixture.config();
        let response = fixture
            .save(STORE_TEST_KEY, serde_json::json!({ALPHA:false}))
            .await;
        assert_eq!(
            response,
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({"ok":false,"data":null,"error":"skills_config_read_failed"})
            )
        );
        assert!(!response
            .1
            .to_string()
            .contains(fixture.root.to_str().unwrap()));
        assert_eq!(fixture.config(), before);
        if shape != "bad_generation" {
            assert_eq!(
                std::fs::read(fixture.service_root().join("current-generation.json")).unwrap(),
                pointer
            );
        }
    }
}

impl Fixture {
    fn service_root(&self) -> &Path {
        &self.admission_root
    }
}

#[tokio::test]
async fn skill_switch_update_tests_reenable_requires_existing_grant_and_does_not_reinstall() {
    let fixture = Fixture::new().await;
    fixture
        .service()
        .set_state(BETA, AdmissionState::AwaitingPolicyApproval, None)
        .unwrap();
    let config = fixture.config();
    for enabled in [false, true] {
        let response = fixture
            .save(
                STORE_TEST_KEY,
                serde_json::json!({ALPHA:enabled, BETA:true}),
            )
            .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.1);
        let snapshot = fixture.service().snapshot().unwrap();
        assert_eq!(
            snapshot.state(ALPHA),
            Some(if enabled {
                AdmissionState::Enabled
            } else {
                AdmissionState::InstalledDisabled
            })
        );
        assert_eq!(
            snapshot.state(BETA),
            Some(AdmissionState::AwaitingPolicyApproval)
        );
        assert!(!snapshot.execution_bindings.contains_key(BETA));
        assert_eq!(response.1["data"]["skill_switches"][BETA], false);
        assert_eq!(fixture.config(), config);
    }
}
