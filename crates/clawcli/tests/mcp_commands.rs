use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;

use serde_json::{json, Value};

#[test]
fn mcp_commands_use_authenticated_machine_endpoints() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock clawd");
    let address = listener.local_addr().expect("mock address");
    let server = thread::spawn(move || serve_requests(listener, 4));
    let base_url = format!("http://{address}");

    let list = run_cli(&base_url, &["mcp", "list", "--json"]);
    assert_eq!(
        list.pointer("/data/servers/0/server_id"),
        Some(&json!("alpha"))
    );

    let status = run_cli(&base_url, &["mcp", "status", "alpha", "--json"]);
    assert_eq!(
        status
            .pointer("/data/servers")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    let tools = run_cli(&base_url, &["mcp", "tools", "alpha", "--json"]);
    assert_eq!(
        tools.pointer("/data/tools/0/capability"),
        Some(&json!("mcp.alpha.lookup"))
    );

    let probe = run_cli(&base_url, &["mcp", "test", "alpha", "--json"]);
    assert_eq!(probe.pointer("/data/probe/status"), Some(&json!("ok")));
    server.join().expect("mock clawd join");
}

fn run_cli(base_url: &str, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_clawcli"))
        .args(["--base-url", base_url, "--key", "test-admin-key"])
        .args(args)
        .output()
        .expect("run clawcli");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("machine JSON output")
}

fn serve_requests(listener: TcpListener, count: usize) {
    for _ in 0..count {
        let (mut stream, _) = listener.accept().expect("accept CLI request");
        let request = read_request(&mut stream);
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("x-rustclaw-key: test-admin-key"));
        let first_line = request.lines().next().unwrap_or_default();
        let body = if first_line.starts_with("GET /v1/admin/mcp/servers ") {
            json!({
                "ok": true,
                "data": {"servers": [
                    {"server_id": "alpha", "state": "ready", "transport": "stdio", "tool_count": 1, "last_error_code": null},
                    {"server_id": "beta", "state": "disabled", "transport": "streamable_http", "tool_count": 0, "last_error_code": null}
                ]},
                "error": null
            })
        } else if first_line.starts_with("GET /v1/admin/mcp/tools?server_id=alpha ") {
            json!({
                "ok": true,
                "data": {"tools": [{
                    "capability": "mcp.alpha.lookup",
                    "server_id": "alpha",
                    "required_args": ["query"],
                    "policy": {"effect": "observe", "risk_level": "low"}
                }]},
                "error": null
            })
        } else if first_line.starts_with("POST /v1/admin/mcp/servers/alpha/test ") {
            json!({
                "ok": true,
                "data": {"probe": {"server_id": "alpha", "status": "ok", "latency_ms": 1}},
                "error": null
            })
        } else {
            panic!("unexpected request: {first_line}");
        };
        write_json(&mut stream, &body);
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(bytes).expect("HTTP request UTF-8")
}

fn write_json(stream: &mut TcpStream, body: &Value) {
    let body = serde_json::to_string(body).expect("serialize response");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .expect("write response");
}
