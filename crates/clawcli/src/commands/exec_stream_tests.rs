use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

#[test]
fn exec_wait_consumes_terminal_sse_then_fetches_final_task_once() {
    let (base_url, server) = spawn_responses(vec![
        MockResponse::sse(
            "/v1/tasks/task-stream/events?cursor=0",
            serde_json::json!({
                "seq": 7,
                "event_type": "task_final",
                "event_kind": "task_final",
                "payload": {"execution_state": "completed"}
            }),
        ),
        MockResponse::json(
            "/v1/tasks/task-stream",
            200,
            serde_json::json!({
                "ok": true,
                "data": {
                    "task_id": "task-stream",
                    "status": "succeeded",
                    "execution_state": "completed",
                    "result_json": {"messages": [{"text": "done"}]}
                }
            }),
        ),
    ]);

    let (task, outcome) =
        wait_for_exec_task(&base_url, "test-key", "task-stream", test_wait_options())
            .expect("wait through event stream");
    server.join().expect("mock server");

    assert_eq!(outcome, ExecWaitOutcome::Terminal);
    assert_eq!(task.status, "succeeded");
    assert_eq!(task.result_text.as_deref(), Some("done"));
}

#[test]
fn exec_wait_uses_polling_only_when_event_endpoint_is_unavailable() {
    let (base_url, server) = spawn_responses(vec![
        MockResponse::json(
            "/v1/tasks/task-fallback/events?cursor=0",
            404,
            serde_json::json!({"error": {"code": "route_not_found"}}),
        ),
        MockResponse::json(
            "/v1/tasks/task-fallback",
            200,
            serde_json::json!({
                "ok": true,
                "data": {
                    "task_id": "task-fallback",
                    "status": "succeeded",
                    "execution_state": "completed",
                    "result_json": {"messages": []}
                }
            }),
        ),
    ]);

    let (task, outcome) =
        wait_for_exec_task(&base_url, "test-key", "task-fallback", test_wait_options())
            .expect("poll fallback");
    server.join().expect("mock server");

    assert_eq!(outcome, ExecWaitOutcome::Terminal);
    assert_eq!(task.status, "succeeded");
}

#[test]
fn stream_read_window_respects_total_deadline() {
    let now = Instant::now();
    assert_eq!(stream_read_window(None, now), Some(Duration::from_secs(2)));
    assert_eq!(
        stream_read_window(Some(now + Duration::from_secs(30)), now),
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        stream_read_window(Some(now + Duration::from_millis(40)), now),
        Some(Duration::from_millis(100))
    );
}

#[test]
fn exec_interrupt_detaches_without_calling_task_cancel() {
    let (base_url, server) = spawn_responses(vec![MockResponse::json(
        "/v1/tasks/task-detach",
        200,
        serde_json::json!({
            "ok": true,
            "data": {
                "task_id": "task-detach",
                "status": "running",
                "execution_state": "running",
                "result_json": {"messages": []}
            }
        }),
    )]);

    let (task, outcome) = wait_for_exec_task_with_interrupt(
        &base_url,
        "test-key",
        "task-detach",
        test_wait_options(),
        &|| true,
    )
    .expect("interrupt detach");
    server.join().expect("mock server");

    assert_eq!(outcome, ExecWaitOutcome::Detached);
    assert_eq!(task.status, "running");
    assert_eq!(
        exec_exit_class(&task, outcome, false),
        ExecExitClass::Detached
    );
}

fn test_wait_options() -> ExecWaitOptions {
    ExecWaitOptions {
        interval_ms: 100,
        timeout_seconds: Some(2),
        continue_on_background: false,
        fail_on_background: false,
        json_output: true,
        jsonl_output: false,
    }
}

struct MockResponse {
    expected_path: &'static str,
    status: u16,
    content_type: &'static str,
    body: String,
}

impl MockResponse {
    fn json(expected_path: &'static str, status: u16, value: Value) -> Self {
        Self {
            expected_path,
            status,
            content_type: "application/json",
            body: serde_json::to_string(&value).expect("serialize mock JSON"),
        }
    }

    fn sse(expected_path: &'static str, event: Value) -> Self {
        Self {
            expected_path,
            status: 200,
            content_type: "text/event-stream",
            body: format!(
                "id: 7\nevent: task_final\ndata: {}\n\n",
                serde_json::to_string(&event).expect("serialize mock event")
            ),
        }
    }
}

fn spawn_responses(responses: Vec<MockResponse>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept request");
            let request_line = read_request_line(&mut stream);
            assert!(
                request_line.contains(response.expected_path),
                "unexpected request: {request_line}"
            );
            let reason = if response.status == 200 {
                "OK"
            } else {
                "Not Found"
            };
            write!(
                stream,
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.status,
                reason,
                response.content_type,
                response.body.len(),
                response.body
            )
            .expect("write mock response");
            stream.flush().expect("flush mock response");
        }
    });
    (format!("http://{address}"), server)
}

fn read_request_line(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .expect("read request line");
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read request header");
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    first_line
}
