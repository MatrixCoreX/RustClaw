#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[test]
fn pty_chat_completes_coding_thread_with_background_resume_and_review() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || run_mock_clawd(listener));

    let session_store = std::env::temp_dir().join(format!(
        "clawcli_pty_chat_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let transcript_path = session_store.with_extension("transcript");
    let base_url = format!("http://{address}");
    let transcript = transcript_path.to_str().expect("transcript path");
    let mut pty_command = Command::new("script");
    #[cfg(target_os = "macos")]
    pty_command.args([
        "-q",
        transcript,
        env!("CARGO_BIN_EXE_clawcli"),
        "--base-url",
        &base_url,
        "--key",
        "test-key",
        "chat",
        "--new",
    ]);
    #[cfg(not(target_os = "macos"))]
    pty_command.args([
        "-qefc",
        &format!(
            "{} --base-url {} --key test-key chat --new",
            env!("CARGO_BIN_EXE_clawcli"),
            base_url
        ),
        transcript,
    ]);
    let mut child = pty_command
        .env("APP_CLAWCLI_SESSION_STORE", &session_store)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn PTY chat");
    let mut stdin = child.stdin.take().expect("PTY stdin");
    for line in [
        "inspect workspace",
        "/approve-scope",
        "update one file",
        "run focused tests",
        "/continue",
        "correct the failing test",
        "review the diff",
        "finish with verification",
        "/exit",
    ] {
        writeln!(stdin, "{line}").expect("write PTY turn");
        stdin.flush().expect("flush PTY turn");
        thread::sleep(Duration::from_millis(700));
    }
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll PTY chat") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let transcript = std::fs::read_to_string(&transcript_path).unwrap_or_default();
            panic!("PTY chat did not finish: {transcript}");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let stdout = std::fs::read_to_string(&transcript_path).expect("read PTY transcript");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("PTY stderr")
        .read_to_string(&mut stderr)
        .expect("read PTY stderr");
    server.join().expect("mock clawd");
    let _ = std::fs::remove_file(session_store);
    let _ = std::fs::remove_file(transcript_path);

    assert!(status.success(), "stdout={stdout}\nstderr={stderr}");
    for task_id in ["task-1", "task-2", "task-3", "task-4", "task-5", "task-6"] {
        assert!(stdout.contains(&format!("task_id={task_id}")), "{stdout}");
    }
    assert!(stdout.contains("approval_scope_grant_created"), "{stdout}");
    assert!(stdout.contains("task_resume_requested"), "{stdout}");
    assert!(stdout.contains("checkpoint-coding-3"), "{stdout}");
    assert_eq!(stdout.matches("turn-6-complete").count(), 1, "{stdout}");
}

#[test]
fn permission_commands_list_and_revoke_backend_scope_grants() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        let list = accept_request(&listener);
        assert_eq!(list.path, "/v1/tasks/approval-grants");
        respond_json(
            list.stream,
            &json!({
                "ok": true,
                "data": {
                    "schema_version": 1,
                    "count": 1,
                    "grants": [{
                        "grant_id": "scope-grant-1",
                        "scope_kind": "session",
                        "scope_fingerprint": "sha256:scope",
                        "scope": {"entries": []},
                        "channel": "ui",
                        "chat_id": 7,
                        "issued_at": 100,
                        "expires_at": 200,
                        "revoked_at": null,
                        "use_count": 0,
                        "last_used_at": null,
                        "source_task_id": "task-1"
                    }]
                }
            }),
        );

        let revoke = accept_request(&listener);
        assert_eq!(revoke.path, "/v1/tasks/approval-grants/revoke");
        assert_eq!(
            parse_json_body(&revoke)["grant_id"],
            Value::String("scope-grant-1".to_string())
        );
        respond_json(
            revoke.stream,
            &json!({
                "ok": true,
                "data": {
                    "schema_version": 1,
                    "status": "approval_scope_grant_revoked",
                    "grant_id": "scope-grant-1"
                }
            }),
        );
    });

    let list = Command::new(env!("CARGO_BIN_EXE_clawcli"))
        .args([
            "--base-url",
            &format!("http://{address}"),
            "--key",
            "test-key",
            "permission",
            "grants",
            "--json",
        ])
        .output()
        .expect("run permission grants");
    assert!(
        list.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );
    let list_body: Value = serde_json::from_slice(&list.stdout).expect("list JSON");
    assert_eq!(list_body["grants"][0]["grant_id"], "scope-grant-1");

    let revoke = Command::new(env!("CARGO_BIN_EXE_clawcli"))
        .args([
            "--base-url",
            &format!("http://{address}"),
            "--key",
            "test-key",
            "permission",
            "revoke",
            "scope-grant-1",
            "--json",
        ])
        .output()
        .expect("run permission revoke");
    assert!(
        revoke.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&revoke.stdout),
        String::from_utf8_lossy(&revoke.stderr)
    );
    let revoke_body: Value = serde_json::from_slice(&revoke.stdout).expect("revoke JSON");
    assert_eq!(revoke_body["status"], "approval_scope_grant_revoked");
    server.join().expect("mock clawd");
}

