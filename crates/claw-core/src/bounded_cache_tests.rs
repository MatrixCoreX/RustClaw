use super::*;

#[test]
fn map_insert_budget_preserves_existing_entries_and_makes_room_for_new_entries() {
    let mut map = HashMap::from([(1_u64, "one"), (2_u64, "two")]);
    prepare_hash_map_insert(&mut map, &1, 2);
    assert_eq!(map.len(), 2);

    prepare_hash_map_insert(&mut map, &3, 2);
    map.insert(3, "three");
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&3), Some(&"three"));
}

#[test]
fn set_insert_budget_makes_room_without_growing_past_the_limit() {
    let mut set = HashSet::from(["one".to_string(), "two".to_string()]);
    prepare_hash_set_insert(&mut set, &"three".to_string(), 2);
    set.insert("three".to_string());
    assert_eq!(set.len(), 2);
    assert!(set.contains("three"));
}
