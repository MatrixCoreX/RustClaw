use super::*;
#[path = "model_catalog_read_test_support.rs"]
mod support;
use support::*;
#[path = "model_catalog_read_http_tests.rs"]
mod http;
#[path = "model_catalog_read_offload_tests.rs"]
mod offload;

#[tokio::test]
async fn model_catalog_parse_failures_never_expose_required_or_optional_config_source() {
    for file in ["config", "image", "audio", "video", "music"] {
        let fixture = Fixture::new();
        fixture.write(
            &format!("configs/{file}.toml"),
            &format!("api_key = {SECRET}\n"),
        );
        // Negative control: the unchanged core error really carries source and
        // absolute path. The HTTP boundary must not serialize its Display.
        let raw = claw_core::model_catalog::build_model_catalog_from_workspace(&fixture.root)
            .unwrap_err()
            .to_string();
        assert!(raw.contains(SECRET));
        assert!(raw.contains(fixture.root.to_str().unwrap()));
        for key in [OWNER, ADMIN] {
            let response = fixture.catalog(key).await;
            assert_error(
                &response,
                StatusCode::INTERNAL_SERVER_ERROR,
                "model_catalog_unavailable",
            );
            assert_eq!(
                response.1["data"],
                json!({
                    "error_code": "model_catalog_unavailable",
                    "failure_code": "model_catalog_parse_failed",
                })
            );
            fixture.assert_private(&response.1);
            let trace = fixture.debug(key, TASK, true).await;
            assert_eq!(trace.0, StatusCode::OK);
            assert_eq!(
                trace.1["data"]["model_catalog_trace"],
                json!({
                    "trace_kind": "model_catalog_decision",
                    "status": "catalog_unavailable",
                    "error_code": "model_catalog_unavailable",
                    "failure_code": "model_catalog_parse_failed",
                })
            );
            fixture.assert_private(&trace.1);
        }
        // Also keep the producer safe before the teaching redactor runs.
        fixture.assert_private(&build_model_catalog_trace_for_debug(&fixture.state, &[]));
    }
}

#[tokio::test]
async fn model_catalog_read_failures_keep_server_error_and_hide_paths() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.root.join("configs/config.toml")).unwrap();
    for key in [OWNER, ADMIN] {
        let response = fixture.catalog(key).await;
        assert_error(
            &response,
            StatusCode::INTERNAL_SERVER_ERROR,
            "model_catalog_unavailable",
        );
        assert_eq!(
            response.1["data"]["failure_code"],
            "model_catalog_read_failed"
        );
        fixture.assert_private(&response.1);
    }
    fixture.write_config("fixture-model");
    // A directory is a deterministic cross-platform read failure, unlike mode
    // bits which behave differently under privileged CI users.
    fs::create_dir(fixture.root.join("configs/audio.toml")).unwrap();
    let response = fixture.catalog(OWNER).await;
    assert_error(
        &response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "model_catalog_unavailable",
    );
    assert_eq!(
        response.1["data"]["failure_code"],
        "model_catalog_read_failed"
    );
    fixture.assert_private(&response.1);
    let trace = fixture.debug(OWNER, TASK, true).await;
    assert_eq!(trace.0, StatusCode::OK);
    assert_eq!(
        trace.1["data"]["model_catalog_trace"]["failure_code"],
        "model_catalog_read_failed"
    );
    fixture.assert_private(&trace.1);
}

#[tokio::test]
async fn model_catalog_success_matches_core_projection_for_users_and_admins() {
    let fixture = Fixture::new();
    fixture.write(
        "logs/agent_parity_gate/fixture/chinese_model_catalog.json",
        r#"{"status":"ok","finding_count":0}"#,
    );
    let mut expected = serde_json::to_value(
        claw_core::model_catalog::build_model_catalog_from_workspace(&fixture.root).unwrap(),
    )
    .unwrap();
    expected["last_guard_status"] = read_model_catalog_guard_status(&fixture.root);
    for key in [OWNER, ADMIN] {
        let response = fixture.catalog(key).await;
        assert_eq!(response.0, StatusCode::OK);
        assert_eq!(
            response.1,
            json!({"ok":true, "data":expected, "error":null})
        );
        fixture.assert_private(&response.1);
        let trace = fixture.debug(key, TASK, true).await;
        assert_eq!(trace.0, StatusCode::OK);
        assert_eq!(
            trace.1["data"]["model_catalog_trace"],
            build_model_catalog_trace_for_debug(&fixture.state, &[])
        );
        fixture.assert_private(&trace.1);
    }
}