#[test]
fn non_tty_plain_chat_exposes_content_before_terminal_and_does_not_repeat_it() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let (delta_sent_tx, delta_sent_rx) = mpsc::channel();
    let (allow_terminal_tx, allow_terminal_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let submit = accept_request(&listener);
        assert_eq!(submit.path, "/v1/tasks");
        respond_json(submit.stream, &task_submit_response("task-stream"));

        let request = accept_request(&listener);
        assert_eq!(request.path, "/v1/tasks/task-stream/events?cursor=0");
        let mut stream = request.stream;
        write_sse_headers(&mut stream);
        let events = assistant_presentation_events("task-stream", "streamed answer");
        write_sse_events(&mut stream, &events[..2]);
        delta_sent_tx.send(()).expect("signal public delta");
        allow_terminal_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("allow terminal events");
        write_sse_events(&mut stream, &events[2..]);
        drop(stream);

        let status = accept_request(&listener);
        assert_eq!(status.path, "/v1/tasks/task-stream");
        respond_json(
            status.stream,
            &task_status_response(
                "task-stream",
                "succeeded",
                "completed",
                Some("streamed answer"),
            ),
        );
    });

    let session_store = temporary_session_store("plain_stream");
    let mut child = spawn_chat(address, &session_store, false);
    let mut stdin = child.stdin.take().expect("plain stdin");
    write!(stdin, "stream a response\n/exit\n").expect("write plain turns");
    drop(stdin);
    let mut stdout = child.stdout.take().expect("plain stdout");
    let (chunk_tx, chunk_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut all = Vec::new();
        let mut buffer = [0_u8; 256];
        loop {
            let count = stdout.read(&mut buffer).expect("read plain stdout");
            if count == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..count]);
            chunk_tx.send(all.clone()).expect("send accumulated stdout");
        }
        all
    });

    delta_sent_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("server emitted public delta");
    let deadline = Instant::now() + Duration::from_secs(5);
    let output_before_terminal = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let output = chunk_rx
            .recv_timeout(remaining)
            .expect("client flushes public delta");
        if String::from_utf8_lossy(&output).contains("streamed answer") {
            break output;
        }
    };
    assert!(
        child.try_wait().expect("poll plain chat").is_none(),
        "task completed before terminal events were released"
    );
    assert!(
        String::from_utf8_lossy(&output_before_terminal).contains("streamed answer"),
        "public content was not visible before task_final"
    );

    allow_terminal_tx.send(()).expect("release terminal events");
    let status = child.wait().expect("wait plain chat");
    let stdout = String::from_utf8(reader.join().expect("plain stdout reader")).expect("UTF-8");
    server.join().expect("mock clawd");
    let _ = std::fs::remove_file(&session_store);

    assert!(status.success(), "{stdout}");
    assert_eq!(stdout.matches("streamed answer").count(), 1, "{stdout}");
    assert!(stdout.contains("task_id=task-stream"), "{stdout}");
    assert!(stdout.contains("status: succeeded"), "{stdout}");
    assert!(!stdout.contains("\u{1b}["), "{stdout}");
}

