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
