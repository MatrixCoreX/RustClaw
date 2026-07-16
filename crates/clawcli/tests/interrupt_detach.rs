#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn sigint_detaches_exec_without_sending_cancel() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || {
        let (mut submit, _) = listener.accept().expect("accept submit");
        assert_eq!(request_path(&mut submit), "/v1/tasks");
        write_json(
            &mut submit,
            r#"{"ok":true,"data":{"task_id":"task-interrupt"}}"#,
        );

        let (mut events, _) = listener.accept().expect("accept event stream");
        assert_eq!(
            request_path(&mut events),
            "/v1/tasks/task-interrupt/events?cursor=0"
        );
        write!(
            events,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )
        .expect("write event headers");
        events.flush().expect("flush event headers");
        let held_stream = thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            drop(events);
        });

        let (mut status, _) = listener.accept().expect("accept status fetch");
        assert_eq!(request_path(&mut status), "/v1/tasks/task-interrupt");
        write_json(
            &mut status,
            r#"{"ok":true,"data":{"task_id":"task-interrupt","status":"running","execution_state":"running","result_json":{"messages":[]}}}"#,
        );
        held_stream.join().expect("held event stream");
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_clawcli"))
        .args([
            "--base-url",
            &format!("http://{address}"),
            "--key",
            "test-key",
            "exec",
            "inspect",
            "workspace",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn clawcli");

    thread::sleep(Duration::from_millis(350));
    let signal_status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal_status.success());

    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll clawcli") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("clawcli did not detach after SIGINT");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout pipe")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr pipe")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    server.join().expect("mock clawd");

    assert_eq!(status.code(), Some(130), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("exec_outcome: detached"), "{stdout}");
    assert!(stdout.contains("exec_exit_class: detached"), "{stdout}");
    assert!(stdout.contains("status: running"), "{stdout}");
}

fn request_path(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().expect("clone request stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("request header");
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    request_line
        .split_whitespace()
        .nth(1)
        .expect("request path")
        .to_string()
}

fn write_json(stream: &mut TcpStream, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .expect("write JSON response");
    stream.flush().expect("flush JSON response");
}