#[test]
fn non_tty_jsonl_chat_is_a_closed_one_object_per_line_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        let submit = accept_request(&listener);
        assert_eq!(submit.path, "/v1/tasks");
        respond_json(submit.stream, &task_submit_response("task-jsonl"));

        let stream = accept_request(&listener);
        assert_eq!(stream.path, "/v1/tasks/task-jsonl/events?cursor=0");
        respond_sse(
            stream.stream,
            &assistant_presentation_events("task-jsonl", "machine answer"),
        );
        let status = accept_request(&listener);
        assert_eq!(status.path, "/v1/tasks/task-jsonl");
        respond_json(
            status.stream,
            &task_status_response(
                "task-jsonl",
                "succeeded",
                "completed",
                Some("machine answer"),
            ),
        );
    });

    let session_store = temporary_session_store("jsonl_stream");
    let mut child = spawn_chat(address, &session_store, true);
    let mut stdin = child.stdin.take().expect("JSONL stdin");
    write!(stdin, "stream a response\n/exit\n").expect("write JSONL turns");
    drop(stdin);
    let output = child.wait_with_output().expect("wait JSONL chat");
    server.join().expect("mock clawd");
    let _ = std::fs::remove_file(&session_store);

    let stdout = String::from_utf8(output.stdout).expect("JSONL UTF-8");
    let records = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("one JSON object per line"))
        .collect::<Vec<_>>();
    assert!(output.status.success(), "{stdout}");
    assert_eq!(records.first().unwrap()["record_type"], "chat_session");
    assert!(records.iter().any(|value| {
        value["record_type"] == "task_submitted" && value["task_id"] == "task-jsonl"
    }));
    assert!(records
        .iter()
        .any(|value| value["event_type"] == "assistant_output_delta"));
    assert!(records
        .iter()
        .any(|value| value["event_type"] == "task_final"));
    assert_eq!(records.last().unwrap()["record_type"], "task_status");
    assert!(records.iter().all(|value| value.is_object()));
    assert!(!stdout.contains("\u{1b}["));
}

#[test]
fn conversation_and_attachments_survive_process_restart_then_clear_after_submit() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        let submit = accept_request(&listener);
        assert_eq!(submit.path, "/v1/tasks");
        let body = parse_json_body(&submit);
        assert_eq!(body["payload"]["conversation_id"], "conversation-golden");
        assert_eq!(body["payload"]["session_id"], "conversation-golden");
        assert_eq!(body["payload"]["text"], "use persisted context");
        let attachments = body["payload"]["attachments"]
            .as_array()
            .expect("persisted attachments");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0]["kind"], "file");
        assert_eq!(attachments[1]["kind"], "image");
        assert!(attachments
            .iter()
            .all(|value| value["sha256"].as_str().is_some()));
        respond_json(submit.stream, &task_submit_response("task-restart"));

        let stream = accept_request(&listener);
        assert_eq!(stream.path, "/v1/tasks/task-restart/events?cursor=0");
        respond_sse(stream.stream, &[task_final_event_for(1, "task-restart")]);
        let status = accept_request(&listener);
        assert_eq!(status.path, "/v1/tasks/task-restart");
        respond_json(
            status.stream,
            &task_status_response(
                "task-restart",
                "succeeded",
                "completed",
                Some("restart complete"),
            ),
        );
    });

    let session_store = temporary_session_store("restart_attachments");
    let workspace = std::env::current_dir().expect("workspace");
    let text_path = workspace.join(format!(".clawcli_restart_{}.txt", std::process::id()));
    let image_path = workspace.join(format!(".clawcli_restart_{}.png", std::process::id()));
    std::fs::write(&text_path, "persisted context").expect("write text fixture");
    std::fs::write(&image_path, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRfixture")
        .expect("write image fixture");

    let first = run_chat_with_input(
        address,
        &session_store,
        false,
        &format!(
            "/file {}\n/image {}\n/exit\n",
            text_path.display(),
            image_path.display()
        ),
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = String::from_utf8(first.stdout).expect("first stdout");
    assert!(
        first_stdout.contains("attachment_count=1"),
        "{first_stdout}"
    );
    assert!(
        first_stdout.contains("attachment_count=2"),
        "{first_stdout}"
    );

    let second = run_chat_with_input(
        address,
        &session_store,
        false,
        "use persisted context\n/exit\n",
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("restart complete"),
        "{}",
        String::from_utf8_lossy(&second.stdout)
    );
    server.join().expect("mock clawd");

    let third = run_chat_with_input(address, &session_store, false, "/attachments\n/exit\n");
    assert!(
        third.status.success(),
        "{}",
        String::from_utf8_lossy(&third.stderr)
    );
    assert!(
        String::from_utf8_lossy(&third.stdout).contains("attachment_count=0"),
        "{}",
        String::from_utf8_lossy(&third.stdout)
    );

    let _ = std::fs::remove_file(&session_store);
    let _ = std::fs::remove_file(text_path);
    let _ = std::fs::remove_file(image_path);
}

#[test]
fn code_diff_and_rewind_use_machine_capabilities_and_jsonl_exit_schema() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        run_code_capability_mock(
            &listener,
            "task-diff",
            "workspace.diff",
            json!({"paths": ["src/lib.rs"]}),
        );
        run_code_capability_mock(
            &listener,
            "task-rewind",
            "workspace.revert_checkpoint",
            json!({"checkpoint_id": "checkpoint-1"}),
        );
    });

    let diff = Command::new(env!("CARGO_BIN_EXE_clawcli"))
        .args([
            "--base-url",
            &format!("http://{address}"),
            "--key",
            "test-key",
            "code",
            "diff",
            "--path",
            "src/lib.rs",
            "--jsonl",
        ])
        .output()
        .expect("run code diff");
    assert_jsonl_capability_output(&diff, "workspace.diff");

    let rewind = Command::new(env!("CARGO_BIN_EXE_clawcli"))
        .args([
            "--base-url",
            &format!("http://{address}"),
            "--key",
            "test-key",
            "code",
            "rewind",
            "--checkpoint-id",
            "checkpoint-1",
            "--jsonl",
        ])
        .output()
        .expect("run code rewind");
    assert_jsonl_capability_output(&rewind, "workspace.revert_checkpoint");
    server.join().expect("mock clawd");
}

