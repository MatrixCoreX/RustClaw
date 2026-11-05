use super::*;
use crate::http::ui_routes::{write_skills_config, UpdateSkillsConfigRequest};
use axum::http::HeaderValue;
use std::time::{Duration, Instant};

async fn observe_admission_lock(inline: bool) {
    let fixture = Fixture::new().await;
    let revision = fixture
        .observed_revision()
        .await
        .as_str()
        .unwrap()
        .to_string();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.service_root().join("mutation.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        fs2::FileExt::unlock(&lock).unwrap();
    });
    let start = Instant::now();
    let (response, delay) = tokio::join!(
        async {
            if inline {
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(
                    claw_core::product_identity::AUTH_KEY_HEADER,
                    HeaderValue::from_static(STORE_TEST_KEY),
                );
                let (status, axum::Json(body)) = write_skills_config(
                    axum::extract::State(fixture.state.clone()),
                    headers,
                    axum::Json(UpdateSkillsConfigRequest {
                        skill_switches: [(ALPHA.to_string(), false)].into(),
                        expected_revision: Some(revision),
                    }),
                );
                (status, serde_json::to_value(body).unwrap())
            } else {
                fixture
                    .save(STORE_TEST_KEY, serde_json::json!({ALPHA:false}))
                    .await
            }
        },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            start.elapsed()
        }
    );
    release.join().unwrap();
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    eprintln!(
        "skill_switch_update inline={inline} lock_hold_ms=500 timer_delay_ms={}",
        delay.as_millis()
    );
    assert!(
        if inline {
            delay >= Duration::from_millis(500)
        } else {
            delay < Duration::from_millis(400)
        },
        "{delay:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn skill_switch_update_tests_executor_is_responsive_during_admission_lock_contention() {
    observe_admission_lock(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn skill_switch_update_tests_inline_control_exposes_admission_lock_blocking() {
    observe_admission_lock(true).await;
}
