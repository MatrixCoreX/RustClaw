use super::*;
use serde_json::json;

#[test]
fn target_missing_returns_structured_error() {
    let args = json!({"action": "start"});
    let out =
        execute("req-1".to_string(), args, None).expect("execute must return Ok(OutputContract)");
    assert_eq!(out.status, "error");
    assert_eq!(out.error_kind, "missing_input");
    assert!(!out.failure_reason.is_empty(), "failure_reason must be set");
    assert!(!out.next_step.is_empty());
}

#[test]
fn ambiguous_target_blocks_high_risk_action() {
    let args = json!({"action": "restart", "target": "\u{540E}\u{7AEF}"});
    let out =
        execute("req-2".to_string(), args, None).expect("execute must return Ok(OutputContract)");
    assert_eq!(out.status, "error");
    assert!(
        out.failure_reason.contains("ambiguous") || out.failure_reason.contains("high-risk"),
        "expected ambiguous/high-risk refusal: {}",
        out.failure_reason
    );
}

#[test]
fn business_failure_produces_runner_error() {
    let args = json!({"action": "start"});
    let out = execute("req-bf".to_string(), args, None).unwrap();
    assert_eq!(out.status, "error");
    let resp = build_runner_response("req-bf".to_string(), Ok(out));
    assert_eq!(resp.status, "error");
    assert_eq!(resp.error_kind.as_deref(), Some("missing_input"));
    assert_eq!(resp.platform.as_deref(), Some(std::env::consts::OS));
    assert!(resp.error_text.is_some());
}

#[test]
fn status_failure_not_overwritten_by_ok_summary() {
    let args = json!({"action": "status", "target": "nonexistent_xyz_123"});
    let out = execute("req-status".to_string(), args, None).unwrap();
    assert_eq!(
        out.status, "error",
        "unknown manager or status failure must set status=error"
    );
    assert!(!out.failure_reason.is_empty());
}

#[test]
fn verify_failure_not_overwritten_by_ok_summary() {
    let args = json!({"action": "verify", "target": "nonexistent_xyz_456"});
    let out = execute("req-verify".to_string(), args, None).unwrap();
    assert_eq!(
        out.status, "error",
        "unknown manager for verify must set status=error"
    );
    assert!(!out.failure_reason.is_empty());
}

#[test]
fn manager_rustclaw_whitelist() {
    let args = json!({"action": "status", "target": "clawd"});
    let out = execute("req-m1".to_string(), args, None).unwrap();
    assert_eq!(out.manager_type, "rustclaw");
}

#[test]
fn rustclaw_service_target_overrides_incompatible_manager_hint() {
    let args = json!({"action": "status", "target": "clawd", "manager_type": "systemd"});
    let out = execute("req-rustclaw-manager-hint".to_string(), args, None).unwrap();
    assert_eq!(out.manager_type, "rustclaw");
    assert_eq!(out.status, "ok");
}

#[test]
fn rustclaw_status_without_user_key_falls_back_to_process_scan() {
    let args = json!({"action": "status", "target": "clawd"});
    let out = execute("req-rustclaw-fallback".to_string(), args, None).unwrap();
    assert_eq!(out.manager_type, "rustclaw");
    assert_eq!(out.status, "ok");
    assert!(out.failure_reason.is_empty());
    assert!(
        out.pre_state.contains("clawd="),
        "pre_state: {}",
        out.pre_state
    );
}

#[test]
fn runner_status_response_serializes_target_alias() {
    let args = json!({"action": "status", "target": "clawd"});
    let out = execute("req-target-alias".to_string(), args, None).unwrap();
    let resp = build_runner_response("req-target-alias".to_string(), Ok(out));
    assert_eq!(resp.status, "ok");
    let parsed: Value = serde_json::from_str(&resp.text).expect("structured service status");
    assert_eq!(resp.extra.as_ref(), Some(&parsed));
    assert_eq!(parsed.get("target").and_then(Value::as_str), Some("clawd"));
    assert_eq!(
        parsed.get("service_name").and_then(Value::as_str),
        Some("clawd")
    );
}

#[test]
fn status_without_target_defaults_to_rustclaw_manager() {
    let input = parse_input(&json!({"action": "status"})).unwrap();
    assert_eq!(resolve_manager(&input, None), "rustclaw");
}

