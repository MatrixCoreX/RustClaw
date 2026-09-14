use super::*;
use crate::{planned_model_kind, AudioInput, AudioTranscribeConfig, VendorKind};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct AudioFixture(std::path::PathBuf);

impl AudioFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "stt-native-test-{}-{}.wav",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap()
            .write_all(b"RIFF-audio-fixture")
            .unwrap();
        Self(path)
    }
}

impl Drop for AudioFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn endpoint(status: u16, body: &str) -> (VendorConfig, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_owned();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let n = socket.read(&mut chunk).unwrap();
            assert!(n > 0, "request ended before complete multipart body");
            request.extend_from_slice(&chunk[..n]);
            if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .expect("known-size multipart should carry content-length")
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        write!(socket, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        String::from_utf8(request).unwrap()
    });
    (
        VendorConfig {
            base_url: format!("http://{address}/v1/"),
            api_key: "probe-token".into(),
            model: "asr-test".into(),
            timeout_seconds: None,
        },
        handle,
    )
}

fn client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

#[test]
fn native_request_uses_speech_endpoint_and_preserves_review_contract() {
    let audio = AudioFixture::new();
    let (vendor, request) = endpoint(200, r#"{"text":"  recognized words  ","duration":1}"#);
    let cfg = crate::RootConfig {
        audio_transcribe: AudioTranscribeConfig {
            default_vendor: Some("minimax".into()),
            default_model: Some("asr-test".into()),
            providers: crate::AudioProviderOverrides {
                minimax: Some(vendor),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let (_, extra) = crate::execute(&cfg, audio.0.parent().unwrap(), json!({
        "audio_path": audio.0, "language": "zh-CN", "transcribe_hint": "Do not transmit this hint.",
    }), None).unwrap();
    assert_eq!(extra["provider"], "minimax");
    assert_eq!(extra["model_kind"], "native");
    assert_eq!(
        extra["transcription_review"]["raw_text"],
        "recognized words"
    );
    assert_eq!(extra["transcription_review"]["required"], true);
    assert_eq!(
        extra["transcription_review"]["delivery"]["mode"],
        "inline_and_artifact"
    );
    let request = request.join().unwrap();
    assert!(request.starts_with("POST /v1/speech_to_text HTTP/1.1"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer probe-token"));
    assert!(request.to_ascii_lowercase().contains("language: zh\r\n"));
    for (name, value) in [
        ("model", "asr-test"),
        ("response_format", "json"),
        ("stream", "false"),
    ] {
        assert!(request.contains(&format!("name=\"{name}\"\r\n\r\n{value}")));
    }
    assert!(request.contains("name=\"file\""));
    assert!(!request.contains("name=\"prompt\""));
    assert!(!request.contains("Do not transmit this hint."));
}

#[test]
fn adapter_preview_and_explicit_compatible_proxy_remain_consistent() {
    for mode in ["auto", "native"] {
        let cfg = AudioTranscribeConfig {
            adapter_mode: Some(mode.into()),
            ..Default::default()
        };
        assert_eq!(
            planned_model_kind(&cfg, VendorKind::MiniMax, "asr-test"),
            "native"
        );
    }
    let cfg = AudioTranscribeConfig {
        adapter_mode: Some("compat".into()),
        ..Default::default()
    };
    assert_eq!(
        planned_model_kind(&cfg, VendorKind::MiniMax, "asr-test"),
        "compat"
    );
    let audio = AudioFixture::new();
    let (vendor, request) = endpoint(200, r#"{"text":"compatible words"}"#);
    let (text, kind) = crate::transcribe_by_vendor(
        &client(),
        &cfg,
        &vendor,
        VendorKind::MiniMax,
        false,
        "minimax",
        "asr-test",
        &AudioInput::LocalPath(audio.0.clone()),
        "hint",
        None,
        Some("probe-token"),
    )
    .unwrap();
    assert_eq!(text, "compatible words");
    assert_eq!(kind, "compat");
    assert!(request
        .join()
        .unwrap()
        .starts_with("POST /v1/audio/transcriptions HTTP/1.1"));
}

#[test]
fn failed_native_calls_do_not_offer_local_fallback_or_retry_in_adapter() {
    let audio = AudioFixture::new();
    for status in [400, 401, 413, 429, 500] {
        let (vendor, request) =
            endpoint(status, r#"{"error":{"message":"private-provider-detail"}}"#);
        let failure = transcribe(
            &client(),
            &vendor,
            "asr-test",
            &audio.0,
            None,
            Some("probe-token"),
        )
        .unwrap_err();
        assert_eq!(failure.retryable, status == 429 || status >= 500);
        assert_eq!(failure.message, format!("minimax_stt_http_{status}"));
        let extra = crate::error_extra(failure.code, failure.retryable);
        assert_eq!(extra["fallback_recommended"], false);
        assert!(extra.get("fallback_capability").is_none());
        assert!(extra.get("fallback_input_value").is_none());
        let request = request.join().unwrap();
        assert!(!request.to_ascii_lowercase().contains("\r\nlanguage:"));
    }
}

#[test]
fn malformed_and_error_responses_are_not_transcripts() {
    for body in [
        json!({}),
        json!({"text":" "}),
        json!({"text":123}),
        json!({"error":{"code":1004},"text":"not a transcript"}),
    ] {
        let failure = extract_transcript(&body).unwrap_err();
        assert_eq!(failure.code, "provider_request_failed");
    }
    let audio = AudioFixture::new();
    let (vendor, request) = endpoint(200, "<html>gateway error</html>");
    let failure = transcribe(
        &client(),
        &vendor,
        "asr-test",
        &audio.0,
        None,
        Some("probe-token"),
    )
    .unwrap_err();
    assert_eq!(failure.message, "minimax_stt_response_invalid");
    request.join().unwrap();
}

#[test]
fn native_language_hints_use_primary_tags_or_auto_detection() {
    for input in [None, Some(""), Some(" auto ")] {
        assert_eq!(language_hint(input), None);
    }
    for (input, expected) in [
        ("zh-CN", "zh"),
        ("yue-HK", "yue"),
        (" en-US ", "en"),
        ("JA", "ja"),
    ] {
        assert_eq!(language_hint(Some(input)).as_deref(), Some(expected));
    }
}

#[test]
fn missing_token_and_unreadable_file_fail_before_network() {
    let vendor = VendorConfig {
        base_url: "http://127.0.0.1:1/v1".into(),
        api_key: String::new(),
        model: "asr-test".into(),
        timeout_seconds: None,
    };
    let missing =
        std::env::temp_dir().join(format!("stt-missing-{}-audio.wav", std::process::id()));
    assert_eq!(
        transcribe(&client(), &vendor, "asr-test", &missing, None, None)
            .unwrap_err()
            .code,
        "provider_not_configured"
    );
    assert_eq!(
        transcribe(
            &client(),
            &vendor,
            "asr-test",
            &missing,
            None,
            Some("probe-token")
        )
        .unwrap_err()
        .code,
        "invalid_input"
    );
}
