use super::{
    embed_one_with_config, embedding_spec_for_config, provider_for_profile, tokenize_text,
    EmbeddingRequestItem, LocalHashEmbeddingProvider, MemoryEmbeddingProvider, MemoryEmbeddingSpec,
    RemoteHttpEmbeddingProvider, LOCAL_HASH_DIMS, LOCAL_HASH_MODEL_ID, LOCAL_HASH_VERSION,
};

async fn spawn_http_response(response: String, delay_ms: u64) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture HTTP server");
    let address = listener.local_addr().expect("fixture address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept fixture request");
        let mut request = vec![0_u8; 16 * 1024];
        let _ = socket.read(&mut request).await;
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write fixture response");
    });
    format!("http://{address}/embeddings")
}

fn remote_fixture_provider(endpoint: String, timeout_ms: u64) -> RemoteHttpEmbeddingProvider {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_millis(timeout_ms))
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .unwrap();
    RemoteHttpEmbeddingProvider {
        client,
        endpoint,
        credential: "synthetic-test-token".to_string(),
        spec: MemoryEmbeddingSpec {
            model_id: "fixture-embedding".to_string(),
            dims: 3,
            version: "fixture-v1".to_string(),
            normalization: "unit_length".to_string(),
            provider_kind: "remote_http".to_string(),
        },
    }
}

#[tokio::test]
async fn mock_http_success_is_deterministic_and_index_aligned() {
    let body =
        r#"{"data":[{"index":0,"embedding":[1.0,0.0,0.0]},{"index":1,"embedding":[0.0,1.0,0.0]}]}"#;
    let endpoint = spawn_http_response(
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ),
        0,
    )
    .await;
    let provider = remote_fixture_provider(endpoint, 1_000);
    let request = vec![
        EmbeddingRequestItem {
            request_item_id: "first".to_string(),
            text: "alpha".to_string(),
        },
        EmbeddingRequestItem {
            request_item_id: "second".to_string(),
            text: "beta".to_string(),
        },
    ];
    let response = provider.embed_batch(&request).await.unwrap();
    assert_eq!(response[0].request_item_id, "first");
    assert_eq!(response[0].vector, vec![1.0, 0.0, 0.0]);
    assert_eq!(response[1].request_item_id, "second");
    assert_eq!(response[1].vector, vec![0.0, 1.0, 0.0]);
}

#[tokio::test]
async fn mock_http_rate_limit_server_error_redirect_and_timeout_are_structured() {
    let cases = [
        (
            "HTTP/1.1 429 Too Many Requests\r\nretry-after: 7\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            0,
            "memory_embedding_rate_limited",
            Some(7),
        ),
        (
            "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            0,
            "memory_embedding_provider_unavailable",
            None,
        ),
        (
            "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:9/blocked\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            0,
            "memory_embedding_provider_redirect_blocked",
            None,
        ),
        (
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"data\":[]}",
            250,
            "memory_embedding_provider_timeout",
            None,
        ),
    ];
    for (raw_response, delay_ms, expected_code, expected_retry_after) in cases {
        let endpoint = spawn_http_response(raw_response.to_string(), delay_ms).await;
        let provider = remote_fixture_provider(endpoint, 100);
        let error = provider
            .embed_batch(&[EmbeddingRequestItem {
                request_item_id: "fixture".to_string(),
                text: "fixture".to_string(),
            }])
            .await
            .unwrap_err();
        assert_eq!(error.error_code, expected_code);
        assert_eq!(error.retry_after_seconds, expected_retry_after);
    }
}
use claw_core::config::MemoryConfig;

#[tokio::test]
async fn local_hash_embedding_provider_is_stable() {
    let provider = LocalHashEmbeddingProvider;
    let items = vec![EmbeddingRequestItem {
        request_item_id: "item-1".to_string(),
        text: "以后默认用中文回复".to_string(),
    }];
    let first = provider.embed_batch(&items).await.expect("first embed");
    let second = provider.embed_batch(&items).await.expect("second embed");

    assert_eq!(first, second);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].vector.len(), LOCAL_HASH_DIMS);
    assert!(first[0].vector.iter().any(|value| *value > 0.0));
}

#[test]
fn tokenizer_v2_covers_long_multilingual_text_symbols_and_exact_identifiers() {
    let tokens = tokenize_text("天地玄黄宇宙洪荒日月盈昃辰宿列张寒来暑往秋收冬藏");
    assert!(tokens.contains(&"天地".to_string()));
    assert!(tokens.contains(&"冬藏".to_string()));
    assert_eq!(
        tokens
            .iter()
            .filter(|token| token.chars().count() == 2)
            .count(),
        23,
        "all bigrams must survive; tokenizer must not silently keep only the first N"
    );

    let multilingual = tokenize_text("メモリーを検索 한국어기억 🧠 /API/UserID");
    assert!(multilingual.contains(&"メモ".to_string()));
    assert!(multilingual.contains(&"한국".to_string()));
    assert!(multilingual.contains(&"symbol_1f9e0".to_string()));

    let upper_path = tokenize_text("/API/UserID");
    let lower_path = tokenize_text("/api/userid");
    let upper_exact = upper_path
        .iter()
        .find(|token| token.starts_with("exact_"))
        .expect("case-sensitive path token");
    let lower_exact = lower_path
        .iter()
        .find(|token| token.starts_with("exact_"))
        .expect("lowercase path token");
    assert_ne!(upper_exact, lower_exact);
}

