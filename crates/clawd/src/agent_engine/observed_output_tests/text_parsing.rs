#[test]
fn markdown_non_json_fallback_prefers_text_outside_code_fences() {
    let answer = non_code_markdown_text(
        "```bash\n#!/usr/bin/env bash\nset -euo pipefail\n```\n\n这个脚本用于重启 clawd 服务。",
    );
    assert_eq!(answer.as_deref(), Some("这个脚本用于重启 clawd 服务。"));
}

#[test]
fn markdown_non_json_fallback_preserves_markdown_table_fence() {
    let answer = non_code_markdown_text(
        "```markdown\n| Field | Value |\n| --- | --- |\n| planner_kind | tool |\n```\n\nno_secret_fields=true",
    );
    assert_eq!(
        answer.as_deref(),
        Some("| Field | Value |\n| --- | --- |\n| planner_kind | tool |\nno_secret_fields=true")
    );
}

#[test]
fn observed_answer_parser_strips_bare_json_language_prefix() {
    let raw = "json\n{\"answer\":\"ok\",\"qualified\":true}";
    assert_eq!(
        strip_bare_json_language_prefix(raw),
        "{\"answer\":\"ok\",\"qualified\":true}"
    );
    assert_eq!(
        strip_bare_json_language_prefix("json response follows"),
        "json response follows"
    );
}

#[test]
fn observed_answer_parser_unwraps_nested_finalizer_envelope() {
    let raw = "json\n{\"answer\":\"# RustClaw\\n正文\",\"qualified\":true,\"needs_clarify\":false,\"is_meta_instruction\":false,\"publishable\":true,\"confidence\":0.85,\"reason\":\"grounded\"}";
    assert_eq!(
        extract_answer_from_finalizer_envelope_text(raw).as_deref(),
        Some("# RustClaw\n正文")
    );
}

#[test]
fn finalizer_out_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../../../prompts/schemas/finalizer_out.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("finalizer_out.schema.json must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "schema root must be object"
    );

    const STRUCT_FIELDS: &[&str] = &[
        "answer",
        "qualified",
        "needs_clarify",
        "is_meta_instruction",
        "publishable",
        "confidence",
        "reason",
    ];
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have `properties` object");
    for field in STRUCT_FIELDS {
        assert!(
            properties.contains_key(*field),
            "schema missing parser field `{}` under properties",
            field
        );
    }

    let required: std::collections::HashSet<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have `required`")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let expected_required: std::collections::HashSet<&str> = [
        "answer",
        "qualified",
        "needs_clarify",
        "is_meta_instruction",
        "publishable",
        "confidence",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        required, expected_required,
        "finalizer_out required set drifted from canonical fields"
    );

    let probes: &[(&str, &str)] = &[
        (
            "minimum",
            r#"{"answer":"ok","qualified":true,"needs_clarify":false,"is_meta_instruction":false,"publishable":true,"confidence":0.0}"#,
        ),
        (
            "boundary_high",
            r#"{"answer":"ok","qualified":true,"needs_clarify":false,"is_meta_instruction":false,"publishable":true,"confidence":1.0,"reason":"r"}"#,
        ),
        (
            "needs_clarify_with_empty_answer",
            r#"{"answer":"","qualified":false,"needs_clarify":true,"is_meta_instruction":false,"publishable":false,"confidence":0.5}"#,
        ),
    ];
    for (label, raw) in probes {
        serde_json::from_str::<ObservedAnswerFallbackOut>(raw).unwrap_or_else(|err| {
            panic!(
                "ObservedAnswerFallbackOut probe `{}` failed: {} (raw: {})",
                label, err, raw
            )
        });
    }
}
