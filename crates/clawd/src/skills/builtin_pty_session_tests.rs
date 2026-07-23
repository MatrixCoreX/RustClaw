use super::*;

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("rustclaw-pty-session-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn session_ids_are_strict_machine_tokens() {
    let workspace = TestWorkspace::new();
    assert!(session_dir(&workspace.root, "session-123_ok").is_ok());
    assert!(session_dir(&workspace.root, "../escape").is_err());
    assert!(session_dir(&workspace.root, "slash/value").is_err());
}

#[cfg(unix)]
#[test]
fn session_state_rejects_symlinked_runtime_roots() {
    use std::os::unix::fs::symlink;

    let workspace = TestWorkspace::new();
    let external =
        std::env::temp_dir().join(format!("rustclaw-pty-external-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&external).unwrap();
    symlink(&external, workspace.root.join(".rustclaw")).unwrap();
    let dir = session_dir(&workspace.root, "session-1").unwrap();

    let error = create_session_directories(&workspace.root, &dir).unwrap_err();
    assert_eq!(error.code, "pty_session_state_path_unsafe");
    assert!(!external.join("pty_sessions").exists());
    fs::remove_dir_all(external).unwrap();
}

#[test]
fn poll_returns_exact_cursor_pages() {
    let workspace = TestWorkspace::new();
    let dir = session_dir(&workspace.root, "session-1").unwrap();
    fs::create_dir_all(&dir).unwrap();
    let spec = PtyLaunchSpec {
        schema_version: 1,
        session_id: "session-1".to_string(),
        task_id: "task-1".to_string(),
        owner_user_id: 1,
        owner_chat_id: 2,
        owner_channel: "telegram".to_string(),
        program: "bash".to_string(),
        args: Vec::new(),
        cwd: workspace.root.display().to_string(),
        env_clear: false,
        env: BTreeMap::new(),
        rows: 24,
        cols: 80,
        created_at: 1,
        expires_at: i64::MAX,
        idle_timeout_seconds: 300,
        max_output_bytes: 1024 * 1024,
    };
    let metadata = json!({
        "status": "running",
        "heartbeat_at": crate::now_ts_u64(),
        "output_bytes": 0,
        "rows": 24,
        "cols": 80,
    });
    atomic_write_json(&dir.join("metadata.json"), &metadata).unwrap();
    let source = "line-中文\n".repeat(100);
    fs::write(dir.join("output.bin"), &source).unwrap();

    let first = poll_session(
        &dir,
        &spec,
        json!({"cursor": 0, "max_bytes": 256}).as_object().unwrap(),
    )
    .unwrap();
    let first: Value = serde_json::from_str(&first).unwrap();
    let cursor = first["page"]["next_cursor"].as_u64().unwrap();
    let second = poll_session(
        &dir,
        &spec,
        json!({"cursor": cursor, "max_bytes": 256})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    let second: Value = serde_json::from_str(&second).unwrap();
    assert_eq!(first["page"]["end_byte"], second["page"]["start_byte"]);
    let joined = format!(
        "{}{}",
        first["content"].as_str().unwrap(),
        second["content"].as_str().unwrap()
    );
    assert_eq!(
        joined.as_bytes(),
        &source.as_bytes()[..second["page"]["end_byte"].as_u64().unwrap() as usize]
    );
}

#[test]
fn owner_binding_uses_actor_chat_and_channel() {
    let spec = PtyLaunchSpec {
        schema_version: 1,
        session_id: "session".to_string(),
        task_id: "task".to_string(),
        owner_user_id: 7,
        owner_chat_id: 9,
        owner_channel: "telegram".to_string(),
        program: "bash".to_string(),
        args: Vec::new(),
        cwd: ".".to_string(),
        env_clear: false,
        env: BTreeMap::new(),
        rows: 24,
        cols: 80,
        created_at: 1,
        expires_at: 2,
        idle_timeout_seconds: 30,
        max_output_bytes: 1024 * 1024,
    };
    let matching = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "next-task".to_string(),
        user_id: 7,
        chat_id: 9,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut mismatched = matching.clone();
    mismatched.chat_id = 10;

    assert!(ensure_owner(Some(&matching), &spec).is_ok());
    assert!(ensure_owner(Some(&mismatched), &spec).is_err());
}

#[tokio::test]
async fn durable_runner_accepts_input_and_exposes_resumable_output() {
    let workspace = TestWorkspace::new();
    let mut command = tokio::process::Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg("printf 'ready\\n'; read line; printf 'got:%s\\n' \"$line\"")
        .current_dir(&workspace.root);

    let started = start_session(&workspace.root, None, &command, 24, 80, 30, 15, 1024 * 1024)
        .await
        .expect("start session");
    let started: Value = serde_json::from_str(&started).unwrap();
    let session_id = started["session_id"].as_str().unwrap().to_string();

    let mut cursor = 0_u64;
    let mut transcript = String::new();
    for _ in 0..80 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id, "cursor": cursor})
                .as_object()
                .unwrap(),
        )
        .await
        .expect("poll ready");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        transcript.push_str(polled["content"].as_str().unwrap_or_default());
        cursor = polled["page"]["end_byte"].as_u64().unwrap_or(cursor);
        if transcript.contains("ready") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(transcript.contains("ready"));

    let resized = execute_existing_session_action(
        &workspace.root,
        None,
        "terminal_resize",
        json!({
            "session_id": session_id,
            "rows": 40,
            "cols": 120,
        })
        .as_object()
        .unwrap(),
    )
    .await
    .expect("resize terminal");
    let resized: Value = serde_json::from_str(&resized).unwrap();
    assert_eq!(resized["status"], "ok");
    assert_eq!(resized["data"]["rows"], 40);
    assert_eq!(resized["data"]["cols"], 120);

    let written = execute_existing_session_action(
        &workspace.root,
        None,
        "terminal_write",
        json!({
            "session_id": session_id,
            "data": "hello",
            "append_newline": true,
        })
        .as_object()
        .unwrap(),
    )
    .await
    .expect("write input");
    let written: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(written["status"], "ok");

    for _ in 0..80 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id, "cursor": cursor})
                .as_object()
                .unwrap(),
        )
        .await
        .expect("poll result");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        transcript.push_str(polled["content"].as_str().unwrap_or_default());
        cursor = polled["page"]["end_byte"].as_u64().unwrap_or(cursor);
        if polled["status"] != "running" && transcript.contains("got:hello") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(transcript.contains("got:hello"), "{transcript:?}");
}

