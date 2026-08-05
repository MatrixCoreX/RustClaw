use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::time::Duration;

use claw_core::config::MemoryConfig;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

pub(crate) const LOCAL_HASH_MODEL_ID: &str = "local-hash-v2";
pub(crate) const LOCAL_HASH_DIMS: usize = 24;
pub(crate) const LOCAL_HASH_VERSION: &str = "local-hash-v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct MemoryEmbeddingSpec {
    pub(crate) model_id: String,
    pub(crate) dims: usize,
    pub(crate) version: String,
    pub(crate) normalization: String,
    pub(crate) provider_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbeddingRequestItem {
    pub(crate) request_item_id: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmbeddingResponseItem {
    pub(crate) request_item_id: String,
    pub(crate) vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbeddingProviderError {
    pub(crate) error_code: &'static str,
    pub(crate) retryable: bool,
    pub(crate) retry_after_seconds: Option<u64>,
    pub(crate) status_code: Option<u16>,
}

impl std::fmt::Display for EmbeddingProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.error_code)
    }
}

impl std::error::Error for EmbeddingProviderError {}

pub(crate) trait MemoryEmbeddingProvider: Send + Sync {
    fn spec(&self) -> MemoryEmbeddingSpec;

    fn embed_batch<'a>(
        &'a self,
        items: &'a [EmbeddingRequestItem],
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<EmbeddingResponseItem>, EmbeddingProviderError>>
                + Send
                + 'a,
        >,
    >;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LocalHashEmbeddingProvider;

impl MemoryEmbeddingProvider for LocalHashEmbeddingProvider {
    fn spec(&self) -> MemoryEmbeddingSpec {
        local_hash_embedding_spec()
    }

    fn embed_batch<'a>(
        &'a self,
        items: &'a [EmbeddingRequestItem],
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<EmbeddingResponseItem>, EmbeddingProviderError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            Ok(items
                .iter()
                .map(|item| EmbeddingResponseItem {
                    request_item_id: item.request_item_id.clone(),
                    vector: embed_text_locally(&item.text),
                })
                .collect())
        })
    }
}

#[derive(Debug, Clone)]
struct MockEmbeddingProvider {
    spec: MemoryEmbeddingSpec,
}

impl MemoryEmbeddingProvider for MockEmbeddingProvider {
    fn spec(&self) -> MemoryEmbeddingSpec {
        self.spec.clone()
    }

    fn embed_batch<'a>(
        &'a self,
        items: &'a [EmbeddingRequestItem],
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<EmbeddingResponseItem>, EmbeddingProviderError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            Ok(items
                .iter()
                .map(|item| EmbeddingResponseItem {
                    request_item_id: item.request_item_id.clone(),
                    vector: embed_text_with_spec(&item.text, &self.spec),
                })
                .collect())
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteHttpEmbeddingProvider {
    client: reqwest::Client,
    endpoint: String,
    credential: String,
    spec: MemoryEmbeddingSpec,
}

impl RemoteHttpEmbeddingProvider {
    pub(crate) fn from_profile(
        profile: &super::vector_store::MemoryEmbeddingProfile,
        config: &MemoryConfig,
    ) -> Result<Self, EmbeddingProviderError> {
        let endpoint_ref = profile.endpoint_ref.as_deref().ok_or_else(|| {
            provider_error("memory_embedding_endpoint_ref_missing", false, None, None)
        })?;
        let credential_ref = profile.credential_ref.as_deref().ok_or_else(|| {
            provider_error("memory_embedding_credential_ref_missing", false, None, None)
        })?;
        let endpoint = resolve_endpoint_ref(endpoint_ref)?;
        let credential = std::env::var(credential_ref).map_err(|_| {
            provider_error("memory_embedding_credential_unavailable", false, None, None)
        })?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(
                config.embedding_connect_timeout_ms.clamp(100, 120_000),
            ))
            .read_timeout(Duration::from_millis(
                config.embedding_idle_timeout_ms.clamp(100, 120_000),
            ))
            .timeout(Duration::from_millis(
                config.embedding_query_timeout_ms.clamp(100, 120_000),
            ))
            .build()
            .map_err(|_| {
                provider_error("memory_embedding_client_build_failed", false, None, None)
            })?;
        Ok(Self {
            client,
            endpoint,
            credential,
            spec: MemoryEmbeddingSpec {
                model_id: profile.model_name.clone(),
                dims: profile.dimensions,
                version: profile.profile_version.clone(),
                normalization: profile.normalization.clone(),
                provider_kind: profile.provider_kind.clone(),
            },
        })
    }
}

impl MemoryEmbeddingProvider for RemoteHttpEmbeddingProvider {
    fn spec(&self) -> MemoryEmbeddingSpec {
        self.spec.clone()
    }

