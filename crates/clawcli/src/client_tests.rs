use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

const KEY: &str = "fixture-cli-credential";
const URL_ENV: &str = "APP_TEST_CLI_HTTP_URL";

struct FixtureServer {
    url: String,
    seen: Arc<Mutex<Vec<bool>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FixtureServer {
    fn new(status: u16, location: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let captured = Arc::clone(&seen);
        let stopping = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        let request = read_request(&mut stream);
                        let headers = request.to_ascii_lowercase();
                        captured.lock().unwrap().push(
                            headers
                                .lines()
                                .any(|line| line == format!("x-agent-key: {KEY}")),
                        );
                        let target = request
                            .lines()
                            .next()
                            .unwrap()
                            .contains(" /redirect-target ");
                        let code = if target { 200 } else { status };
                        let redirect = if code == 200 {
                            String::new()
                        } else {
                            format!("Location: {}\r\n", location.as_deref().unwrap())
                        };
                        let body = r#"{"ok":true,"data":{"worker_state":"running"}}"#;
                        write!(stream, "HTTP/1.1 {code} Fixture\r\n{redirect}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            }
        });
        Self {
            url,
            seen,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 16 * 1024);
        if let Some(index) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let size = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(size <= 4096);
    while bytes.len() < end + size {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    headers
}

// Child processes isolate proxy environment without mutating parallel Rust tests.
fn run_child_probe() -> bool {
    let Ok(url) = std::env::var(URL_ENV) else {
        return false;
    };
    let mode = std::env::var("APP_TEST_CLI_HTTP_MODE").unwrap();
    let expected: u16 = std::env::var("APP_TEST_CLI_HTTP_STATUS")
        .unwrap()
        .parse()
        .unwrap();
    if mode == "health" {
        assert_eq!(
            crate::commands::run_health(&url, Some(KEY)).is_ok(),
            expected == 200
        );
        return true;
    }
    let client = match mode.as_str() {
        "normal" => make_client().unwrap(),
        "stream_finite" => make_stream_client_with_timeout(Some(Duration::from_secs(5))).unwrap(),
        "stream_unbounded" => make_stream_client_with_timeout(None).unwrap(),
        _ => panic!("unknown fixture mode"),
    };
    let method = std::env::var("APP_TEST_CLI_HTTP_METHOD").unwrap();
    let request = if method == "POST" {
        client.post(&url).body("fixture mutation")
    } else {
        client.get(&url)
    };
    let response = request.header("x-agent-key", KEY).send().unwrap();
    assert_eq!(response.status().as_u16(), expected);
    true
}

fn probe(name: &str, url: &str, mode: &str, method: &str, status: u16) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env_clear()
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .env(URL_ENV, url)
        .env("APP_TEST_CLI_HTTP_MODE", mode)
        .env("APP_TEST_CLI_HTTP_METHOD", method)
        .env("APP_TEST_CLI_HTTP_STATUS", status.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("fixture client deadline");
        }
        thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{mode} {method} {status}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn authenticated_clients_reject_redirects() {
    if run_child_probe() {
        return;
    }
    for status in [301, 302, 303, 307, 308] {
        for same_origin in [false, true] {
            let target = FixtureServer::new(200, None);
            let location = if same_origin {
                "/redirect-target".into()
            } else {
                format!("{}/redirect-target", target.url)
            };
            let origin = FixtureServer::new(status, Some(location));
            for mode in ["normal", "stream_finite", "stream_unbounded"] {
                for method in ["GET", "POST"] {
                    origin.seen.lock().unwrap().clear();
                    probe(
                        "client::tests::authenticated_clients_reject_redirects",
                        &origin.url,
                        mode,
                        method,
                        status,
                    );
                    assert_eq!(*origin.seen.lock().unwrap(), vec![true]);
                    assert!(target.seen.lock().unwrap().is_empty());
                }
            }
        }
    }
}

#[test]
fn health_command_rejects_redirects() {
    if run_child_probe() {
        return;
    }
    for status in [301, 302, 303, 307, 308] {
        for same_origin in [false, true] {
            let target = FixtureServer::new(200, None);
            let location = if same_origin {
                "/redirect-target".into()
            } else {
                format!("{}/redirect-target", target.url)
            };
            let origin = FixtureServer::new(status, Some(location));
            probe(
                "client::tests::health_command_rejects_redirects",
                &origin.url,
                "health",
                "GET",
                status,
            );
            assert_eq!(*origin.seen.lock().unwrap(), vec![true]);
            assert!(target.seen.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn direct_authenticated_requests_still_succeed() {
    if run_child_probe() {
        return;
    }
    let origin = FixtureServer::new(200, None);
    for mode in ["normal", "stream_finite", "stream_unbounded"] {
        for method in ["GET", "POST"] {
            probe(
                "client::tests::direct_authenticated_requests_still_succeed",
                &origin.url,
                mode,
                method,
                200,
            );
        }
    }
    probe(
        "client::tests::direct_authenticated_requests_still_succeed",
        &origin.url,
        "health",
        "GET",
        200,
    );
    assert_eq!(*origin.seen.lock().unwrap(), vec![true; 7]);
}