fn temporary_session_store(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "clawcli_{label}_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn spawn_chat(
    address: std::net::SocketAddr,
    session_store: &std::path::Path,
    jsonl: bool,
) -> std::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_clawcli"));
    command
        .args([
            "--base-url",
            &format!("http://{address}"),
            "--key",
            "test-key",
            "chat",
            "--conversation-id",
            "conversation-golden",
        ])
        .env("APP_CLAWCLI_SESSION_STORE", session_store)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if jsonl {
        command.arg("--jsonl");
    }
    command.spawn().expect("spawn chat")
}

fn run_chat_with_input(
    address: std::net::SocketAddr,
    session_store: &std::path::Path,
    jsonl: bool,
    input: &str,
) -> std::process::Output {
    let mut child = spawn_chat(address, session_store, jsonl);
    let mut stdin = child.stdin.take().expect("chat stdin");
    write!(stdin, "{input}").expect("write chat input");
    drop(stdin);
    child.wait_with_output().expect("wait chat")
}

fn assistant_presentation_events(task_id: &str, content: &str) -> Vec<Value> {
    let digest = format!("sha256:{:x}", Sha256::digest(content.as_bytes()));
    let common = |sequence: u64, offset: usize| {
        json!({
            "schema_version": 1,
            "task_id": task_id,
            "conversation_id": "conversation-golden",
            "turn_id": "turn-golden",
            "stream_id": "stream-golden",
            "attempt_id": "attempt-golden",
            "sequence": sequence,
            "content_offset_bytes": offset,
            "created_at": 10 + sequence,
        })
    };
    let mut started = common(0, 0);
    let mut delta = common(1, 0);
    delta["content"] = Value::String(content.to_string());
    let mut completed = common(2, content.len());
    completed["total_content_bytes"] = json!(content.len());
    completed["content_sha256"] = Value::String(digest);
    vec![
        journal_event(1, task_id, "assistant_output_started", started.take()),
        journal_event(2, task_id, "assistant_output_delta", delta.take()),
        journal_event(3, task_id, "assistant_output_completed", completed.take()),
        task_final_event_for(4, task_id),
    ]
}

fn journal_event(seq: u64, task_id: &str, event_type: &str, payload: Value) -> Value {
    json!({
        "schema_version": 1,
        "seq": seq,
        "task_id": task_id,
        "event_type": event_type,
        "payload": payload,
    })
}

fn task_final_event_for(seq: u64, task_id: &str) -> Value {
    journal_event(
        seq,
        task_id,
        "task_final",
        json!({"execution_state": "completed", "status": "succeeded"}),
    )
}

