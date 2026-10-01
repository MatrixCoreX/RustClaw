use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub fn prepare_hash_map_insert<K, V>(map: &mut HashMap<K, V>, key: &K, max_entries: usize)
where
    K: Clone + Eq + Hash,
{
    assert!(max_entries > 0, "bounded cache capacity must be positive");
    if map.contains_key(key) {
        return;
    }
    while map.len() >= max_entries {
        let Some(evicted) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&evicted);
    }
}

pub fn prepare_hash_set_insert<K>(set: &mut HashSet<K>, key: &K, max_entries: usize)
where
    K: Clone + Eq + Hash,
{
    assert!(max_entries > 0, "bounded cache capacity must be positive");
    if set.contains(key) {
        return;
    }
    while set.len() >= max_entries {
        let Some(evicted) = set.iter().next().cloned() else {
            break;
        };
        set.remove(&evicted);
    }
}

#[cfg(test)]
#[path = "bounded_cache_tests.rs"]
mod tests;
