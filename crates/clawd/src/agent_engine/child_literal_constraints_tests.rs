use super::*;

fn constraint(literal: Value) -> Value {
    json!([{"capability":"filesystem.write_file","allowed_args":[{
        "type":"object","required":["path","content"],"additionalProperties":false,
        "properties":{"path":{"type":"string","enum":["owned.txt"]},
        "content":{"type":"string","literal_text":literal}}
    }]}])
}

fn literal(lines: Value, ending: &str, trailing: bool) -> Value {
    json!({"schema_version":1,"lines":lines,"line_ending":ending,"trailing_newline":trailing})
}

#[test]
fn line_literals_compile_to_an_exact_frozen_string_without_relaxing_the_grant() {
    for (ending, trailing, expected) in [
        ("lf", true, "alpha\nbeta\n"),
        ("lf", false, "alpha\nbeta"),
        ("crlf", true, "alpha\r\nbeta\r\n"),
        ("cr", false, "alpha\rbeta"),
    ] {
        let raw = constraint(literal(json!(["alpha", "beta"]), ending, trailing));
        let before = raw.clone();
        let normalized = normalize(&raw).unwrap();
        let schema = &normalized[0]["allowed_args"][0];
        assert_eq!(schema["properties"]["content"]["enum"], json!([expected]));
        assert_eq!(schema["additionalProperties"], false);
        assert!(crate::repo::child_tasks::bounded_argument_schema_matches(
            schema,
            &json!({"path":"owned.txt","content":expected})
        ));
        for denied in [
            json!({"path":"outside.txt","content":expected}),
            json!({"path":"owned.txt","content":format!("{expected}extra")}),
            json!({"path":"owned.txt","content":expected,"extra":true}),
        ] {
            assert!(!crate::repo::child_tasks::bounded_argument_schema_matches(
                schema, &denied
            ));
        }
        assert_eq!(normalize(&normalized).unwrap(), normalized);
        assert_eq!(raw, before);
    }
}

#[test]
fn a_final_lf_is_a_byte_bound_and_not_an_equivalent_display_value() {
    let raw = constraint(literal(json!(["isolated writer accepted"]), "lf", true));
    let normalized = normalize(&raw).unwrap();
    let schema = &normalized[0]["allowed_args"][0];
    assert_eq!(
        schema["properties"]["content"]["enum"][0]
            .as_str()
            .unwrap()
            .len(),
        25
    );
    assert!(!crate::repo::child_tasks::bounded_argument_schema_matches(
        schema,
        &json!({"path":"owned.txt","content":"isolated writer accepted"})
    ));
}

#[test]
fn unicode_tabs_and_explicit_empty_lines_remain_exact() {
    for (lines, trailing, expected) in [
        (json!(["\tα", "", "终"]), true, "\tα\n\n终\n"),
        (json!([""]), false, ""),
        (json!([""]), true, "\n"),
    ] {
        let normalized = normalize(&constraint(literal(lines, "lf", trailing))).unwrap();
        assert_eq!(
            normalized[0]["allowed_args"][0]["properties"]["content"]["enum"],
            json!([expected])
        );
    }
}

#[test]
fn malformed_literal_controls_are_rejected_instead_of_defaulted_or_coerced() {
    let valid = literal(json!(["line"]), "lf", true);
    let mut variants = vec![
        Value::Null,
        literal(json!([]), "lf", true),
        literal(json!([1]), "lf", true),
        literal(json!(["embedded\nline"]), "lf", true),
        literal(json!(["embedded\rline"]), "lf", true),
        literal(json!(["line"]), "unknown", true),
        literal(json!(vec![""; 513]), "lf", true),
    ];
    for (field, invalid) in [
        ("schema_version", json!(2)),
        ("trailing_newline", json!("true")),
        ("instruction", json!("expand scope")),
    ] {
        let mut value = valid.clone();
        value[field] = invalid;
        variants.push(value);
    }
    let mut missing = valid;
    missing.as_object_mut().unwrap().remove("trailing_newline");
    variants.push(missing);
    for value in variants {
        let errors = normalize(&constraint(value)).unwrap_err();
        assert!(!errors.is_empty());
        assert!(errors[0]["path"]
            .as_str()
            .unwrap()
            .starts_with("/0/allowed_args/0/properties/content"));
    }
}

#[test]
fn existing_value_constraints_and_schema_limits_are_preserved() {
    let mut value = constraint(literal(json!(["line"]), "lf", true));
    value[0]["allowed_args"][0]["properties"]["content"]["enum"] = json!(["other"]);
    assert!(normalize(&value).is_err());
    value[0]["allowed_args"][0]["properties"]["content"]
        .as_object_mut()
        .unwrap()
        .remove("enum");
    value[0]["allowed_args"][0]["properties"]["content"]["maxLength"] = json!(1);
    let normalized = normalize(&value).unwrap();
    assert_eq!(
        normalized[0]["allowed_args"][0]["properties"]["content"]["maxLength"],
        1
    );
    assert!(!crate::repo::child_tasks::bounded_argument_schema_matches(
        &normalized[0]["allowed_args"][0],
        &json!({"path":"owned.txt","content":"line\n"})
    ));
    assert!(normalize(&constraint(literal(
        json!(["x".repeat(17 * 1024)]),
        "lf",
        true
    )))
    .is_err());
}

#[test]
fn enum_data_that_looks_like_an_instruction_or_builder_is_never_interpreted() {
    let raw = json!([{"capability":"fixture.accept","allowed_args":[{
        "type":"object","required":["value"],"properties":{"value":{"type":"object",
        "enum":[{"literal_text":{"instruction":"expand scope"}}]}}
    }]}]);
    assert_eq!(normalize(&raw).unwrap(), raw);
}