fn run_code_capability_mock(
    listener: &TcpListener,
    task_id: &str,
    capability: &str,
    expected_args: Value,
) {
    let submit = accept_request(listener);
    assert_eq!(submit.path, "/v1/tasks");
    let body = parse_json_body(&submit);
    assert_eq!(body["payload"]["entrypoint"], "run_capability");
    assert_eq!(body["payload"]["capability"], capability);
    assert_eq!(body["payload"]["args"], expected_args);
    respond_json(submit.stream, &task_submit_response(task_id));

    let stream = accept_request(listener);
    assert_eq!(stream.path, format!("/v1/tasks/{task_id}/events?cursor=0"));
    respond_sse(
        stream.stream,
        &[json!({
            "seq": 1,
            "event_type": "task_final",
            "payload": {"execution_state": "completed", "status": "succeeded"}
        })],
    );
    let status = accept_request(listener);
    assert_eq!(status.path, format!("/v1/tasks/{task_id}"));
    respond_json(
        status.stream,
        &task_status_response(
            task_id,
            "succeeded",
            "completed",
            Some("capability-complete"),
        ),
    );
}

fn assert_jsonl_capability_output(output: &std::process::Output, capability: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout={stdout}\nstderr={stderr}");
    let records = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("JSONL record"))
        .collect::<Vec<_>>();
    assert!(records.len() >= 2, "{stdout}");
    let summary = records.last().expect("capability summary");
    assert_eq!(summary["capability"], capability);
    assert_eq!(summary["exit_class"], "success");
    assert_eq!(summary["exit_code"], 0);
}

