use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const FIXTURE_KEY: &str = "fixture-admission-credential";

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    stopped: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
    }
}

fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut request = Vec::new();
    loop {
        let mut buffer = [0_u8; 2048];
        let count = socket.read(&mut buffer).unwrap();
        assert_ne!(count, 0, "incomplete fixture request");
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                return request;
            }
        }
        assert!(request.len() < 16384, "oversized fixture request");
    }
}

fn server(status: u16, redirect_base: Option<&str>, body: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let destination = format!("{}/destination", redirect_base.unwrap_or(&base));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let shutdown = stopped.clone();
    let body = body.to_owned();
    let task = thread::spawn(move || {
        while !shutdown.load(Ordering::SeqCst) {
            let (mut socket, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("fixture accept: {error}"),
            };
            let request = read_request(&mut socket);
            let original = request.starts_with(b"POST /v1/skills/import ");
            captured.lock().unwrap().push(request);
            let actual_status = if original { status } else { 200 };
            let location = if (300..400).contains(&actual_status) {
                format!("Location: {destination}\r\n")
            } else {
                String::new()
            };
            let response = format!(
                "HTTP/1.1 {actual_status} Fixture\r\n{location}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).unwrap();
        }
    });
    Server {
        base,
        requests,
        stopped,
        task: Some(task),
    }
}

fn admit(base: &str, source: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skillctl"))
        .args([
            "admit",
            source.to_str().unwrap(),
            base,
            FIXTURE_KEY,
            "--disabled",
            "--network",
            "--timeout-seconds",
            "5",
        ])
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .unwrap()
}

fn source() -> tempfile::TempDir {
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("skill.toml"), "schema_version = 1\n").unwrap();
    std::fs::write(
        source.path().join("INTERFACE.md"),
        "Admission HTTP fixture\n",
    )
    .unwrap();
    source
}

fn payload(output: &Output) -> Value {
    assert!(!String::from_utf8_lossy(&output.stdout).contains(FIXTURE_KEY));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(FIXTURE_KEY));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn fixture_reads_nonblocking_accepted_streams_as_bounded_blocking_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut accepted, _) = listener.accept().unwrap();
    accepted.set_nonblocking(true).unwrap();
    let error = accepted.peek(&mut [0_u8; 1]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        client
            .write_all(b"POST /fixture HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
    });
    let request = read_request(&mut accepted);
    assert_eq!(
        request,
        b"POST /fixture HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}"
    );
    writer.join().unwrap();
}

#[test]
fn admission_never_forwards_credentials_or_replays_through_redirects() {
    let source = source();
    let mut failures = Vec::new();
    for status in [301, 302, 303, 307, 308] {
        for cross_origin in [false, true] {
            let target = server(
                200,
                None,
                r#"{"ok":true,"data":{"state":"installed_disabled"}}"#,
            );
            let origin = server(
                status,
                cross_origin.then_some(target.base.as_str()),
                r#"{"ok":true,"data":{"state":"installed_disabled"}}"#,
            );
            let output = admit(&origin.base, source.path());
            let value = payload(&output);
            let origins = origin.requests.lock().unwrap();
            let targets = target.requests.lock().unwrap();
            let forwarded_key = origins.iter().skip(1).chain(targets.iter()).any(|request| {
                String::from_utf8_lossy(request)
                    .to_ascii_lowercase()
                    .contains(&format!("x-agent-key: {FIXTURE_KEY}\r\n"))
            });
            if output.status.success()
                || value["error"]["code"] != "admission_redirect_rejected"
                || origins.len() != 1
                || !targets.is_empty()
            {
                failures.push(format!("status={status} cross_origin={cross_origin} origins={} targets={} forwarded_key={forwarded_key} result={value}", origins.len(), targets.len()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn direct_admission_preserves_auth_options_and_structured_response() {
    let source = source();
    let server = server(
        200,
        None,
        r#"{"ok":true,"data":{"state":"installed_disabled","generation":"fixture-generation"}}"#,
    );
    let output = admit(&format!("{}/", server.base), source.path());
    assert!(output.status.success());
    assert_eq!(
        payload(&output),
        json!({"ok":true,"command":"admit","admission":{
            "state":"installed_disabled","generation":"fixture-generation"
        }})
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let raw = String::from_utf8_lossy(&requests[0]);
    assert!(raw.starts_with("POST /v1/skills/import HTTP/1.1\r\n"));
    assert!(raw
        .to_ascii_lowercase()
        .contains(&format!("x-agent-key: {FIXTURE_KEY}\r\n")));
    let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({"source":source.path().canonicalize().unwrap(),
        "enabled":false,"allow_network":true})
    );
}

#[test]
fn direct_admission_keeps_rejection_and_malformed_response_failures() {
    let source = source();
    for (status, body, code) in [
        (
            403,
            r#"{"ok":false,"error":"policy_denied"}"#,
            "admission_rejected",
        ),
        (
            200,
            r#"{"ok":false,"error":"policy_denied"}"#,
            "admission_rejected",
        ),
        (200, "not-json", "admission_response_invalid"),
    ] {
        let server = server(status, None, body);
        let output = admit(&server.base, source.path());
        assert!(!output.status.success());
        assert_eq!(payload(&output)["error"]["code"], code);
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}