#[tokio::test]
async fn model_catalog_reads_fresh_configuration_and_recovers_without_caches() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.catalog(OWNER).await.1["data"]["selected_model"],
        "fixture-model"
    );
    fixture.write_config("fixture-replacement");
    assert_eq!(
        fixture.catalog(OWNER).await.1["data"]["selected_model"],
        "fixture-replacement"
    );
    fixture.write("configs/config.toml", "api_key = invalid\n");
    assert_eq!(
        fixture.catalog(OWNER).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    fixture.write_config("fixture-recovered");
    assert_eq!(
        fixture.catalog(OWNER).await.1["data"]["selected_model"],
        "fixture-recovered"
    );
    let guard = "logs/agent_parity_gate/fixture/chinese_model_catalog.json";
    fixture.write(guard, "invalid");
    assert_eq!(
        fixture.catalog(OWNER).await.1["data"]["last_guard_status"]["status"],
        "parse_failed"
    );
    fixture.write(guard, r#"{"status":"error","finding_count":2}"#);
    let response = fixture.catalog(OWNER).await;
    assert_eq!(response.1["data"]["last_guard_status"]["status"], "error");
    assert_eq!(response.1["data"]["last_guard_status"]["finding_count"], 2);
}

#[tokio::test]
async fn model_catalog_authentication_precedes_config_and_debug_requires_opt_in_and_owner() {
    let fixture = Fixture::new();
    fixture.write("configs/config.toml", &format!("api_key = {SECRET}\n"));
    for (key, code) in [
        ("", "auth_key_required"),
        ("fixture-catalog-unknown", "auth_key_invalid"),
    ] {
        let response = fixture.catalog(key).await;
        assert_error(&response, StatusCode::UNAUTHORIZED, code);
        assert_eq!(response.1["data"]["error_code"], code);
        let trace = fixture.debug(key, TASK, true).await;
        assert_error(&trace, StatusCode::UNAUTHORIZED, code);
    }
    for (key, task, opt_in, status, code) in [
        (
            OWNER,
            TASK,
            false,
            StatusCode::BAD_REQUEST,
            "teaching_trace_opt_in_required",
        ),
        (
            ADMIN,
            "  ",
            true,
            StatusCode::BAD_REQUEST,
            "task_id_required",
        ),
        (
            ADMIN,
            "missing-task",
            true,
            StatusCode::NOT_FOUND,
            "teaching_trace_task_not_found",
        ),
        (
            OTHER,
            TASK,
            true,
            StatusCode::FORBIDDEN,
            "teaching_trace_access_denied",
        ),
    ] {
        let trace = fixture.debug(key, task, opt_in).await;
        assert_error(&trace, status, code);
        fixture.assert_private(&trace.1);
    }
    assert_eq!(
        fixture.debug(OWNER, TASK, true).await.1["data"]["access"]["scope"],
        "task_owner"
    );
    assert_eq!(
        fixture.debug(ADMIN, TASK, true).await.1["data"]["access"]["scope"],
        "admin"
    );
}

#[tokio::test]
async fn model_catalog_and_debug_recheck_key_role_ownership_and_revocation() {
    let fixture = Fixture::new();
    assert_eq!(fixture.debug(ADMIN, TASK, true).await.0, StatusCode::OK);
    fixture.sql("UPDATE auth_keys SET role='user' WHERE user_key='fixture-catalog-admin'");
    assert_eq!(
        fixture.debug(ADMIN, TASK, true).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(fixture.catalog(ADMIN).await.0, StatusCode::OK);
    fixture.sql("UPDATE tasks SET user_key='fixture-catalog-other'");
    assert_eq!(
        fixture.debug(OWNER, TASK, true).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(fixture.debug(OTHER, TASK, true).await.0, StatusCode::OK);
    fixture.sql("UPDATE auth_keys SET enabled=0 WHERE user_key='fixture-catalog-other'");
    for response in [
        fixture.catalog(OTHER).await,
        fixture.debug(OTHER, TASK, true).await,
    ] {
        assert_error(&response, StatusCode::UNAUTHORIZED, "auth_key_invalid");
        assert_eq!(response.1["data"]["error_code"], "auth_key_invalid");
    }
}

#[tokio::test]
async fn model_catalog_auth_database_failure_never_becomes_invalid_key_or_catalog_output() {
    let fixture = Fixture::new();
    fixture.sql("DROP TABLE auth_keys");
    for response in [
        fixture.catalog(ADMIN).await,
        fixture.debug(ADMIN, TASK, true).await,
    ] {
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["ok"], false);
        assert!(response.1["data"].is_null());
        assert_ne!(response.1["error"], "auth_key_invalid");
        fixture.assert_private(&response.1);
        assert!(!response.1.to_string().contains("auth_keys"));
    }
}
