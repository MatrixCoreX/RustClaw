use super::*;
use axum::response::IntoResponse;
use std::time::{Duration, Instant};

#[path = "aipp_read_surfaces_tests/fixtures.rs"]
mod fixtures;
use fixtures::{Surface, SurfaceFixture, BUNDLE, PREVIEW, SURFACES};

fn auth_headers(key: Option<&'static str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(key) = key {
        headers.insert(
            claw_core::product_identity::AUTH_KEY_HEADER,
            axum::http::HeaderValue::from_static(key),
        );
    }
    headers
}

async fn request(
    surface: Surface,
    state: &AppState,
    key: Option<&'static str>,
    asset: &str,
) -> axum::response::Response {
    let headers = auth_headers(key);
    match surface {
        Surface::Catalog => get_aipp_catalog(State(state.clone()), headers)
            .await
            .into_response(),
        Surface::Preview => {
            get_aipp_media_preview(
                State(state.clone()),
                headers,
                AxumPath(("fixture_view".into(), 1)),
            )
            .await
        }
        Surface::Bundle => {
            get_aipp_bundle_asset(
                State(state.clone()),
                headers,
                AxumPath(("fixture_view".into(), asset.into())),
            )
            .await
        }
    }
}

async fn error_body(
    response: axum::response::Response,
    status: StatusCode,
    code: &str,
    auth: bool,
) {
    assert_eq!(response.status(), status);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    // Pin the existing auth boundary's distinct response contracts, not an empty payload.
    let data = if auth {
        match code {
            "auth_key_required" | "auth_key_invalid" => json!({
                "owner_layer": "ui_auth", "error_code": code, "status_code": code,
                "message_key": format!("clawd.ui.auth.{code}"),
            }),
            "admin_required" => json!({
                "error_code": code, "message_key": "clawd.ui.auth.admin_required",
            }),
            "auth_lookup_failed" => Value::Null,
            _ => panic!("unexpected auth contract: {code}"),
        }
    } else {
        json!({"error_code": code, "message_key": format!("clawd.ui.aipp.{code}")})
    };
    assert_eq!(body, json!({"ok": false, "error": code, "data": data}));
}

async fn pool_wait(key: &'static str, status: StatusCode, code: Option<&str>) {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        let held = fixture.state.core.db.get().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(held);
        });
        let start = Instant::now();
        let (response, timer_delay) =
            tokio::join!(request(surface, &fixture.state, Some(key), ""), async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                start.elapsed()
            },);
        release.join().unwrap();
        let scoped_catalog = matches!(surface, Surface::Catalog) && code == Some("admin_required");
        assert_eq!(
            response.status(),
            if scoped_catalog {
                StatusCode::OK
            } else {
                status
            },
            "{surface:?}"
        );
        eprintln!(
            "aipp_surface={surface:?} pool_hold_ms=500 timer_delay_ms={}",
            timer_delay.as_millis()
        );
        assert!(
            timer_delay < Duration::from_millis(400),
            "{surface:?}: {timer_delay:?}"
        );
        if let Some(code) = code.filter(|_| !scoped_catalog) {
            error_body(response, status, code, true).await;
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_keep_the_executor_responsive_during_admin_auth_pool_wait() {
    pool_wait("aipp-view-admin", StatusCode::OK, None).await;
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_keep_the_executor_responsive_during_invalid_auth_pool_wait() {
    pool_wait(
        "aipp-view-unknown",
        StatusCode::UNAUTHORIZED,
        Some("auth_key_invalid"),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_keep_the_executor_responsive_during_non_admin_auth_pool_wait() {
    pool_wait(
        "aipp-view-user",
        StatusCode::FORBIDDEN,
        Some("admin_required"),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn inline_aipp_catalog_control_demonstrates_executor_stall() {
    let fixture = SurfaceFixture::new(Surface::Catalog);
    let held = fixture.state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let start = Instant::now();
    let ((status, _), timer_delay) = tokio::join!(
        async { read_aipp_catalog(&fixture.state, &auth_headers(Some("aipp-view-admin"))) },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            start.elapsed()
        },
    );
    release.join().unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(timer_delay >= Duration::from_millis(500), "{timer_delay:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn all_aipp_read_surfaces_reject_missing_keys_without_waiting_for_the_database() {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        let _held = fixture.state.core.db.get().unwrap();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            request(surface, &fixture.state, None, ""),
        )
        .await
        .unwrap();
        error_body(
            response,
            StatusCode::UNAUTHORIZED,
            "auth_key_required",
            true,
        )
        .await;
    }
}

#[tokio::test]
async fn all_aipp_read_surfaces_recheck_roles_and_key_revocation_on_every_request() {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        assert_eq!(
            request(surface, &fixture.state, Some("aipp-view-admin"), "")
                .await
                .status(),
            StatusCode::OK
        );
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE auth_keys SET role='user' WHERE user_key='aipp-view-admin'",
                [],
            )
            .unwrap();
        let response = request(surface, &fixture.state, Some("aipp-view-admin"), "").await;
        if matches!(surface, Surface::Catalog) {
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["data"]["apps"], json!([]));
        } else {
            error_body(response, StatusCode::FORBIDDEN, "admin_required", true).await;
        }
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE auth_keys SET role='admin', enabled=0 WHERE user_key='aipp-view-admin'",
                [],
            )
            .unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::UNAUTHORIZED,
            "auth_key_invalid",
            true,
        )
        .await;
    }
}

