use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use unicode_normalization::UnicodeNormalization;

pub const LOCAL_HASH_MODEL_ID: &str = "local-hash-v3";
pub const LOCAL_HASH_DIMS: usize = 24;
pub const LOCAL_HASH_VERSION: &str = "local-hash-v3";
pub const LOCAL_HASH_NORMALIZATION: &str = "unit_length";
pub const LOCAL_HASH_PROFILE_ID: &str = "retrieval/local-hash-v3";
pub const RETRIEVAL_TOKENIZER_VERSION: &str = "retrieval-tokenizer-v3-nfkc";
pub const DEFAULT_RETRIEVAL_TOKEN_BUDGET: usize = 4_096;
pub const VECTOR_ANN_PROJECTION_VERSION: &str = "lsh-hyperplane-v2";
pub const VECTOR_ANN_BANDS: usize = 8;
pub const VECTOR_ANN_BITS_PER_BAND: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalTokenProjectionMetadata {
    pub tokenizer_version: String,
    pub token_budget: usize,
    pub retained_unique_tokens: usize,
    pub candidate_tokens: usize,
    pub dropped_candidates: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalTokenProjection {
    pub tokens: Vec<String>,
    pub metadata: RetrievalTokenProjectionMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingProfile {
    pub profile_id: String,
    pub provider_kind: String,
    pub model_id: String,
    pub dimensions: usize,
    pub normalization: String,
    pub version: String,
    pub cross_language_semantic: bool,
}

impl EmbeddingProfile {
    pub fn validate(&self) -> Result<(), EmbeddingBrokerError> {
        if self.profile_id.trim().is_empty()
            || self.provider_kind.trim().is_empty()
            || self.model_id.trim().is_empty()
            || self.version.trim().is_empty()
            || self.dimensions == 0
            || !matches!(self.normalization.as_str(), "unit_length" | "none")
        {
            return Err(EmbeddingBrokerError::new("embedding_profile_invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingPurpose {
    Query,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingOutboundPolicy {
    LocalOnly,
    RemoteAllowed,
}

#[derive(Debug, Clone, Copy)]
pub struct EmbeddingRequest<'a> {
    pub text: &'a str,
    pub purpose: EmbeddingPurpose,
    pub principal_id: &'a str,
    pub outbound_policy: EmbeddingOutboundPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingOutput {
    pub profile: EmbeddingProfile,
    pub vector: Vec<f32>,
    pub content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingBrokerError {
    pub error_code: &'static str,
    pub retryable: bool,
}

impl EmbeddingBrokerError {
    pub const fn new(error_code: &'static str) -> Self {
        Self {
            error_code,
            retryable: false,
        }
    }
}

impl std::fmt::Display for EmbeddingBrokerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.error_code)
    }
}

impl std::error::Error for EmbeddingBrokerError {}

pub trait EmbeddingBroker: Send + Sync {
    fn profile(&self) -> EmbeddingProfile;
    fn embed(&self, request: EmbeddingRequest<'_>)
        -> Result<EmbeddingOutput, EmbeddingBrokerError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalHashEmbeddingBroker;

impl EmbeddingBroker for LocalHashEmbeddingBroker {
    fn profile(&self) -> EmbeddingProfile {
        local_hash_profile()
    }

    fn embed(
        &self,
        request: EmbeddingRequest<'_>,
    ) -> Result<EmbeddingOutput, EmbeddingBrokerError> {
        if request.principal_id.trim().is_empty() || request.text.chars().any(|ch| ch == '\0') {
            return Err(EmbeddingBrokerError::new("embedding_request_invalid"));
        }
        if request.outbound_policy != EmbeddingOutboundPolicy::LocalOnly {
            return Err(EmbeddingBrokerError::new(
                "embedding_local_provider_outbound_policy_invalid",
            ));
        }
        let _purpose = request.purpose;
        Ok(EmbeddingOutput {
            profile: self.profile(),
            vector: compute_local_hash(request.text),
            content_digest: format!("sha256:{:x}", Sha256::digest(request.text.as_bytes())),
        })
    }
}

pub fn local_hash_profile() -> EmbeddingProfile {
    EmbeddingProfile {
        profile_id: LOCAL_HASH_PROFILE_ID.to_string(),
        provider_kind: "local".to_string(),
        model_id: LOCAL_HASH_MODEL_ID.to_string(),
        dimensions: LOCAL_HASH_DIMS,
        normalization: LOCAL_HASH_NORMALIZATION.to_string(),
        version: LOCAL_HASH_VERSION.to_string(),
        cross_language_semantic: false,
    }
}

pub fn embed_local_hash_with_context(
    text: &str,
    purpose: EmbeddingPurpose,
    principal_id: &str,
) -> Result<EmbeddingOutput, EmbeddingBrokerError> {
    LocalHashEmbeddingBroker.embed(EmbeddingRequest {
        text,
        purpose,
        principal_id,
        outbound_policy: EmbeddingOutboundPolicy::LocalOnly,
    })
}

pub fn embed_local_hash(text: &str) -> Vec<f32> {
    compute_local_hash(text)
}

pub fn vector_ann_buckets(vector: &[f32]) -> Vec<u16> {
    (0..VECTOR_ANN_BANDS)
        .map(|band| {
            let mut bucket = 0_u16;
            for bit in 0..VECTOR_ANN_BITS_PER_BAND {
                let mut projection = 0.0_f32;
                for (dimension, value) in vector.iter().copied().enumerate() {
                    let seed = ((band as u64) << 40)
                        ^ ((bit as u64) << 32)
                        ^ dimension as u64
                        ^ 0x9e37_79b9_7f4a_7c15;
                    let weight = if splitmix64(seed) & 1 == 0 { -1.0 } else { 1.0 };
                    projection += value * weight;
                }
                if projection >= 0.0 {
                    bucket |= 1_u16 << bit;
                }
            }
            bucket
        })
        .collect()
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn compute_local_hash(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; LOCAL_HASH_DIMS];
    for token in tokenize_retrieval_text(text) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        let index = (hasher.finish() as usize) % LOCAL_HASH_DIMS;
        vector[index] += 1.0;
    }
    normalize_unit_length(&mut vector);
    vector
}

pub fn tokenize_retrieval_text(text: &str) -> Vec<String> {
    project_retrieval_tokens(text, DEFAULT_RETRIEVAL_TOKEN_BUDGET).tokens
}

pub fn project_retrieval_tokens(text: &str, token_budget: usize) -> RetrievalTokenProjection {
    let normalized = text.nfkc().collect::<String>();
    let mut collector = RetrievalTokenCollector::new(token_budget.max(1));
    let mut word = String::new();
    let mut script_run = Vec::new();
    let mut exact_candidate = String::new();

    for character in normalized.chars() {
        if character.is_whitespace() {
            push_exact_identifier_token(&mut collector, &mut exact_candidate);
        } else {
            exact_candidate.push(character);
        }
        if character.is_alphanumeric() || character == '_' {
            for lowered in character.to_lowercase() {
                word.push(lowered);
            }
        } else {
            push_word_token(&mut collector, &mut word);
        }

        if is_multilingual_bigram_character(character) {
            script_run.push(character);
        } else {
            push_script_bigrams(&mut collector, &mut script_run);
        }

        if is_semantic_symbol(character) {
            collector.push(format!("symbol_{:x}", character as u32));
        }
    }
    push_word_token(&mut collector, &mut word);
    push_script_bigrams(&mut collector, &mut script_run);
    push_exact_identifier_token(&mut collector, &mut exact_candidate);
    collector.finish()
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        0.0
    } else {
        (dot / (left_norm * right_norm)).clamp(-1.0, 1.0)
    }
}

pub fn retrieval_evidence_id(
    source_ref: &str,
    document_digest: &str,
    namespace_revision: u64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(RETRIEVAL_TOKENIZER_VERSION.as_bytes());
    digest.update([0]);
    digest.update(source_ref.as_bytes());
    digest.update([0]);
    digest.update(document_digest.as_bytes());
    digest.update([0]);
    digest.update(namespace_revision.to_be_bytes());
    format!("evidence:sha256:{:x}", digest.finalize())
}

struct RetrievalTokenCollector {
    budget: usize,
    tokens: Vec<String>,
    retained: HashSet<String>,
    candidate_tokens: usize,
    dropped_candidates: usize,
}

impl RetrievalTokenCollector {
    fn new(budget: usize) -> Self {
        Self {
            budget,
            tokens: Vec::with_capacity(budget.min(256)),
            retained: HashSet::with_capacity(budget.min(256)),
            candidate_tokens: 0,
            dropped_candidates: 0,
        }
    }

    fn push(&mut self, token: String) {
        if token.is_empty() {
            return;
        }
        self.candidate_tokens = self.candidate_tokens.saturating_add(1);
        if self.retained.contains(&token) {
            return;
        }
        if self.tokens.len() >= self.budget {
            self.dropped_candidates = self.dropped_candidates.saturating_add(1);
            return;
        }
        self.retained.insert(token.clone());
        self.tokens.push(token);
    }

    fn finish(self) -> RetrievalTokenProjection {
        RetrievalTokenProjection {
            metadata: RetrievalTokenProjectionMetadata {
                tokenizer_version: RETRIEVAL_TOKENIZER_VERSION.to_string(),
                token_budget: self.budget,
                retained_unique_tokens: self.tokens.len(),
                candidate_tokens: self.candidate_tokens,
                dropped_candidates: self.dropped_candidates,
                truncated: self.dropped_candidates > 0,
            },
            tokens: self.tokens,
        }
    }
}

fn push_word_token(out: &mut RetrievalTokenCollector, word: &mut String) {
    if word.chars().count() >= 2 {
        out.push(std::mem::take(word));
    } else {
        word.clear();
    }
}

fn push_script_bigrams(out: &mut RetrievalTokenCollector, script_run: &mut Vec<char>) {
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
    matches!(
        character as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF
    )
}

fn push_exact_identifier_token(out: &mut RetrievalTokenCollector, candidate: &mut String) {
    let value = candidate.trim_matches(|character: char| {
        matches!(
            character,
            ',' | ';' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    });
    let looks_exact = value.chars().count() >= 3
        && (value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':' | '.' | '#' | '@' | '_'))
            || has_mixed_case(value)
            || looks_like_digest(value));
    if looks_exact {
        let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
        out.push(format!("exact_{}", &digest[..24]));
    }
    candidate.clear();
}

fn has_mixed_case(candidate: &str) -> bool {
    candidate.chars().any(char::is_uppercase) && candidate.chars().any(char::is_lowercase)
}

fn looks_like_digest(candidate: &str) -> bool {
    candidate.len() >= 12 && candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_unit_length(vector: &mut [f32]) {
    let norm = vector
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt() as f32;
    if norm <= f32::EPSILON {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multilingual_tokens_cover_the_end_of_long_script_runs() {
        let text = "这是一个足够长的中文检索语句用于验证尾部独特事实仍然能够建立索引尾部答案";
        let tokens = tokenize_retrieval_text(text);
        assert!(tokens.contains(&"答案".to_string()));
        assert!(tokens.contains(&"尾部".to_string()));
    }

    #[test]
    fn token_projection_is_ordered_bounded_and_reports_truncation() {
        let projection = project_retrieval_tokens("alpha beta gamma delta", 2);
        assert_eq!(projection.tokens, vec!["alpha", "beta"]);
        assert_eq!(projection.metadata.token_budget, 2);
        assert_eq!(projection.metadata.retained_unique_tokens, 2);
        assert!(projection.metadata.truncated);
        assert!(projection.metadata.dropped_candidates >= 2);
        assert_eq!(
            projection.metadata.tokenizer_version,
            RETRIEVAL_TOKENIZER_VERSION
        );
    }

    #[test]
    fn tokenizer_normalizes_unicode_without_language_phrase_tables() {
        assert_eq!(
            tokenize_retrieval_text("Ｃａｆｅ́１２３"),
            tokenize_retrieval_text("Café123")
        );
        let tokens = tokenize_retrieval_text("中文, 日本語。한국어! English-42 🧠");
        for expected in ["中文", "日本", "한국", "english", "42", "symbol_1f9e0"] {
            assert!(tokens.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn retrieval_evidence_identity_is_stable_and_revision_bound() {
        let first = retrieval_evidence_id("kb://docs/doc/chunk-1", "digest", 7);
        assert_eq!(
            first,
            retrieval_evidence_id("kb://docs/doc/chunk-1", "digest", 7)
        );
        assert_ne!(
            first,
            retrieval_evidence_id("kb://docs/doc/chunk-1", "digest", 8)
        );
        assert!(first.starts_with("evidence:sha256:"));
    }

    #[test]
    fn local_embedding_is_versioned_and_unit_length() {
        let output = embed_local_hash_with_context(
            "中文 English /tmp/demo.rs",
            EmbeddingPurpose::Document,
            "user:test",
        )
        .expect("local broker output");
        let vector = output.vector;
        assert_eq!(vector.len(), LOCAL_HASH_DIMS);
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.0001);
        assert_eq!(LOCAL_HASH_MODEL_ID, LOCAL_HASH_VERSION);
        assert_eq!(output.profile, local_hash_profile());
        assert!(output.content_digest.starts_with("sha256:"));
        assert!(!output.profile.cross_language_semantic);
    }

    #[test]
    fn local_broker_enforces_tenant_and_outbound_contract() {
        let broker = LocalHashEmbeddingBroker;
        assert_eq!(
            broker
                .embed(EmbeddingRequest {
                    text: "query",
                    purpose: EmbeddingPurpose::Query,
                    principal_id: "",
                    outbound_policy: EmbeddingOutboundPolicy::LocalOnly,
                })
                .expect_err("missing principal must fail")
                .error_code,
            "embedding_request_invalid"
        );
        assert_eq!(
            broker
                .embed(EmbeddingRequest {
                    text: "query",
                    purpose: EmbeddingPurpose::Query,
                    principal_id: "user:test",
                    outbound_policy: EmbeddingOutboundPolicy::RemoteAllowed,
                })
                .expect_err("local provider must not accept remote policy")
                .error_code,
            "embedding_local_provider_outbound_policy_invalid"
        );
    }

    #[test]
    fn ann_projection_is_deterministic_bounded_and_similarity_sensitive() {
        let first = embed_local_hash("alpha beta gamma");
        let similar = embed_local_hash("alpha beta gamma delta");
        let unrelated = embed_local_hash("東京 天気 予報");
        let buckets = vector_ann_buckets(&first);
        assert_eq!(buckets, vector_ann_buckets(&first));
        assert_eq!(buckets.len(), VECTOR_ANN_BANDS);
        assert!(buckets
            .iter()
            .all(|bucket| *bucket < (1_u16 << VECTOR_ANN_BITS_PER_BAND)));
        let similar_matches = buckets
            .iter()
            .zip(vector_ann_buckets(&similar))
            .filter(|(left, right)| **left == *right)
            .count();
        let unrelated_matches = buckets
            .iter()
            .zip(vector_ann_buckets(&unrelated))
            .filter(|(left, right)| **left == *right)
            .count();
        assert!(similar_matches >= unrelated_matches);
        assert!(similar_matches > 0);
    }
}