#[tokio::test]
async fn mock_provider_obeys_the_configured_profile_contract() {
    let profile = super::super::vector_store::MemoryEmbeddingProfile {
        profile_id: "memory_embedding:mock:test".to_string(),
        provider_kind: "mock".to_string(),
        endpoint_ref: None,
        credential_ref: None,
        model_name: "fixture-mock".to_string(),
        dimensions: 7,
        normalization: "unit_length".to_string(),
        projection_version: "fixture-projection".to_string(),
        profile_version: "fixture-v2".to_string(),
        remote_consent_required: false,
        generation: 1,
        config_digest: "fixture-digest".to_string(),
    };
    let provider = provider_for_profile(&profile, &MemoryConfig::default()).expect("mock provider");
    assert_eq!(provider.spec().dims, 7);
    assert_eq!(provider.spec().version, "fixture-v2");
    let response = provider
        .embed_batch(&[EmbeddingRequestItem {
            request_item_id: "fixture".to_string(),
            text: "多语言 fixture 🧠".to_string(),
        }])
        .await
        .expect("mock embedding");
    assert_eq!(response[0].vector.len(), 7);
    let norm = response[0]
        .vector
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    assert!((norm - 1.0).abs() < 1.0e-5);
}

#[test]
fn memory_embedding_provider_falls_back_to_local_hash() {
    let mut cfg = MemoryConfig {
        embedding_model: "unknown-remote-provider".to_string(),
        embedding_dims: 1536,
        embedding_version: "remote-v1".to_string(),
        ..MemoryConfig::default()
    };
    let spec = embedding_spec_for_config(&cfg);
    assert_eq!(spec.model_id, LOCAL_HASH_MODEL_ID);
    assert_eq!(spec.dims, LOCAL_HASH_DIMS);
    assert_eq!(spec.version, LOCAL_HASH_VERSION);
    assert_eq!(spec.provider_kind, "local");

    cfg.embedding_model = LOCAL_HASH_MODEL_ID.to_string();
    let vector = embed_one_with_config(&cfg, "Réponds toujours en français")
        .expect("local hash fallback embeds");
    assert_eq!(vector.len(), LOCAL_HASH_DIMS);
}

#[test]
fn remote_response_validation_rejects_count_dimension_and_non_finite_values() {
    let requests = vec![EmbeddingRequestItem {
        request_item_id: "stable-item".to_string(),
        text: "fixture".to_string(),
    }];
    let spec = super::MemoryEmbeddingSpec {
        model_id: "fixture-model".to_string(),
        dims: 2,
        version: "v1".to_string(),
        normalization: "none".to_string(),
        provider_kind: "mock".to_string(),
    };
    let count_error = super::validate_remote_response(
        &requests,
        super::RemoteEmbeddingResponse { data: Vec::new() },
        &spec,
    )
    .unwrap_err();
    assert_eq!(
        count_error.error_code,
        "memory_embedding_response_count_mismatch"
    );
    let dims_error = super::validate_remote_response(
        &requests,
        super::RemoteEmbeddingResponse {
            data: vec![super::RemoteEmbeddingItem {
                index: 0,
                embedding: vec![1.0],
            }],
        },
        &spec,
    )
    .unwrap_err();
    assert_eq!(
        dims_error.error_code,
        "memory_embedding_response_dims_mismatch"
    );
    let finite_error = super::validate_remote_response(
        &requests,
        super::RemoteEmbeddingResponse {
            data: vec![super::RemoteEmbeddingItem {
                index: 0,
                embedding: vec![f32::NAN, 0.0],
            }],
        },
        &spec,
    )
    .unwrap_err();
    assert_eq!(
        finite_error.error_code,
        "memory_embedding_response_non_finite"
    );
    let index_error = super::validate_remote_response(
        &requests,
        super::RemoteEmbeddingResponse {
            data: vec![super::RemoteEmbeddingItem {
                index: 1,
                embedding: vec![1.0, 0.0],
            }],
        },
        &spec,
    )
    .unwrap_err();
    assert_eq!(
        index_error.error_code,
        "memory_embedding_response_index_mismatch"
    );

    let normalized_spec = super::MemoryEmbeddingSpec {
        normalization: "unit_length".to_string(),
        ..spec
    };
    let normalization_error = super::validate_remote_response(
        &requests,
        super::RemoteEmbeddingResponse {
            data: vec![super::RemoteEmbeddingItem {
                index: 0,
                embedding: vec![2.0, 0.0],
            }],
        },
        &normalized_spec,
    )
    .unwrap_err();
    assert_eq!(
        normalization_error.error_code,
        "memory_embedding_response_normalization_mismatch"
    );
}
