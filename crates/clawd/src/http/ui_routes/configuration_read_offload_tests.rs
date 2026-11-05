use super::*;
use std::time::Instant;

async fn observe_pool(index: usize, key: &str, inline_control: bool) {
    let fixture = read_fixture();
    let held = fixture.state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let (response, delay) = tokio::join!(
        async {
            if inline_control {
                inline(&fixture, index, key)
            } else {
                fixture.api(key, Method::GET, ROUTES[index].0, None).await
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
        if key == "fixture-unknown" {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::OK
        }
    );
    eprintln!(
        "configuration_read={index} inline={inline_control} pool_hold_ms=500 timer_delay_ms={}",
        delay.as_millis()
    );
    assert!(
        if inline_control {
            delay >= Duration::from_millis(500)
        } else {
            delay < Duration::from_millis(400)
        },
        "{delay:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn configuration_reads_keep_executor_responsive_for_all_roles_during_pool_waits() {
    for index in 0..2 {
        for key in [ADMIN, USER, "fixture-unknown"] {
            observe_pool(index, key, false).await;
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn configuration_reads_inline_controls_expose_auth_pool_blocking() {
    for index in 0..2 {
        observe_pool(index, ADMIN, true).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn configuration_reads_missing_credentials_do_not_wait_for_the_database() {
    let fixture = read_fixture();
    let _held = fixture.state.core.db.get().unwrap();
    for (path, _) in ROUTES {
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            fixture.api("", Method::GET, path, None),
        )
        .await
        .unwrap();
        assert_error(&response, StatusCode::UNAUTHORIZED, "auth_key_required");
    }
}

#[cfg(unix)]
async fn observe_file(index: usize, inline_control: bool) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let fixture = read_fixture();
    // The model read waits on its active config. The skill read waits later,
    // during admission registry hashing, proving that preparation is not split.
    let filename = if index == 0 {
        "config.toml"
    } else {
        "skills_registry.toml"
    };
    let path = fixture.root.join("configs").join(filename);
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
            if inline_control {
                inline(&fixture, index, ADMIN)
            } else {
                fixture.api(ADMIN, Method::GET, ROUTES[index].0, None).await
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
        "configuration_read={index} inline={inline_control} config_hold_ms=500 timer_delay_ms={}",
        delay.as_millis()
    );
    assert!(
        if inline_control {
            delay >= Duration::from_millis(500)
        } else {
            delay < Duration::from_millis(400)
        },
        "{delay:?}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn configuration_reads_keep_executor_responsive_during_config_and_admission_io() {
    for index in 0..2 {
        observe_file(index, false).await;
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn configuration_reads_inline_controls_expose_config_and_admission_io_blocking() {
    for index in 0..2 {
        observe_file(index, true).await;
    }
}
