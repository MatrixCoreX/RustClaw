use std::fs;
use std::io::{self, Read};
use std::thread;
use std::time::Duration;

fn string_field<'a>(input: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("\"{key}\":\"");
    let start = input.find(&marker)? + marker.len();
    let end = start + input[start..].find('"')?;
    Some(&input[start..end])
}

fn number_field(input: &str, key: &str) -> Option<i64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let value = input[start..]
        .chars()
        .take_while(|value| value.is_ascii_digit() || *value == '-')
        .collect::<String>();
    value.parse().ok()
}

fn ok(request_id: &str, extra: &str) {
    println!(
        "{{\"request_id\":\"{request_id}\",\"status\":\"ok\",\"text\":\"\",\"error_text\":null,\"extra\":{extra}}}"
    );
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("stdin");
    let request_id = string_field(&input, "request_id").unwrap_or("invalid");
    let action = string_field(&input, "action").unwrap_or("calculate");
    match action {
        "calculate" => ok(
            request_id,
            &format!(
                "{{\"result\":{{\"value\":{}}}}}",
                number_field(&input, "a").unwrap_or(0) + number_field(&input, "b").unwrap_or(0)
            ),
        ),
        "validation_error" => println!(
            "{{\"request_id\":\"{request_id}\",\"status\":\"error\",\"text\":\"\",\"error_text\":\"invalid fixture input\",\"extra\":{{\"error_code\":\"fixture_invalid\",\"message_key\":\"fixture.invalid\"}}}}"
        ),
        "artifact" => {
            let path = string_field(&input, "artifact_path").expect("artifact_path");
            fs::write(path, b"reference-artifact\n").expect("artifact write");
            ok(request_id, "{\"artifact\":{\"created\":true}}");
        }
        "waiting" => ok(request_id, "{\"continuation\":{\"state\":\"waiting\",\"poll_after_ms\":10}}"),
        "needs_user" => ok(request_id, "{\"continuation\":{\"state\":\"needs_user\",\"required_fields\":[\"confirmation\"]}}"),
        "timeout" => {
            thread::sleep(Duration::from_secs(5));
            ok(request_id, "{}");
        }
        "malformed" => println!("{{not-json"),
        "multiple" => {
            ok(request_id, "{}");
            ok(request_id, "{}");
        }
        "oversized" => println!("{}", "x".repeat(1024 * 1024 + 1)),
        "stderr" => {
            eprintln!("reference diagnostic");
            ok(request_id, "{\"diagnostic_preserved\":true}");
        }
        _ => ok(request_id, "{}"),
    }
}
