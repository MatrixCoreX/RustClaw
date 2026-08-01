use std::collections::{BTreeMap, HashSet};

/// Preserve only non-sensitive parent values that the pinned package receipt
/// explicitly declared. These values are injected into `skill-runner`, which
/// applies the same exact receipt allowlist again before launching the skill.
/// Credentials remain broker-only and are never inherited through this path.
pub(crate) fn collect_declared_skill_env_pairs<I, K, V>(
    declared: &[String],
    source: I,
) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let declared: HashSet<&str> = declared.iter().map(String::as_str).collect();
    let mut kept = BTreeMap::new();
    for (key, value) in source {
        let key = key.as_ref();
        let value = value.as_ref();
        if !declared.contains(key)
            || value.is_empty()
            || skill_sdk::is_sensitive_runtime_environment_name(key)
        {
            continue;
        }
        kept.insert(key.to_string(), value.to_string());
    }
    kept.into_iter().collect()
}