    fn embed_batch<'a>(
        &'a self,
        items: &'a [EmbeddingRequestItem],
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Vec<EmbeddingResponseItem>, EmbeddingProviderError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if items.is_empty() {
                return Ok(Vec::new());
            }
            let response = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.credential)
                .json(&json!({
                    "model": self.spec.model_id,
                    "input": items.iter().map(|item| item.text.as_str()).collect::<Vec<_>>(),
                }))
                .send()
                .await
                .map_err(|error| {
                    provider_error(
                        if error.is_timeout() {
                            "memory_embedding_provider_timeout"
                        } else {
                            "memory_embedding_provider_transport"
                        },
                        true,
                        None,
                        None,
                    )
                })?;
            let status = response.status();
            let retry_after_seconds = parse_retry_after_seconds(response.headers());
            if status.is_redirection() {
                return Err(provider_error(
                    "memory_embedding_provider_redirect_blocked",
                    false,
                    retry_after_seconds,
                    Some(status.as_u16()),
                ));
            }
            if !status.is_success() {
                return Err(provider_error(
                    match status.as_u16() {
                        413 => "memory_embedding_payload_too_large",
                        429 => "memory_embedding_rate_limited",
                        500..=599 => "memory_embedding_provider_unavailable",
                        _ => "memory_embedding_provider_rejected",
                    },
                    status.as_u16() == 429 || status.is_server_error(),
                    retry_after_seconds,
                    Some(status.as_u16()),
                ));
            }
            let body = response
                .json::<RemoteEmbeddingResponse>()
                .await
                .map_err(|_| {
                    provider_error(
                        "memory_embedding_response_invalid",
                        false,
                        None,
                        Some(status.as_u16()),
                    )
                })?;
            validate_remote_response(items, body, &self.spec)
        })
    }
}

#[derive(Debug, Deserialize)]
struct RemoteEmbeddingResponse {
    data: Vec<RemoteEmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct RemoteEmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

fn validate_remote_response(
    request: &[EmbeddingRequestItem],
    mut response: RemoteEmbeddingResponse,
    spec: &MemoryEmbeddingSpec,
) -> Result<Vec<EmbeddingResponseItem>, EmbeddingProviderError> {
    if response.data.len() != request.len() {
        return Err(provider_error(
            "memory_embedding_response_count_mismatch",
            false,
            None,
            None,
        ));
    }
    response.data.sort_by_key(|item| item.index);
    let mut out = Vec::with_capacity(request.len());
    for (expected_index, (request_item, response_item)) in
        request.iter().zip(response.data.into_iter()).enumerate()
    {
        if response_item.index != expected_index {
            return Err(provider_error(
                "memory_embedding_response_index_mismatch",
                false,
                None,
                None,
            ));
        }
        super::vector_store::validate_vector(
            &response_item.embedding,
            spec.dims,
            &spec.normalization,
        )
        .map_err(|error| {
            provider_error(
                match error.to_string().as_str() {
                    "memory_vector_dims_mismatch" => "memory_embedding_response_dims_mismatch",
                    "memory_vector_non_finite" => "memory_embedding_response_non_finite",
                    "memory_vector_not_normalized" => {
                        "memory_embedding_response_normalization_mismatch"
                    }
                    _ => "memory_embedding_response_invalid",
                },
                false,
                None,
                None,
            )
        })?;
        out.push(EmbeddingResponseItem {
            request_item_id: request_item.request_item_id.clone(),
            vector: response_item.embedding,
        });
    }
    Ok(out)
}

pub(crate) fn provider_for_profile(
    profile: &super::vector_store::MemoryEmbeddingProfile,
    config: &MemoryConfig,
) -> Result<Box<dyn MemoryEmbeddingProvider>, EmbeddingProviderError> {
    match profile.provider_kind.as_str() {
        "local" => Ok(Box::new(LocalHashEmbeddingProvider)),
        "mock" => Ok(Box::new(MockEmbeddingProvider {
            spec: MemoryEmbeddingSpec {
                model_id: profile.model_name.clone(),
                dims: profile.dimensions,
                version: profile.profile_version.clone(),
                normalization: profile.normalization.clone(),
                provider_kind: profile.provider_kind.clone(),
            },
        })),
        "remote_http" => Ok(Box::new(RemoteHttpEmbeddingProvider::from_profile(
            profile, config,
        )?)),
        _ => Err(provider_error(
            "memory_embedding_provider_kind_invalid",
            false,
            None,
            None,
        )),
    }
}

pub(crate) fn embedding_spec_for_config(cfg: &MemoryConfig) -> MemoryEmbeddingSpec {
    if cfg.embedding_provider_kind.trim() == "local" {
        local_hash_embedding_spec()
    } else {
        MemoryEmbeddingSpec {
            model_id: cfg.embedding_model.clone(),
            dims: cfg.embedding_dims,
            version: cfg.embedding_version.clone(),
            normalization: cfg.embedding_normalization.clone(),
            provider_kind: cfg.embedding_provider_kind.clone(),
        }
    }
}

pub(crate) fn local_hash_embedding_spec() -> MemoryEmbeddingSpec {
    MemoryEmbeddingSpec {
        model_id: LOCAL_HASH_MODEL_ID.to_string(),
        dims: LOCAL_HASH_DIMS,
        version: LOCAL_HASH_VERSION.to_string(),
        normalization: "unit_length".to_string(),
        provider_kind: "local".to_string(),
    }
}

pub(crate) fn embed_one_with_config(cfg: &MemoryConfig, text: &str) -> anyhow::Result<Vec<f32>> {
    if cfg.embedding_provider_kind.trim() != "local" {
        anyhow::bail!("memory_embedding_async_provider_required");
    }
    Ok(embed_text_locally(text))
}

pub(crate) fn embed_text_locally(text: &str) -> Vec<f32> {
    embed_text_with_spec(text, &local_hash_embedding_spec())
}

fn embed_text_with_spec(text: &str, spec: &MemoryEmbeddingSpec) -> Vec<f32> {
    let mut vector = vec![0.0_f32; spec.dims];
    for token in tokenize_text(text) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish() as usize;
        let index = hash % spec.dims;
        vector[index] += 1.0;
    }
    if spec.normalization == "unit_length" {
        normalize_vector(&mut vector);
    }
    vector
}

