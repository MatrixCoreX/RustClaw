use super::*;

#[test]
fn unavailable_api_has_structured_non_applied_proof_in_any_language() {
    for message in ["unavailable", "indisponible", "nicht verfuegbar"] {
        let error = GenerationError::provider_response(
            410,
            &json!({"base_resp":{"status_code":2153,"status_msg":message}}),
        );
        let extra = error.extra();
        assert_eq!(extra["status_code"], 410);
        assert_eq!(extra["provider_status_code"], 2153);
        assert_eq!(extra["failure_phase"], "provider_rejected");
        assert_eq!(extra["side_effect_applied"], false);
        assert_eq!(extra["retryable"], false);
        assert_eq!(extra["error_code"], "provider_capability_unavailable");
    }
}

#[test]
fn ambiguous_responses_never_claim_no_effect() {
    for (status, value) in [
        (500, json!({"base_resp":{"status_code":2153}})),
        (410, json!({"base_resp":{"status_code":1000}})),
        (410, json!({"base_resp":{"status_code":"2153"}})),
        (410, json!({"message":"API unavailable"})),
        (
            410,
            json!({"base_resp":{"status_code":2153},"data":{"audio":"01"}}),
        ),
        (
            410,
            json!({"base_resp":{"status_code":2153},"data":{"task_id":"job"}}),
        ),
        (
            410,
            json!({"base_resp":{"status_code":2153},"task_id":"job"}),
        ),
        (
            410,
            json!({"base_resp":{"status_code":2153},"job_id":"job"}),
        ),
    ] {
        assert!(
            GenerationError::provider_response(status, &value)
                .extra()
                .get("side_effect_applied")
                .is_none(),
            "{status}: {value}"
        );
    }
    let extra = GenerationError::from("request timed out".to_string()).extra();
    assert!(extra.get("side_effect_applied").is_none());
    assert!(extra.get("failure_phase").is_none());
}

#[test]
fn http_rejection_preserves_typed_evidence_through_provider_adapter() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 2048];
        loop {
            let count = socket.read(&mut buffer).unwrap();
            assert!(count > 0 && request.len() < 16_384);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    assert!(headers.starts_with("POST /v1/music_generation "));
                    break;
                }
            }
        }
        let body = json!({"base_resp":{"status_code":2153,"status_msg":"unavailable"}}).to_string();
        write!(socket, "HTTP/1.1 410 Gone\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let config = crate::VendorConfig {
        base_url: format!("http://{address}/v1"),
        api_key: "fixture-key".to_string(),
        model: "fixture-model".to_string(),
        timeout_seconds: Some(3),
        adapter_kind: None,
    };
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let result = crate::call_music_generation(&client, &config, &json!({"prompt":"test"}));
    server.join().unwrap();
    let extra = result
        .unwrap_err()
        .with_provider("custom", "fixture-model")
        .extra();
    assert_eq!(extra["status_code"], 410);
    assert_eq!(extra["provider_status_code"], 2153);
    assert_eq!(extra["failure_phase"], "provider_rejected");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["provider"], "custom");
    assert_eq!(extra["model"], "fixture-model");
}
