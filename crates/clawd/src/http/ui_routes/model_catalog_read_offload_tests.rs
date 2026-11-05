use super::*;
use std::time::Instant;

async fn observe_pool_wait(debug: bool, key: &str, inline_control: bool) {
    let fixture = Fixture::new();
    let held = fixture.state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let (response, timer_delay) = tokio::join!(
        async {
            if inline_control {
                let (status, Json(response)) = if debug {
                    read_task_debug_detail(
                        &fixture.state,
                        &headers(key),
                        TASK,
                        TeachingTraceQuery {
                            teaching: Some(true),
                            ..Default::default()
                        },
                    )
                } else {
                    read_model_catalog(&fixture.state, &headers(key))
                };
                (status, serde_json::to_value(response).unwrap())
            } else if debug {
                fixture.debug(key, TASK, true).await
            } else {
                fixture.catalog(key).await
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
        if key == "fixture-catalog-unknown" {
            StatusCode::UNAUTHORIZED
        } else if debug && key == OTHER {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::OK
        }
    );
    eprintln!(
        "catalog_debug={debug} inline={inline_control} pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    if inline_control {
        assert!(timer_delay >= Duration::from_millis(500), "{timer_delay:?}");
    } else {
        assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_pool_wait_keeps_the_async_executor_responsive() {
    for key in [OWNER, ADMIN, "fixture-catalog-unknown"] {
        observe_pool_wait(false, key, false).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_debug_pool_wait_keeps_the_async_executor_responsive() {
    for key in [OWNER, ADMIN, OTHER, "fixture-catalog-unknown"] {
        observe_pool_wait(true, key, false).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_inline_pool_wait_control_exposes_executor_blocking() {
    for debug in [false, true] {
        observe_pool_wait(debug, OWNER, true).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn model_catalog_missing_credentials_do_not_wait_for_a_database_connection() {
    let fixture = Fixture::new();
    let _held = fixture.state.core.db.get().unwrap();
    for debug in [false, true] {
        let response = tokio::time::timeout(Duration::from_secs(2), async {
            if debug {
                fixture.debug("", TASK, true).await
            } else {
                fixture.catalog("").await
            }
        })
        .await
        .unwrap();
        assert_error(&response, StatusCode::UNAUTHORIZED, "auth_key_required");
    }
}

// A FIFO makes the filesystem wait deterministic on both native Unix targets.
// The writer has a bounded connect loop so an early route failure cannot leave
// a fixture thread hanging forever. These deadlines are test-only.
#[cfg(unix)]
async fn observe_config_read_wait(debug: bool, inline_control: bool) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let fixture = Fixture::new();
    let path = fixture.root.join("configs/config.toml");
    let contents = fs::read_to_string(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
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
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture FIFO writer failed: {error}"),
            }
        };
        let _ = started_tx.send(Instant::now());
        std::thread::sleep(Duration::from_millis(500));
        file.write_all(contents.as_bytes()).unwrap();
    });
    let (response, timer_delay) = tokio::join!(
        async {
            if inline_control {
                let (status, Json(response)) = read_model_catalog(&fixture.state, &headers(OWNER));
                (status, serde_json::to_value(response).unwrap())
            } else if debug {
                fixture.debug(OWNER, TASK, true).await
            } else {
                fixture.catalog(OWNER).await
            }
        },
        async {
            let started = started_rx.await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    writer.join().unwrap();
    assert_eq!(response.0, StatusCode::OK);
    eprintln!(
        "catalog_debug={debug} inline={inline_control} config_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    if inline_control {
        assert!(timer_delay >= Duration::from_millis(500), "{timer_delay:?}");
    } else {
        assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn model_catalog_config_filesystem_wait_keeps_the_async_executor_responsive() {
    observe_config_read_wait(false, false).await;
    observe_config_read_wait(true, false).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn model_catalog_inline_filesystem_control_exposes_executor_blocking() {
    observe_config_read_wait(false, true).await;
}