#[tokio::test]
async fn durable_runner_can_be_terminated_through_machine_control() {
    let workspace = TestWorkspace::new();
    let mut command = tokio::process::Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg("printf 'waiting\\n'; sleep 30")
        .current_dir(&workspace.root);

    let started = start_session(&workspace.root, None, &command, 24, 80, 30, 15, 1024 * 1024)
        .await
        .expect("start session");
    let started: Value = serde_json::from_str(&started).unwrap();
    let session_id = started["session_id"].as_str().unwrap().to_string();

    let terminated = execute_existing_session_action(
        &workspace.root,
        None,
        "terminal_terminate",
        json!({"session_id": session_id}).as_object().unwrap(),
    )
    .await
    .expect("terminate session");
    let terminated: Value = serde_json::from_str(&terminated).unwrap();
    assert_eq!(terminated["status"], "ok");
    assert_eq!(terminated["data"]["termination_requested"], true);

    for _ in 0..80 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id}).as_object().unwrap(),
        )
        .await
        .expect("poll terminated session");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        if polled["status"] != "running" {
            assert_eq!(polled["reason_code"], "pty_terminated");
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("PTY session did not terminate");
}

#[tokio::test]
async fn durable_runner_delivers_signals_to_the_terminal_process_group() {
    let workspace = TestWorkspace::new();
    let mut command = tokio::process::Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg(
            "trap 'printf \"interrupted\\n\"; exit 0' INT; \
             printf 'waiting\\n'; while :; do sleep 1; done",
        )
        .current_dir(&workspace.root);

    let started = start_session(&workspace.root, None, &command, 24, 80, 30, 15, 1024 * 1024)
        .await
        .expect("start session");
    let started: Value = serde_json::from_str(&started).unwrap();
    let session_id = started["session_id"].as_str().unwrap().to_string();
    let mut ready = false;
    for _ in 0..80 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id}).as_object().unwrap(),
        )
        .await
        .expect("poll signal readiness");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        if polled["content"]
            .as_str()
            .is_some_and(|content| content.contains("waiting"))
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(ready, "signal fixture did not become ready");

    let signaled = execute_existing_session_action(
        &workspace.root,
        None,
        "terminal_signal",
        json!({"session_id": session_id, "signal": "INT"})
            .as_object()
            .unwrap(),
    )
    .await
    .expect("signal session");
    let signaled: Value = serde_json::from_str(&signaled).unwrap();
    assert_eq!(signaled["status"], "ok");
    assert_eq!(signaled["data"]["signal"], "INT");

    let mut transcript = String::new();
    for _ in 0..120 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id}).as_object().unwrap(),
        )
        .await
        .expect("poll signaled session");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        transcript = polled["content"].as_str().unwrap_or_default().to_string();
        if polled["status"] != "running" {
            assert!(transcript.contains("interrupted"), "{transcript:?}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("PTY session did not react to SIGINT: {transcript:?}");
}

#[tokio::test]
async fn durable_runner_enforces_hard_expiry_independently() {
    let workspace = TestWorkspace::new();
    let mut command = tokio::process::Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg("while :; do printf '.'; sleep 0.1; done")
        .current_dir(&workspace.root);

    let started = start_session(&workspace.root, None, &command, 24, 80, 1, 30, 1024 * 1024)
        .await
        .expect("start session");
    let started: Value = serde_json::from_str(&started).unwrap();
    let session_id = started["session_id"].as_str().unwrap().to_string();

    for _ in 0..120 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id}).as_object().unwrap(),
        )
        .await
        .expect("poll expiring session");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        if polled["status"] == "expired" {
            assert_eq!(polled["reason_code"], "pty_hard_timeout");
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("PTY session did not enforce its hard expiry");
}