#[tokio::test]
async fn all_aipp_read_surfaces_fail_closed_without_exposing_auth_database_errors() {
    for surface in SURFACES {
        let fixture = SurfaceFixture::new(surface);
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute("DROP TABLE auth_keys", [])
            .unwrap();
        error_body(
            request(surface, &fixture.state, Some("aipp-view-admin"), "").await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "auth_lookup_failed",
            true,
        )
        .await;
    }
}

#[path = "aipp_read_surfaces_tests/state.rs"]
mod state_and_streams;

#[path = "aipp_read_surfaces_tests/security.rs"]
mod security;

#[path = "aipp_read_surfaces_tests/generation.rs"]
mod generation;

#[tokio::test]
async fn transcript_aipp_binds_assets_and_keeps_history_account_owned() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let manifest_path = fixture.manifest_path();
    let mut manifest = skill_sdk::PackageManifest::load(&manifest_path).unwrap();
    manifest.aipp.as_mut().unwrap().data_contract = TRANSCRIPT_CONTRACT.to_owned();
    fs::write(&manifest_path, manifest.to_toml_string().unwrap()).unwrap();
    let state = &fixture.state;
    let release_view = state.get_skill_views_snapshot();
    let install = fixture.admitted_bundle();
    assert_eq!(
        active_aipp_package(state, "fixture_view").err().as_deref(),
        Some("aipp_bundle_requires_release_approval")
    );
    *state.core.skill_views_snapshot.write().unwrap() = release_view.clone();
    let user = auth_headers(Some("aipp-view-user"));
    let admin = auth_headers(Some("aipp-view-admin"));
    let version = manifest.package.version.clone();
    assert_eq!(
        prepare_aipp_transcript_frame(state, &user, "fixture_view", "0.0.0")
            .unwrap_err()
            .0,
        StatusCode::CONFLICT,
    );
    let frame = prepare_aipp_transcript_frame(state, &user, "fixture_view", &version).unwrap();
    let ticket: uuid::Uuid = serde_json::from_value(frame["script_ticket"].clone()).unwrap();
    assert_eq!(frame["html"], std::str::from_utf8(BUNDLE).unwrap());
    assert_eq!(
        redeem_aipp_transcript_script(state, "fixture_view", ticket).unwrap(),
        b"/* fixture script */"
    );
    assert!(redeem_aipp_transcript_script(state, "fixture_view", ticket).is_none());
    assert!(
        prepare_aipp_transcript_frame(state, &HeaderMap::new(), "fixture_view", &version).is_err()
    );

    let (status, _) = submit_aipp_transcript(
        State(state.clone()),
        user.clone(),
        AxumPath("fixture_view".into()),
        Json(AippTranscriptRequest {
            request_id: uuid::Uuid::new_v4(),
            package_version: version.clone(),
            attachment: json!({"name": "wrong.wav", "kind": "audio", "size": 16,
                                "mime_type": "audio/wav", "path": "other/owner.wav"}),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let identity = require_ui_identity(state, &user).unwrap();
    let task_id = uuid::Uuid::new_v4();
    {
        let db = state.core.db.get().unwrap();
        db.execute(
            "INSERT INTO tasks(task_id, user_id, chat_id, principal_id, channel, kind, payload_json,
                               status, result_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'ui', 'ask', ?5, 'succeeded', ?6, '100', '101')",
            rusqlite::params![
                task_id.to_string(), identity.user_id, identity.chat_id, identity.principal_id,
                json!({"entrypoint": TRANSCRIPT_ENTRYPOINT, "aipp_skill": "fixture_view",
                       "attachments": [{"name": "fixture.wav", "path": "private/upload"}]}).to_string(),
                json!({"text": "A test sentence.", "private_path": "private/export"}).to_string(),
            ],
        ).unwrap();
    }
    let (status, Json(list)) = list_aipp_transcripts(
        State(state.clone()),
        user.clone(),
        AxumPath("fixture_view".into()),
        Query(AippTranscriptQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list.data.unwrap()["items"],
        json!([{
            "task_id": task_id, "name": "fixture.wav", "status": "succeeded", "created_at": 100, "source": "aipp"
        }])
    );

    let read = |headers| {
        read_aipp_transcript(
            State(state.clone()),
            headers,
            AxumPath(("fixture_view".into(), task_id)),
        )
    };
    let (status, Json(result)) = read(user.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        result.data.unwrap(),
        json!({
            "task_id": task_id, "name": "fixture.wav", "status": "succeeded",
            "text": "A test sentence.", "error_code": null, "source": "aipp",
        })
    );
    assert_eq!(read(admin).await.0, StatusCode::NOT_FOUND);

    let service = aipp_admission_service(state).unwrap();
    service.set_aipp_installed("fixture_view", false).unwrap();
    assert_eq!(read(user.clone()).await.0, StatusCode::NOT_FOUND);
    service.set_aipp_installed("fixture_view", true).unwrap();
    assert_eq!(read(user.clone()).await.0, StatusCode::OK);
    fs::write(install.join("aipp/index.html"), b"tampered").unwrap();
    assert!(prepare_aipp_transcript_frame(state, &user, "fixture_view", &version).is_err());
}

#[tokio::test]
async fn aipp_history_reads_owned_agent_delivery_and_legacy_transcripts() {
    let fixture = SurfaceFixture::new(Surface::Bundle);
    let path = fixture.manifest_path();
    let mut manifest = skill_sdk::PackageManifest::load(&path).unwrap();
    manifest.aipp.as_mut().unwrap().data_contract = TRANSCRIPT_CONTRACT.into();
    fs::write(path, manifest.to_toml_string().unwrap()).unwrap();
    let state = &fixture.state;
    let release_view = state.get_skill_views_snapshot();
    fixture.admitted_bundle();
    *state.core.skill_views_snapshot.write().unwrap() = release_view;

    let user = auth_headers(Some("aipp-view-user"));
    let admin = auth_headers(Some("aipp-view-admin"));
    let identity = require_ui_identity(state, &user).unwrap();
    let workspace = state.identity_files_root(&identity).unwrap();
    let id = uuid::Uuid::new_v4();
    let text = "An actual retained transcript.\n";
    let artifact = crate::skill_output_artifact::publish_task_text_artifact(
        &workspace,
        &id.to_string(),
        "transcript-review",
        "transcript.txt",
        text,
        json!({"artifact_role":"transcript_text"}),
    )
    .unwrap();
    let artifacts =
        json!({"text":"Unrelated accompanying reply", "artifacts":[artifact.artifact_ref]});
    let result = crate::task_artifacts::materialize_scoped_task_artifacts(
        &workspace,
        &fixture.workspace.0,
        &id.to_string(),
        &artifacts.to_string(),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    let manifest =
        crate::task_artifacts::transcript_delivery_manifest(&id.to_string(), &result).unwrap();
    let delivered = crate::task_artifacts::validated_delivery_artifact_path(
        &fixture.workspace.0,
        &id.to_string(),
        &manifest,
    )
    .unwrap();
    let inline = uuid::Uuid::new_v4();
    let other = uuid::Uuid::new_v4();
    let legacy = json!({
        "text": "旧转写",
        "task_journal":{"trace":{"capability_results":[{
            "status":"ok",
            "data":{"extra":{"transcription_delivery":{"mode":"inline","character_count":3,"reviewed_by_model":true},
                             "transcription_review":{"schema_version":1,"raw_text":"原识别"}}}
        }]}}
    });
    {
        let db = state.core.db.get().unwrap();
        for (task, status, result) in [
            (id, "succeeded", result.clone()),
            (inline, "succeeded", legacy.clone()),
            (other, "succeeded", result),
            (uuid::Uuid::new_v4(), "failed", legacy),
            (
                uuid::Uuid::new_v4(),
                "succeeded",
                json!({"text": "transcript-review: is just a string."}),
            ),
        ] {
            db.execute(
                "INSERT INTO tasks(task_id,user_id,chat_id,principal_id,channel,kind,payload_json,
                                   status,result_json,created_at,updated_at)
                VALUES (?1,?2,?3,?4,'wechat','ask','{}',?5,?6,'100','101')",
                rusqlite::params![
                    task.to_string(),
                    identity.user_id,
                    identity.chat_id,
                    identity.principal_id,
                    status,
                    result.to_string()
                ],
            )
            .unwrap();
        }
    }

    let (status, Json(list)) = list_aipp_transcripts(
        State(state.clone()),
        user.clone(),
        AxumPath("fixture_view".into()),
        Query(AippTranscriptQuery::default()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list.data.unwrap()["items"],
        json!([
            {"task_id": inline, "status":"succeeded","created_at":100,"name":"transcript.txt","source":"agent"},
            {"task_id": id, "status":"succeeded","created_at":100,"name":"transcript.txt","source":"agent"}
        ])
    );

    let read = |headers, task| {
        read_aipp_transcript(
            State(state.clone()),
            headers,
            AxumPath(("fixture_view".into(), task)),
        )
    };
    assert_eq!(read(admin, id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(read(user.clone(), other).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        read(user.clone(), inline).await.1 .0.data.unwrap()["text"],
        "旧转写"
    );
    assert_eq!(
        read(user.clone(), id).await.1 .0.data.unwrap()["text"],
        text
    );

    fs::remove_file(delivered).unwrap();
    assert_eq!(
        read(user.clone(), id).await.1 .0.data.unwrap()["text"],
        text
    );
    let original = workspace.join(artifact.artifact_ref["path"].as_str().unwrap());
    fs::write(original, "corrupt transcript").unwrap();
    assert_eq!(read(user, id).await.0, StatusCode::GONE);
}
