use super::*;
use std::time::Instant;

fn operation(index: usize) -> (Method, &'static str, Option<Value>) {
    match index {
        0 => (Method::GET, MODELS, None),
        1 => (Method::GET, KEYS, None),
        2 => (Method::POST, MODELS, Some(json!({}))),
        3 => (Method::POST, KEYS, Some(json!({}))),
        _ => panic!("fixture operation invalid"),
    }
}

fn inline_read(fixture: &Fixture, index: usize) -> (StatusCode, Value) {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static(ADMIN),
    );
    assert!(require_ui_identity(&fixture.state, &headers).is_ok());
    let data = if index == 0 {
        serde_json::to_value(read_model_config(&fixture.state).unwrap()).unwrap()
    } else {
        serde_json::to_value(read_provider_keys(&fixture.state).unwrap()).unwrap()
    };
    (StatusCode::OK, json!({"ok":true,"data":data,"error":null}))
}

async fn observe_pool(index: usize, key: &str, inline: bool) {
    let fixture = Fixture::new();
    let held = fixture.state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let (response, delay) = tokio::join!(
        async {
            if inline {
                inline_read(&fixture, index)
            } else {
                let (method, path, body) = operation(index);
                fixture.api(key, method, path, body).await
            }
        },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    release.join().unwrap();
    assert_eq!(
        response.0,
        if key == "fixture-settings-unknown" {
            StatusCode::UNAUTHORIZED
        } else if index >= 2 && key == USER {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::OK
        }
    );
    eprintln!(
        "settings_operation={index} inline={inline} pool_hold_ms=500 timer_delay_ms={}",
        delay.as_millis()
    );
    if inline {
        assert!(delay >= Duration::from_millis(500), "{delay:?}");
    } else {
        assert!(delay < Duration::from_millis(400), "{delay:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_settings_all_routes_keep_async_executor_responsive_during_auth_pool_waits() {
    for index in 0..4 {
        for key in [ADMIN, USER, "fixture-settings-unknown"] {
            observe_pool(index, key, false).await;
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_settings_inline_read_control_exposes_pool_blocking() {
    for index in 0..2 {
        observe_pool(index, ADMIN, true).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_settings_missing_credentials_do_not_need_an_available_database() {
    let fixture = Fixture::new();
    let _held = fixture.state.core.db.get().unwrap();
    for index in 0..4 {
        let (method, path, body) = operation(index);
        let response =
            tokio::time::timeout(Duration::from_secs(2), fixture.api("", method, path, body))
                .await
                .unwrap();
        assert_error(&response, StatusCode::UNAUTHORIZED, "auth_key_required");
    }
}

#[cfg(unix)]
async fn observe_file(index: usize, inline: bool) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let fixture = Fixture::new();
    let path = fixture.root.join("configs/config.toml");
    let contents = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(StdCommand::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut file = loop {
            match fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(file) => break file,
                Err(error)
                    if error.raw_os_error() == Some(libc::ENXIO) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fixture FIFO connect failed: {error}"),
            }
        };
        let _ = tx.send(Instant::now());
        std::thread::sleep(Duration::from_millis(500));
        file.write_all(&contents).unwrap();
    });
    let (response, delay) = tokio::join!(
        async {
            if inline {
                inline_read(&fixture, index)
            } else {
                let (method, path, body) = operation(index);
                fixture.api(ADMIN, method, path, body).await
            }
        },
        async {
            let started = rx.await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    writer.join().unwrap();
    assert_eq!(response.0, StatusCode::OK);
    eprintln!(
        "settings_operation={index} inline={inline} config_hold_ms=500 timer_delay_ms={}",
        delay.as_millis()
    );
    if inline {
        assert!(delay >= Duration::from_millis(500), "{delay:?}");
    } else {
        assert!(delay < Duration::from_millis(400), "{delay:?}");
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn model_settings_both_reads_keep_async_executor_responsive_during_slow_filesystem_io() {
    observe_file(0, false).await;
    observe_file(1, false).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn model_settings_inline_read_control_exposes_filesystem_blocking() {
    observe_file(0, true).await;
}
