use std::hash::{Hash, Hasher};

use claw_core::config::MemoryConfig;

pub(crate) const LOCAL_HASH_MODEL_ID: &str = "local-hash-v1";
pub(crate) const LOCAL_HASH_DIMS: usize = 24;
pub(crate) const LOCAL_HASH_VERSION: &str = "local-hash-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryEmbeddingSpec {
    pub(crate) model_id: String,
    pub(crate) dims: usize,
    pub(crate) version: String,
}

pub(crate) trait MemoryEmbeddingProvider {
    fn spec(&self) -> MemoryEmbeddingSpec;
    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LocalHashEmbeddingProvider;

impl MemoryEmbeddingProvider for LocalHashEmbeddingProvider {
    fn spec(&self) -> MemoryEmbeddingSpec {
        local_hash_embedding_spec()
    }

    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| embed_text_locally(text)).collect())
    }
}

pub(crate) fn provider_for_config(_cfg: &MemoryConfig) -> Box<dyn MemoryEmbeddingProvider> {
    Box::new(LocalHashEmbeddingProvider)
}

pub(crate) fn embedding_spec_for_config(cfg: &MemoryConfig) -> MemoryEmbeddingSpec {
    provider_for_config(cfg).spec()
}

pub(crate) fn local_hash_embedding_spec() -> MemoryEmbeddingSpec {
    MemoryEmbeddingSpec {
        model_id: LOCAL_HASH_MODEL_ID.to_string(),
        dims: LOCAL_HASH_DIMS,
        version: LOCAL_HASH_VERSION.to_string(),
    }
}

pub(crate) fn embed_one_with_config(cfg: &MemoryConfig, text: &str) -> anyhow::Result<Vec<f32>> {
    let texts = vec![text.to_string()];
    let mut embedded = provider_for_config(cfg).embed(&texts)?;
    Ok(embedded.pop().unwrap_or_default())
}

pub(crate) fn embed_text_locally(text: &str) -> Vec<f32> {
    let mut vec = vec![0.0_f32; LOCAL_HASH_DIMS];
    for token in tokenize_text(text) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish() as usize;
        let idx = hash % LOCAL_HASH_DIMS;
        vec[idx] += 1.0;
    }
    normalize_vector(&mut vec);
    vec
}

pub(crate) fn tokenize_text(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut out = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_string())
        .collect::<Vec<_>>();
    let cjk = text
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .collect::<String>();
    let chars = cjk.chars().collect::<Vec<_>>();
    for w in chars.windows(2).take(16) {
        out.push(w.iter().collect::<String>());
    }
    out.sort();
    out.dedup();
    out
}

fn normalize_vector(vec: &mut [f32]) {
    let norm = vec
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt() as f32;
    if norm <= f32::EPSILON {
        return;
    }
    for item in vec.iter_mut() {
        *item /= norm;
    }
}

#[cfg(test)]
#[path = "embedding_tests.rs"]
mod tests;
