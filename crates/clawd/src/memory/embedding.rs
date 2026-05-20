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
mod tests {
    use super::{
        embed_one_with_config, embedding_spec_for_config, LocalHashEmbeddingProvider,
        MemoryEmbeddingProvider, LOCAL_HASH_DIMS, LOCAL_HASH_MODEL_ID, LOCAL_HASH_VERSION,
    };
    use claw_core::config::MemoryConfig;

    #[test]
    fn local_hash_embedding_provider_is_stable() {
        let provider = LocalHashEmbeddingProvider;
        let texts = vec!["以后默认用中文回复".to_string()];
        let first = provider.embed(&texts).expect("first embed");
        let second = provider.embed(&texts).expect("second embed");

        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].len(), LOCAL_HASH_DIMS);
        assert!(first[0].iter().any(|value| *value > 0.0));
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

        cfg.embedding_model = LOCAL_HASH_MODEL_ID.to_string();
        let vector = embed_one_with_config(&cfg, "Réponds toujours en français")
            .expect("local hash fallback embeds");
        assert_eq!(vector.len(), LOCAL_HASH_DIMS);
    }
}
