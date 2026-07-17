#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[test]
fn pty_chat_reconnects_approves_and_keeps_four_turns_in_one_thread() {
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
    let command_line = format!(
        "{} --base-url http://{} --key test-key chat --new --jsonl",
        env!("CARGO_BIN_EXE_clawcli"),
        address
    );
    let mut child = Command::new("script")
        .args([
            "-qefc",
            &command_line,
            transcript_path.to_str().expect("transcript path"),
        ])
        .env("RUSTCLAW_CLAWCLI_SESSION_STORE", &session_store)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn PTY chat");
    let mut stdin = child.stdin.take().expect("PTY stdin");
    for line in [
        "inspect workspace",
        "/approve",
        "update one file",
        "run focused tests",
        "review the diff",
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
    for task_id in ["task-1", "task-2", "task-3", "task-4"] {
        assert!(stdout.contains(&format!("task_id={task_id}")), "{stdout}");
    }
    assert!(stdout.contains("\"seq\":1"), "{stdout}");
    assert!(stdout.contains("\"seq\":3"), "{stdout}");
    assert!(stdout.contains("approval_grant_approved"), "{stdout}");
    assert!(stdout.contains("turn-4-complete"), "{stdout}");
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
        Value::String("approve_once".to_string())
    );
    respond_json(
        approval.stream,
        &json!({
            "ok": true,
            "data": {
                "status": "approval_grant_approved",
                "task_id": "task-1",
                "approval_request_id": "approval-1",
                "approval_decision": "approve_once"
            }
        }),
    );

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

    for turn in 2..=4 {
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
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
    )
    .expect("write SSE headers");
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