#[tokio::test]
async fn durable_runner_enforces_the_total_output_budget() {
    let workspace = TestWorkspace::new();
    let mut command = tokio::process::Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg("yes bounded-output")
        .current_dir(&workspace.root);

    let started = start_session(&workspace.root, None, &command, 24, 80, 30, 15, 4_096)
        .await
        .expect("start session");
    let started: Value = serde_json::from_str(&started).unwrap();
    let session_id = started["session_id"].as_str().unwrap().to_string();

    for _ in 0..120 {
        let polled = execute_existing_session_action(
            &workspace.root,
            None,
            "terminal_poll",
            json!({"session_id": session_id, "max_bytes": 8192})
                .as_object()
                .unwrap(),
        )
        .await
        .expect("poll bounded session");
        let polled: Value = serde_json::from_str(&polled).unwrap();
        if polled["status"] != "running" {
            assert_eq!(polled["status"], "failed");
            assert_eq!(polled["reason_code"], "pty_output_limit");
            assert!(polled["output_bytes"].as_u64().unwrap() <= 4_096);
            let output_path = session_dir(&workspace.root, &session_id)
                .unwrap()
                .join("output.bin");
            assert!(fs::metadata(output_path).unwrap().len() <= 4_096);
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("PTY session did not enforce its output budget");
}

#[tokio::test]
async fn durable_runner_rejects_tampered_control_request_ids() {
    let workspace = TestWorkspace::new();
    let mut command = tokio::process::Command::new("/bin/bash");
    command
        .arg("-lc")
        .arg("sleep 30")
        .current_dir(&workspace.root);

    let started = start_session(&workspace.root, None, &command, 24, 80, 30, 15, 1024 * 1024)
        .await
        .expect("start session");
    let started: Value = serde_json::from_str(&started).unwrap();
    let session_id = started["session_id"].as_str().unwrap().to_string();
    let dir = session_dir(&workspace.root, &session_id).unwrap();
    let control_path = dir.join("controls/tampered.json");
    fs::write(
        &control_path,
        json!({
            "schema_version": 1,
            "request_id": "../../escaped",
            "action": "terminal_write",
            "data": "should-not-run"
        })
        .to_string(),
    )
    .unwrap();

    for _ in 0..40 {
        if !control_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!control_path.exists());
    assert!(!dir.join("escaped.json").exists());

    execute_existing_session_action(
        &workspace.root,
        None,
        "terminal_terminate",
        json!({"session_id": session_id}).as_object().unwrap(),
    )
    .await
    .expect("terminate session");
}