#[test]
fn manager_explicit_type() {
    let args = json!({"action": "status", "target": "nginx", "manager_type": "systemd"});
    let out = execute("req-m2".to_string(), args, None).unwrap();
    assert_eq!(out.manager_type, "systemd");
}

#[test]
fn manager_unknown_or_detected() {
    let args = json!({"action": "status", "target": "nonexistent_svc_xyz_789"});
    let out = execute("req-m3".to_string(), args, None).unwrap();
    assert!(
        out.manager_type == "unknown"
            || out.manager_type == "brew_services"
            || out.manager_type == "launchd"
            || out.manager_type == "systemd"
            || out.manager_type == "service"
            || out.manager_type == "process_only",
        "fallback or detected: {}",
        out.manager_type
    );
}

#[test]
fn output_contract_has_required_keys() {
    let args = json!({"action": "start"});
    let out = execute("req-3".to_string(), args, None).unwrap();
    let text = serde_json::to_string(&out).unwrap();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let required = [
        "status",
        "service_name",
        "manager_type",
        "requested_action",
        "executed_actions",
        "key_evidence",
        "failure_reason",
        "error_kind",
    ];
    for key in required {
        assert!(
            parsed.get(key).is_some(),
            "output must contain key: {}",
            key
        );
    }
}

#[test]
fn safe_target_accepts_alphanumeric_and_dots() {
    assert!(is_safe_target("nginx"));
    assert!(is_safe_target("redis-server"));
    assert!(is_safe_target("unit@.service"));
    assert!(!is_safe_target(""));
    assert!(!is_safe_target("a;b"));
    assert!(!is_safe_target("/etc/passwd"));
}

#[test]
fn ambiguous_target_detection() {
    assert!(is_ambiguous_target("\u{540E}\u{7AEF}"));
    assert!(is_ambiguous_target("\u{670D}\u{52A1}\u{4EEC}"));
    assert!(is_ambiguous_target("all"));
    assert!(is_ambiguous_target("  ALL  "));
    assert!(!is_ambiguous_target("nginx"));
    assert!(!is_ambiguous_target("clawd"));
}

#[test]
fn high_risk_actions() {
    assert!(is_high_risk_action("stop"));
    assert!(is_high_risk_action("restart"));
    assert!(!is_high_risk_action("start"));
    assert!(!is_high_risk_action("status"));
}

#[test]
fn read_only_action_classification() {
    // Read-only: the multi-match auto-pick path applies to these.
    assert!(is_read_only_action("status"));
    assert!(is_read_only_action("logs"));
    assert!(is_read_only_action("verify"));
    assert!(is_read_only_action("diagnose_start_failure"));
    assert!(is_read_only_action("diagnose_unhealthy_state"));
    // Mutating: multi-match must still hard-fail.
    assert!(!is_read_only_action("start"));
    assert!(!is_read_only_action("stop"));
    assert!(!is_read_only_action("restart"));
    assert!(!is_read_only_action("reload"));
}

#[test]
fn normalize_target_alias_maps_common_aliases() {
    assert_eq!(normalize_target_alias("nginx"), "nginx");
    assert_eq!(normalize_target_alias("mysql"), "mysql");
    assert_eq!(normalize_target_alias("mysqld"), "mysql");
    assert_eq!(normalize_target_alias("redis"), "redis");
    assert_eq!(normalize_target_alias("redis-server"), "redis");
    assert_eq!(normalize_target_alias("postgres"), "postgresql");
    assert_eq!(normalize_target_alias("postgresql"), "postgresql");
    assert_eq!(normalize_target_alias("docker"), "docker");
    assert_eq!(normalize_target_alias("dockerd"), "docker");
    assert_eq!(normalize_target_alias("sshd"), "sshd");
    assert_eq!(normalize_target_alias("ssh"), "sshd");
    assert_eq!(normalize_target_alias("cron"), "cron");
    assert_eq!(normalize_target_alias("crond"), "cron");
}

#[test]
fn normalize_target_alias_strips_service_suffix() {
    assert_eq!(normalize_target_alias("redis service"), "redis");
}

#[test]
fn normalize_target_alias_does_not_parse_natural_language_suffixes() {
    assert_eq!(
        normalize_target_alias("nginx\u{670D}\u{52A1}"),
        "nginx\u{670D}\u{52A1}"
    );
    assert_eq!(
        normalize_target_alias("mysql \u{670D}\u{52A1}"),
        "mysql \u{670D}\u{52A1}"
    );
}