fn run_mock_clawd(listener: TcpListener) {
    let first_submit = accept_request(&listener);
    assert_eq!(first_submit.path, "/v1/tasks");
    let first_payload = parse_json_body(&first_submit);
    let thread_id = json_string(&first_payload, "/payload/thread_id");
    let session_id = json_string(&first_payload, "/payload/session_id");
    assert!(first_payload.pointer("/payload/resume_task_id").is_none());
    respond_json(first_submit.stream, &task_submit_response("task-1"));

    let first_stream = accept_request(&listener);
    assert_eq!(first_stream.path, "/v1/tasks/task-1/events?cursor=0");
    respond_sse(
        first_stream.stream,
        &[json!({
            "seq": 1,
            "event_type": "tool_started",
            "payload": {"execution_state": "running", "skill": "fs_basic"}
        })],
    );
    let running_status = accept_request(&listener);
    assert_eq!(running_status.path, "/v1/tasks/task-1");
    respond_json(
        running_status.stream,
        &task_status_response("task-1", "running", "running", None),
    );

    let resumed_stream = accept_request(&listener);
    assert_eq!(resumed_stream.path, "/v1/tasks/task-1/events?cursor=1");
    respond_sse(
        resumed_stream.stream,
        &[json!({
            "seq": 2,
            "event_type": "permission",
            "payload": {
                "execution_state": "needs_confirmation",
                "decision": "require_confirmation"
            }
        })],
    );
    let pending_status = accept_request(&listener);
    assert_eq!(pending_status.path, "/v1/tasks/task-1");
    respond_json(pending_status.stream, &approval_status_response("task-1"));

    let approval_lookup = accept_request(&listener);
    assert_eq!(approval_lookup.path, "/v1/tasks/task-1");
    respond_json(approval_lookup.stream, &approval_status_response("task-1"));
    let approval = accept_request(&listener);
    assert_eq!(approval.path, "/v1/tasks/resume-by-task-id");
    let approval_body = parse_json_body(&approval);
    assert_eq!(
        approval_body["approval_request_id"],
        Value::String("approval-1".to_string())
    );
    assert_eq!(
        approval_body["approval_decision"],
        Value::String("always_for_scope".to_string())
    );
    respond_json(
        approval.stream,
        &json!({
            "ok": true,
            "data": {
                "status": "approval_scope_grant_created",
                "task_id": "task-1",
                "approval_request_id": "approval-1",
                "approval_decision": "always_for_scope",
                "scope_grant": {
                    "grant_id": "scope-grant-1",
                    "scope_kind": "session",
                    "scope_fingerprint": "sha256:scope",
                    "issued_at": 100,
                    "expires_at": 200
                }
            }
        }),
    );

    let presentation_snapshot = accept_request(&listener);
    assert_eq!(
        presentation_snapshot.path,
        "/v1/tasks/task-1/events?cursor=0&follow=false"
    );
    respond_sse(presentation_snapshot.stream, &[]);

    let approved_stream = accept_request(&listener);
    assert_eq!(approved_stream.path, "/v1/tasks/task-1/events?cursor=2");
    respond_sse(
        approved_stream.stream,
        &[json!({
            "seq": 3,
            "event_type": "task_final",
            "payload": {"execution_state": "completed", "status": "succeeded"}
        })],
    );
    let first_final = accept_request(&listener);
    assert_eq!(first_final.path, "/v1/tasks/task-1");
    respond_json(
        first_final.stream,
        &task_status_response("task-1", "succeeded", "completed", Some("turn-1-complete")),
    );

    for turn in 2..=2 {
        let submit = accept_request(&listener);
        assert_eq!(submit.path, "/v1/tasks");
        let payload = parse_json_body(&submit);
        assert_eq!(json_string(&payload, "/payload/thread_id"), thread_id);
        assert_eq!(json_string(&payload, "/payload/session_id"), session_id);
        assert_eq!(
            json_string(&payload, "/payload/resume_task_id"),
            format!("task-{}", turn - 1)
        );
        let task_id = format!("task-{turn}");
        respond_json(submit.stream, &task_submit_response(&task_id));

        let stream = accept_request(&listener);
        assert_eq!(stream.path, format!("/v1/tasks/{task_id}/events?cursor=0"));
        let events = if turn == 6 {
            assistant_presentation_events(&task_id, "turn-6-complete")
        } else {
            vec![task_final_event_for(1, &task_id)]
        };
        respond_sse(stream.stream, &events);
        let final_status = accept_request(&listener);
        assert_eq!(final_status.path, format!("/v1/tasks/{task_id}"));
        respond_json(
            final_status.stream,
            &task_status_response(
                &task_id,
                "succeeded",
                "completed",
                Some(&format!("turn-{turn}-complete")),
            ),
        );
    }

    let background_submit = accept_request(&listener);
    assert_eq!(background_submit.path, "/v1/tasks");
    let background_payload = parse_json_body(&background_submit);
    assert_eq!(
        json_string(&background_payload, "/payload/thread_id"),
        thread_id
    );
    assert_eq!(
        json_string(&background_payload, "/payload/session_id"),
        session_id
    );
    assert_eq!(
        json_string(&background_payload, "/payload/resume_task_id"),
        "task-2"
    );
    assert_eq!(
        json_string(&background_payload, "/payload/text"),
        "run focused tests"
    );
    respond_json(background_submit.stream, &task_submit_response("task-3"));

    let background_stream = accept_request(&listener);
    assert_eq!(background_stream.path, "/v1/tasks/task-3/events?cursor=0");
    respond_sse(
        background_stream.stream,
        &[json!({
            "seq": 1,
            "event_type": "checkpoint_created",
            "payload": {
                "execution_state": "background",
                "checkpoint_id": "checkpoint-coding-3",
                "next_action_kind": "resume_checkpoint"
            }
        })],
    );
    let background_status = accept_request(&listener);
    assert_eq!(background_status.path, "/v1/tasks/task-3");
    respond_json(
        background_status.stream,
        &background_task_status_response("task-3"),
    );

    let continue_request = accept_request(&listener);
    assert_eq!(continue_request.path, "/v1/tasks/resume-by-task-id");
    let continue_payload = parse_json_body(&continue_request);
    assert_eq!(continue_payload["task_id"], "task-3");
    assert_eq!(continue_payload["resume_reason"], "user_continue");
    assert!(continue_payload.get("approval_decision").is_none());
    respond_json(
        continue_request.stream,
        &json!({
            "ok": true,
            "data": {
                "status": "task_resume_requested",
                "task_id": "task-3",
                "checkpoint_id": "checkpoint-coding-3",
                "resume_due": true
            }
        }),
    );

    let continued_snapshot = accept_request(&listener);
    assert_eq!(
        continued_snapshot.path,
        "/v1/tasks/task-3/events?cursor=0&follow=false"
    );
    respond_sse(continued_snapshot.stream, &[]);

    let continued_stream = accept_request(&listener);
    assert_eq!(continued_stream.path, "/v1/tasks/task-3/events?cursor=1");
    respond_sse(
        continued_stream.stream,
        &[json!({
            "seq": 2,
            "event_type": "task_final",
            "payload": {"execution_state": "completed", "status": "succeeded"}
        })],
    );
    let continued_status = accept_request(&listener);
    assert_eq!(continued_status.path, "/v1/tasks/task-3");
    respond_json(
        continued_status.stream,
        &task_status_response("task-3", "succeeded", "completed", Some("turn-3-complete")),
    );

    let expected_prompts = [
        "correct the failing test",
        "review the diff",
        "finish with verification",
    ];
    for turn in 4..=6 {
        let submit = accept_request(&listener);
        assert_eq!(submit.path, "/v1/tasks");
        let payload = parse_json_body(&submit);
        assert_eq!(json_string(&payload, "/payload/thread_id"), thread_id);
        assert_eq!(json_string(&payload, "/payload/session_id"), session_id);
        assert_eq!(
            json_string(&payload, "/payload/resume_task_id"),
            format!("task-{}", turn - 1)
        );
        assert_eq!(
            json_string(&payload, "/payload/text"),
            expected_prompts[turn - 4]
        );
        let task_id = format!("task-{turn}");
        respond_json(submit.stream, &task_submit_response(&task_id));

        let stream = accept_request(&listener);
        assert_eq!(stream.path, format!("/v1/tasks/{task_id}/events?cursor=0"));
        respond_sse(
            stream.stream,
            &[json!({
                "seq": 1,
                "event_type": "task_final",
                "payload": {"execution_state": "completed", "status": "succeeded"}
            })],
        );
        let final_status = accept_request(&listener);
        assert_eq!(final_status.path, format!("/v1/tasks/{task_id}"));
        respond_json(
            final_status.stream,
            &task_status_response(
                &task_id,
                "succeeded",
                "completed",
                Some(&format!("turn-{turn}-complete")),
            ),
        );
    }
}