pub(crate) fn tokenize_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut script_run = Vec::new();

    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            for lowered in character.to_lowercase() {
                word.push(lowered);
            }
        } else {
            push_word_token(&mut out, &mut word);
        }

        if is_multilingual_bigram_character(character) {
            script_run.push(character);
        } else {
            push_script_bigrams(&mut out, &mut script_run);
        }

        if is_semantic_symbol(character) {
            out.push(format!("symbol_{:x}", character as u32));
        }
    }
    push_word_token(&mut out, &mut word);
    push_script_bigrams(&mut out, &mut script_run);
    push_exact_identifier_tokens(&mut out, text);
    out.sort();
    out.dedup();
    out
}

fn push_word_token(out: &mut Vec<String>, word: &mut String) {
    if word.chars().count() >= 2 {
        out.push(std::mem::take(word));
    } else {
        word.clear();
    }
}

fn push_script_bigrams(out: &mut Vec<String>, script_run: &mut Vec<char>) {
    for window in script_run.windows(2) {
        out.push(window.iter().collect());
    }
    script_run.clear();
}

fn is_multilingual_bigram_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0x31F0..=0x31FF
            | 0xFF65..=0xFF9F
            | 0x1100..=0x11FF
            | 0x3130..=0x318F
            | 0xAC00..=0xD7AF
    )
}

fn is_semantic_symbol(character: char) -> bool {
    let codepoint = character as u32;
    matches!(
        codepoint,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF
    )
}

fn push_exact_identifier_tokens(out: &mut Vec<String>, text: &str) {
    for candidate in text.split_whitespace() {
        let candidate = candidate.trim_matches(|character: char| {
            matches!(
                character,
                ',' | ';' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        });
        let looks_exact = candidate.chars().count() >= 3
            && (candidate
                .chars()
                .any(|character| matches!(character, '/' | '\\' | ':' | '.' | '#' | '@' | '_'))
                || has_mixed_case(candidate)
                || looks_like_digest(candidate));
        if looks_exact {
            let digest = Sha256::digest(candidate.as_bytes());
            out.push(format!("exact_{}", hex::encode(&digest[..12])));
        }
    }
}

fn has_mixed_case(candidate: &str) -> bool {
    candidate.chars().any(char::is_uppercase) && candidate.chars().any(char::is_lowercase)
}

fn looks_like_digest(candidate: &str) -> bool {
    candidate.len() >= 12 && candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_vector(vector: &mut [f32]) {
    let norm = vector
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt() as f32;
    if norm <= f32::EPSILON {
        return;
    }
    for item in vector.iter_mut() {
        *item /= norm;
    }
}

fn resolve_endpoint_ref(endpoint_ref: &str) -> Result<String, EmbeddingProviderError> {
    let endpoint_ref = endpoint_ref.trim();
    if endpoint_ref.starts_with("https://") || endpoint_ref.starts_with("http://127.0.0.1:") {
        return Ok(endpoint_ref.to_string());
    }
    std::env::var(endpoint_ref)
        .map_err(|_| provider_error("memory_embedding_endpoint_unavailable", false, None, None))
}

fn parse_retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn provider_error(
    error_code: &'static str,
    retryable: bool,
    retry_after_seconds: Option<u64>,
    status_code: Option<u16>,
) -> EmbeddingProviderError {
    EmbeddingProviderError {
        error_code,
        retryable,
        retry_after_seconds,
        status_code,
    }
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
