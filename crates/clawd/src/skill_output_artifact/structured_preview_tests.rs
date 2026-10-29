use super::*;

#[test]
fn large_strings_keep_actual_sizes_and_small_machine_fields_without_secret_fields() {
    let value = json!({"stdout":"a".repeat(40017),"exit_code":0,"complete":true,
        "stderr":"","api_key":"never-publish","note":"Authorization: Bearer never-publish",
        "nested":{"secret":"never-publish"}});
    let projected = project(&value);
    assert_eq!(projected["fields"]["exit_code"], 0);
    assert_eq!(projected["fields"]["complete"], true);
    assert_eq!(projected["fields"]["stderr"], "");
    assert_eq!(projected["string_bytes"]["stdout"], 40017);
    assert_eq!(projected["complete"], false);
    assert_eq!(projected["instruction_authority"], "none");
    assert!(!projected.to_string().contains("never-publish"));
    assert!(value["stdout"].as_str().unwrap().len() > MAX_PREVIEW_BYTES);
}

#[test]
fn many_or_large_keys_are_explicitly_omitted_within_the_projection_budget() {
    let object = (0..1000)
        .map(|i| (format!("field-{i}-{}", "x".repeat(100)), json!(i)))
        .collect::<Map<_, _>>();
    let projected = project(&Value::Object(object));
    assert!(projected.to_string().len() <= MAX_PREVIEW_BYTES);
    assert!(projected["omitted_top_level_fields"].as_u64().unwrap() > 0);
    assert!(project(&json!([1, 2, 3])).get("fields").is_none());
    assert_eq!(
        project(&json!({"stdout":"\u{4e2d}".repeat(1000)}))["string_bytes"]["stdout"],
        3000
    );
}