struct MockRequest {
    path: String,
    body: Vec<u8>,
    stream: TcpStream,
}

fn accept_request(listener: &TcpListener) -> MockRequest {
    listener
        .set_nonblocking(true)
        .expect("nonblocking mock listener");
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "mock request timeout");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept request: {error}"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("mock request read timeout");
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let path = request_line
        .split_whitespace()
        .nth(1)
        .expect("request path")
        .to_string();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim())
        {
            content_length = value.parse().expect("content length");
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).expect("request body");
    drop(reader);
    MockRequest { path, body, stream }
}

fn parse_json_body(request: &MockRequest) -> Value {
    serde_json::from_slice(&request.body).expect("request JSON")
}

fn json_string(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .expect("JSON string")
        .to_string()
}

fn task_submit_response(task_id: &str) -> Value {
    json!({"ok": true, "data": {"task_id": task_id}})
}

fn task_status_response(
    task_id: &str,
    status: &str,
    execution_state: &str,
    message: Option<&str>,
) -> Value {
    let messages = message
        .map(|text| vec![json!({"text": text})])
        .unwrap_or_default();
    json!({
        "ok": true,
        "data": {
            "task_id": task_id,
            "status": status,
            "execution_state": execution_state,
            "result_json": {"messages": messages}
        }
    })
}

fn background_task_status_response(task_id: &str) -> Value {
    json!({
        "ok": true,
        "data": {
            "task_id": task_id,
            "status": "running",
            "execution_state": "background",
            "task_lifecycle": {
                "state": "background",
                "execution_state": "background",
                "checkpoint_id": "checkpoint-coding-3",
                "resume_due": false,
                "next_action_kind": "resume_checkpoint"
            },
            "result_json": {
                "messages": [],
                "resume_context": {
                    "checkpoint_id": "checkpoint-coding-3",
                    "resume_entrypoint": "next_planner_round"
                }
            }
        }
    })
}

fn approval_status_response(task_id: &str) -> Value {
    json!({
        "ok": true,
        "data": {
            "task_id": task_id,
            "status": "failed",
            "execution_state": "needs_confirmation",
            "result_json": {
                "messages": [],
                "resume_context": {
                    "approval_request": {
                        "request_id": "approval-1",
                        "status": "pending"
                    }
                }
            }
        }
    })
}

fn respond_json(mut stream: TcpStream, body: &Value) {
    let body = serde_json::to_string(body).expect("response JSON");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .expect("write JSON response");
    stream.flush().expect("flush JSON response");
}

fn respond_sse(mut stream: TcpStream, events: &[Value]) {
    write_sse_headers(&mut stream);
    write_sse_events(&mut stream, events);
}

fn write_sse_headers(stream: &mut TcpStream) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
    )
    .expect("write SSE headers");
    stream.flush().expect("flush SSE headers");
}

fn write_sse_events(stream: &mut TcpStream, events: &[Value]) {
    for event in events {
        writeln!(
            stream,
            "data: {}\n",
            serde_json::to_string(event).expect("SSE JSON")
        )
        .expect("write SSE event");
    }
    stream.flush().expect("flush SSE response");
}
