use super::*;

#[test]
fn generic_file_delivery_wrapped_missing_find_name_supplies_checked_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generated-missing-delivery",
        "ask",
        "send definitely_missing_named_file_golden_001.txt",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.delivery_required = true;
    route.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.response_shape = crate::OutputResponseShape::FileToken;
    route.locator_kind = crate::OutputLocatorKind::Path;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "find_name",
                    "count": 0,
                    "exact": false,
                    "patterns": ["definitely_missing_named_file_golden_001.txt"],
                    "results": [],
                    "root": ""
                },
                "text": "{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"definitely_missing_named_file_golden_001.txt\"],\"results\":[],\"root\":\"\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("exists_false"));
}

#[test]
fn docker_success_exit_text_counts_as_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-docker-version", "ask", "检查 Docker 是否可用");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("exit=0\nClient: Docker Engine".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("command_output"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("exit")
            && item.get("excerpt").and_then(Value::as_str) == Some("0")
    }));
}

#[test]
fn selected_package_manager_field_uses_structured_evidence() {
    let mut journal = TaskJournal::for_task("task-package-manager", "ask", "检测包管理器");
    let mut route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: true,
        ..Default::default()
    };
    route.selection.structured_field_selector = Some("manager".to_string());
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "package_manager".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "detect",
                    "manager": "apt-get"
                },
                "text": "untrusted fallback"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("manager"));
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn selected_keys_array_counts_as_generic_selector_evidence() {
    let mut journal = TaskJournal::for_task("task-selected-keys", "ask", "config keys");
    let mut route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..Default::default()
    };
    route.selection.structured_field_selector = Some("keys".to_string());
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "structured_keys",
                "exists": true,
                "keys": ["app", "features", "paths"],
                "count": 3
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("keys"));
}

#[test]
fn command_not_found_text_counts_as_generic_command_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-command-not-found",
        "ask",
        "Check service availability",
    );
    let route = route_for_semantic(crate::OutputSemanticKind::None);
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("bash: line 1: service-cli: command not found\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn scalar_count_json_value_counts_as_count_evidence() {
    let mut journal = TaskJournal::for_task("task-scalar-count", "ask", "输出数量");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ScalarCount);
    route.response_shape = crate::OutputResponseShape::Scalar;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("3\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn log_analyze_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-log-summary", "ask", "总结日志异常");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ContentExcerptSummary);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "log_analyze".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "path": "logs/clawd.log",
                "level_counts": {"error": 1},
                "recent_notable_lines": ["ERROR sample"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn browser_web_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-web-summary", "ask", "总结网页");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Url;
    route.locator_hint = "https://example.com".to_string();
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "browser_web".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "items": [{
                    "url": "https://example.com",
                    "title": "Example Domain",
                    "content_excerpt": "Example Domain is for documentation examples."
                }],
                "summary": "Extracted 1 page(s) using browser"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("browser_web.structured_json_v1"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|evidence| evidence.get("items"))
        .and_then(Value::as_array)
        .expect("browser observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("items[0].title")
            && item.get("excerpt").and_then(Value::as_str) == Some("Example Domain")
            && item.get("redacted").is_none()
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("items[0].content_excerpt")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("documentation examples"))
            && item.get("redacted").is_none()
    }));
}

#[test]
fn web_search_extract_output_counts_as_candidates_evidence() {
    let mut journal = TaskJournal::for_task("task-web-search-summary", "ask", "总结搜索结果");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "web_search_extract".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "items": [{
                    "title": "Rust Async",
                    "url": "https://example.com",
                    "snippet": "Async Rust tutorial"
                }],
                "extract_urls": ["https://example.com"],
                "summary": "1 result"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage
        .observed_extractors
        .contains("web_search_extract.structured_json_v1"));
}

#[test]
fn web_search_extract_empty_candidates_count_as_candidates_evidence() {
    let mut journal = TaskJournal::for_task("task-web-search-empty", "ask", "总结搜索结果");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "web_search_extract".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "text": "{\"status\":\"ok\",\"items\":[],\"summary\":\"No results found\"}",
                "extra": {
                    "schema_version": 1,
                    "action": "search",
                    "status": "ok",
                    "backend": "duckduckgo_html",
                    "backend_connected": true,
                    "field_value": {
                        "status": "ok",
                        "result_count": 0,
                        "summary": "No results found"
                    },
                    "items": [],
                    "candidates": [],
                    "extract_urls": [],
                    "citations": []
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage
        .observed_extractors
        .contains("web_search_extract.structured_json_v1"));
}

#[test]
fn weather_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-weather-query", "ask", "查天气");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "weather".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "Beijing current weather: clear, 22 C.",
                "extra": {"action": "query", "mode": "current", "locale": "en-US"},
                "error_text": null
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("weather.structured_json_v1"));
}

#[test]
fn stock_output_counts_as_generic_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-market-quote", "ask", "查行情");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "stock".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "【SH600519】贵州茅台 现价 1688.00",
                "error_text": null
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("stock.structured_json_v1"));
}

#[test]
fn crypto_quote_extra_content_excerpt_counts_as_generic_evidence() {
    let mut journal = TaskJournal::for_task("task-crypto-quote", "ask", "查 BTCUSDT 价格");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "crypto".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "text": "BTCUSDT | 价格来源：- 币安(BINANCE) $69587.260000",
                "extra": {
                    "action": "quote",
                    "content_excerpt": "BTCUSDT | 价格来源：- 币安(BINANCE) $69587.260000",
                    "quote": {
                        "symbol": "BTCUSDT",
                        "price_usd": 69587.26,
                        "exchange": "binance",
                        "source": "binance_api"
                    }
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("crypto.structured_json_v1"));
}

#[test]
fn image_vision_output_counts_as_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task("task-image-understanding", "ask", "描述图片");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "image_vision".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "status": "ok",
                "text": "The image shows a Rust logo.",
                "extra": {"action": "describe"},
                "error_text": null
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_extractors
        .contains("image_vision.structured_json_v1"));
}

#[test]
fn x_preview_output_counts_as_generic_content_evidence() {
    let mut journal = TaskJournal::for_task("task-publishing-preview", "ask", "预览发布文案");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.locator_kind = crate::OutputLocatorKind::None;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "x".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("x skill dry_run=1, preview post: RustClaw release notes".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(
        coverage.observed_canonical.contains("content_excerpt"),
        "coverage: {coverage:?}"
    );
    assert!(coverage.observed_extractors.contains("x.text_legacy_v1"));
}

#[test]
fn json_observed_evidence_prioritizes_complete_candidate_names_before_entry_details() {
    let output = r#"{
        "action": "inventory_dir",
        "counts": {"dirs": 1, "files": 2, "hidden": 0, "total": 3},
        "dirs_only": false,
        "entries": [
            {"hidden": false, "kind": "dir", "modified_ts": 1, "name": "archive", "path": "docs/archive", "size_bytes": 0},
            {"hidden": false, "kind": "file", "modified_ts": 1, "name": "release_checklist.md", "path": "docs/release_checklist.md", "size_bytes": 153},
            {"hidden": false, "kind": "file", "modified_ts": 1, "name": "service_notes.md", "path": "docs/service_notes.md", "size_bytes": 272}
        ],
        "names": ["archive", "release_checklist.md", "service_notes.md"],
        "names_by_kind": {
            "dirs": ["archive"],
            "files": ["release_checklist.md", "service_notes.md"],
            "other": []
        }
    }"#;

    let observed = observed_evidence_from_output(Some(output))
        .expect("json output should produce observed evidence");
    assert_eq!(
        observed.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("names[2]")
            && item.get("excerpt").and_then(Value::as_str) == Some("service_notes.md")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("names_by_kind.files[1]")
            && item.get("excerpt").and_then(Value::as_str) == Some("service_notes.md")
    }));
}

#[test]
fn json_read_range_excerpt_preserves_provider_safe_line_evidence() {
    let output = json!({
        "action": "read_range",
        "mode": "tail",
        "path": "logs/clawd.run.log",
        "excerpt": "1695|INFO task_call: [ASK_STATE] ask_state_transition state_from=none state_to=finalizing\n1696|INFO task_call: answer_verifier_skipped_structural_satisfaction\n1697|INFO task_call: task_call_end kind=ask status=success path=normal"
    })
    .to_string();

    let observed = observed_evidence_from_output(Some(&output))
        .expect("json read_range output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");

    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("content_excerpt")
            && item.get("origin_field").and_then(Value::as_str) == Some("excerpt")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| {
                    excerpt.contains("task_call_end") && excerpt.contains("status=success")
                })
    }));
}

#[test]
fn json_wrapped_read_range_excerpt_samples_tail_line_evidence() {
    let excerpt = (1..=20)
        .map(|line| {
            if line == 16 {
                format!("{line}|2026-04-01T10:08:44Z ERROR provider timeout")
            } else {
                format!("{line}|2026-04-01T10:00:{line:02}Z INFO line {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output = json!({
        "text": json!({
            "action": "read_range",
            "excerpt": excerpt,
            "path": "scripts/nl_tests/fixtures/device_local/logs/app.log",
            "resolved_path": "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log"
        })
        .to_string(),
        "extra": {
            "action": "read_range",
            "excerpt": excerpt,
            "path": "scripts/nl_tests/fixtures/device_local/logs/app.log",
            "resolved_path": "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log"
        }
    })
    .to_string();

    let observed = observed_evidence_from_output(Some(&output))
        .expect("wrapped json read_range output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");

    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("content_excerpt")
            && item.get("origin_field").and_then(Value::as_str) == Some("extra.excerpt")
            && item.get("line_index").and_then(Value::as_u64) == Some(15)
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("ERROR provider timeout"))
    }));
}

#[test]
fn json_observed_evidence_prioritizes_health_check_process_counts() {
    let output = json!({
        "clawd_health_port_open": true,
        "clawd_log": {"exists": true, "keyword_error_count": 43, "modified_ts": 1779824680, "size_bytes": 1046356},
        "clawd_process_count": 1,
        "log_dir": "/home/guagua/rustclaw/logs",
        "system_health": {
            "arch": "x86_64",
            "cpu_count": 8,
            "disk_root_available_bytes": 17418850304u64,
            "disk_root_total_bytes": 156546629632u64,
            "hostname": "ThinkPad-X1",
            "kernel_release": "6.17.0-29-generic",
            "load_avg_15m": 1.26,
            "load_avg_1m": 0.15,
            "load_avg_5m": 0.56,
            "memory_available_bytes": 10011176960u64,
            "memory_total_bytes": 15937286144u64,
            "os_family": "linux",
            "service_manager": "systemd",
            "uptime_seconds": 485924,
            "warnings": ["disk_root_low"]
        },
        "telegramd_log": {"exists": true, "keyword_error_count": 1, "modified_ts": 1779821271, "size_bytes": 942},
        "telegramd_process_count": 0,
        "workspace_root": "/home/guagua/rustclaw"
    });
    let output = json!({ "extra": output, "text": "health_check structured output" });

    let observed = observed_evidence_from_output(Some(&output.to_string()))
        .expect("json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.telegramd_process_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("0")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.clawd_process_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("1")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.clawd_log.keyword_error_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("43")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.telegramd_log.keyword_error_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("1")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.telegramd_log.size_bytes")
            && item.get("excerpt").and_then(Value::as_str) == Some("942")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str)
            == Some("extra.system_health.disk_root_total_bytes")
            && item.get("excerpt").and_then(Value::as_str) == Some("156546629632")
    }));
    for (field, expected) in [
        ("extra.system_health.hostname", "ThinkPad-X1"),
        ("extra.system_health.kernel_release", "6.17.0-29-generic"),
        ("extra.system_health.os_family", "linux"),
        ("extra.system_health.arch", "x86_64"),
        ("extra.system_health.cpu_count", "8"),
        ("extra.system_health.service_manager", "systemd"),
        ("extra.system_health.load_avg_1m", "0.15"),
        ("extra.system_health.load_avg_5m", "0.56"),
        ("extra.system_health.load_avg_15m", "1.26"),
    ] {
        assert!(
            items.iter().any(|item| {
                item.get("field").and_then(Value::as_str) == Some(field)
                    && item.get("excerpt").and_then(Value::as_str) == Some(expected)
            }),
            "missing priority health_check field {field}"
        );
    }
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.system_health.warnings")
            && item
                .get("sample_values")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().any(|value| value == "disk_root_low"))
    }));
}

#[test]
fn text_observed_evidence_parses_status_prefixed_json_body() {
    let output = concat!(
        "status=200\n",
        "{\"ok\":true,\"data\":{\"version\":\"0.1.7\",\"worker_state\":\"running\",\"uptime_seconds\":95,\"telegramd_process_count\":0},\"error\":null}"
    );

    let observed = observed_evidence_from_output(Some(output))
        .expect("status-prefixed json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("status")
            && item.get("excerpt").and_then(Value::as_str) == Some("200")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("body.ok")
            && item.get("excerpt").and_then(Value::as_str) == Some("true")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("body.data.worker_state")
            && item.get("excerpt").and_then(Value::as_str) == Some("running")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("body.data.telegramd_process_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("0")
    }));
}

#[test]
fn embedded_http_health_body_prioritizes_optional_daemon_statuses() {
    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.8",
            "worker_state": "running",
            "uptime_seconds": 76,
            "running_length": 1,
            "queue_length": 0,
            "memory_rss_bytes": 72663040,
            "telegramd_healthy": true,
            "telegramd_process_count": 1,
            "channel_gateway_healthy": false,
            "channel_gateway_process_count": 0,
            "telegram_bot_healthy": true,
            "telegram_bot_process_count": 1,
            "whatsappd_healthy": true,
            "whatsappd_process_count": 1,
            "webd_healthy": false,
            "webd_process_count": 0,
            "wechatd_healthy": true,
            "wechatd_process_count": 1,
            "feishud_healthy": true,
            "feishud_process_count": 1,
            "larkd_healthy": true,
            "larkd_process_count": 1,
            "whatsapp_cloud_healthy": true,
            "whatsapp_cloud_process_count": 1,
            "whatsapp_web_healthy": true,
            "whatsapp_web_process_count": 1,
            "gateway_instance_statuses": [
                {"kind": "telegram", "name": "primary", "scope": "telegram:primary", "healthy": false, "status": "stale"},
                {"kind": "feishu", "name": "primary", "scope": "feishu:primary", "healthy": true, "status": "running"}
            ]
        }
    });
    let output = json!({
        "extra": {
            "action": "get",
            "url": "http://127.0.0.1:8787/v1/health",
            "status_code": 200,
            "success_status": true,
            "body_preview": body.to_string()
        },
        "text": "status=200"
    });

    let observed = observed_evidence_from_output(Some(&output.to_string()))
        .expect("json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");

    for (field, expected) in [
        ("body.data.whatsapp_cloud_healthy", "true"),
        ("body.data.whatsapp_cloud_process_count", "1"),
        ("body.data.whatsapp_web_healthy", "true"),
        ("body.data.whatsapp_web_process_count", "1"),
        ("body.data.gateway_instance_statuses[0].status", "stale"),
        (
            "body.data.gateway_instance_statuses[0].scope",
            "telegram:primary",
        ),
    ] {
        assert!(
            items.iter().any(|item| {
                item.get("field").and_then(Value::as_str) == Some(field)
                    && item.get("excerpt").and_then(Value::as_str) == Some(expected)
            }),
            "missing priority http health field {field}"
        );
    }
}

#[test]
fn text_observed_evidence_keeps_safe_file_tokens_while_redacting_secret_tokens() {
    let output = concat!(
        "The files are builtin_write_smoke.txt, full_suite_trace_note.txt, gen-1778122040.png, ",
        "and hello.sh; secrets sk-123456789012345678901234 and ",
        "rustclaw-secret://v1/12345678-1234-1234-1234-123456789abc should not be exposed."
    );

    let observed = observed_evidence_from_output(Some(output))
        .expect("text output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    let text_excerpt = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("text_excerpt"))
        .and_then(|item| item.get("excerpt"))
        .and_then(Value::as_str)
        .expect("text excerpt");

    assert!(text_excerpt.contains("full_suite_trace_note.txt"));
    assert!(text_excerpt.contains("gen-1778122040.png"));
    assert!(text_excerpt.contains("hello.sh"));
    assert!(text_excerpt.contains("[redacted]"));
    assert!(!text_excerpt.contains("sk-123456789012345678901234"));
    assert!(!text_excerpt.contains("rustclaw-secret://"));
}

#[test]
fn json_observed_evidence_array_items_include_provider_safe_sample_values() {
    let names = vec![
        "builtin_write_smoke.txt",
        "full_suite_trace_note.txt",
        "gen-1778122040.png",
        "gen-1778122536.png",
        "hello.sh",
        "hello_from_manual_test.sh",
        "hello_from_p2_smoke.sh",
        "hello_from_p2_smoke_v2.sh",
        "hello_world.sh",
        "manual_fixture_note.txt",
        "manual_meta.json",
        "manual_meta_variant.json",
        "manual_note.txt",
        "manual_note_variant.txt",
        "minimax_pwd_line.txt",
        "natural_manual_note.txt",
    ];
    let output = json!({
        "action": "inventory_dir",
        "names": names,
        "names_by_kind": {
            "files": names,
            "dirs": [],
            "other": []
        },
        "path": "document"
    });

    let observed = observed_evidence_from_output(Some(&output.to_string()))
        .expect("json output should produce observed evidence");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    let names_item = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("names"))
        .expect("names array evidence item");
    let sample_values = names_item
        .get("sample_values")
        .and_then(Value::as_array)
        .expect("names array should expose sample_values");
    assert!(sample_values
        .iter()
        .any(|item| item.as_str() == Some("manual_note_variant.txt")));
}

#[test]
fn large_inventory_dir_observed_evidence_preserves_mtime_metadata_when_truncated() {
    let entries = (0..68)
        .map(|idx| {
            json!({
                "hidden": false,
                "kind": if idx % 2 == 0 { "file" } else { "dir" },
                "modified_ts": 1780000000_u64 - idx,
                "name": format!("entry_{idx}.txt"),
                "path": format!("entry_{idx}.txt"),
                "size_bytes": 100 + idx
            })
        })
        .collect::<Vec<_>>();
    let names = (0..68)
        .map(|idx| format!("entry_{idx}.txt"))
        .collect::<Vec<_>>();
    let output = json!({
        "action": "inventory_dir",
        "counts": {"dirs": 34, "files": 34, "hidden": 0, "total": 68},
        "entries": entries,
        "names": names,
        "names_by_kind": {
            "dirs": ["entry_1.txt", "entry_3.txt", "entry_5.txt"],
            "files": ["entry_0.txt", "entry_2.txt", "entry_4.txt"],
            "other": []
        },
        "path": "/home/guagua/rustclaw",
        "sort_by": "mtime_desc"
    });
    let output_text = output.to_string();

    let observed = observed_evidence_from_output(Some(&output_text))
        .expect("json output should produce observed evidence");
    assert_eq!(
        observed.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("sort_by")
            && item.get("excerpt").and_then(Value::as_str) == Some("mtime_desc")
    }));
    let entries_item = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("entries"))
        .expect("entries array evidence item");
    let sample_keys = entries_item
        .get("sample_keys")
        .and_then(Value::as_array)
        .expect("array object sample keys");
    assert!(sample_keys
        .iter()
        .any(|item| item.as_str() == Some("modified_ts")));
    assert!(sample_keys
        .iter()
        .any(|item| item.as_str() == Some("size_bytes")));

    let mut journal = TaskJournal::for_task("task-large-mtime-dir", "ask", "list recent entries");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output_text),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("modified_ts"));
    assert!(coverage.observed_canonical.contains("sort_by"));
}

#[test]
fn health_check_fields_count_as_generic_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-health-fields", "ask", "inspect runtime health");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        locator_kind: crate::OutputLocatorKind::None,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(r#"{"clawd_health_port_open":true,"clawd_process_count":1}"#.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn wrapped_system_basic_info_counts_as_generic_field_value_evidence() {
    let mut journal = TaskJournal::for_task("task-system-info", "ask", "show runtime information");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {
                    "arch": "x86_64",
                    "current_user": "guagua",
                    "cwd": "/home/guagua/rustclaw",
                    "hostname": "ThinkPad-X1",
                    "os": "linux",
                    "pid": 2268074,
                    "process_rss_bytes": 3084288,
                    "uptime_seconds": "868570.44",
                    "workspace_root": "/home/guagua/rustclaw"
                },
                "text": "{\"arch\":\"x86_64\",\"current_user\":\"guagua\",\"cwd\":\"/home/guagua/rustclaw\",\"hostname\":\"ThinkPad-X1\",\"os\":\"linux\",\"pid\":2268074,\"process_rss_bytes\":3084288,\"uptime_seconds\":\"868570.44\",\"workspace_root\":\"/home/guagua/rustclaw\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    let trace = journal.to_trace_json();
    let extractor_ref = trace
        .pointer("/step_results/0/observed_evidence/extractor/extractor_ref")
        .and_then(Value::as_str);
    assert_eq!(extractor_ref, Some("system_basic.info.structured_json_v1"));
}

#[test]
fn doc_parse_metadata_path_counts_as_required_path_before_truncation() {
    let mut journal =
        TaskJournal::for_task("task-doc-parse-path", "ask", "读 README 并用三句话总结");
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Filename;
    route.locator_hint = "README.md".to_string();
    journal.record_output_contract(&route.clone());
    let sections = (0..32)
        .map(|idx| {
            json!({
                "id": format!("sec_{idx}"),
                "title": format!("Section {idx}"),
                "level": 2,
                "content": "long section body"
            })
        })
        .collect::<Vec<_>>();
    let output = json!({
        "text": "RustClaw is a local Rust agent runtime.",
        "sections": sections,
        "metadata": {
            "path": "/home/guagua/rustclaw/README.md",
            "type": "md"
        },
        "status": "ok"
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "doc_parse".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn run_cmd_process_output_counts_as_generic_command_evidence() {
    let mut journal =
        TaskJournal::for_task("task-process-run-cmd", "ask", "inspect the running process");
    let route = route_for_semantic(crate::OutputSemanticKind::None);
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            "154421 clawd /home/guagua/rustclaw/target/release/clawd --config /home/guagua/rustclaw/configs/config.toml\n"
                .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn http_basic_text_counts_as_generic_field_value_evidence() {
    let mut journal =
        TaskJournal::for_task("task-http-basic-fields", "ask", "检查本地 health 接口");
    let route = route_for_semantic(crate::OutputSemanticKind::None);
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("status=200\n{\"ok\":true,\"service\":\"clawd\"}\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn http_basic_json_wrapper_extracts_embedded_body_status_fields() {
    let mut journal = TaskJournal::for_task(
        "task-http-basic-json",
        "ask",
        "observe local health endpoint",
    );
    let route = route_for_semantic(crate::OutputSemanticKind::None);
    journal.record_output_contract(&route.clone());
    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.7",
            "worker_state": "running",
            "uptime_seconds": 53,
            "queue_length": 0,
            "memory_rss_bytes": 161181696,
            "user_count": 2,
            "bound_channel_count": 3,
            "channel_gateway_healthy": false,
            "telegram_bot_statuses": [
                {
                    "name": "primary",
                    "healthy": false,
                    "status": "stale"
                }
            ]
        },
        "error": null
    })
    .to_string();
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "get",
                    "url": "http://127.0.0.1:8787/v1/health",
                    "status_code": 200,
                    "success_status": true,
                    "body_preview": body.clone(),
                },
                "text": format!("status=200\n{body}")
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage
        .observed_extractors
        .contains("http_basic.structured_json_v1"));
    assert!(coverage
        .observed_fields
        .contains("body.data.channel_gateway_healthy"));
    assert!(coverage.observed_fields.contains("body.data.version"));
    assert!(coverage
        .observed_fields
        .contains("body.data.uptime_seconds"));
    assert!(coverage.observed_fields.contains("body.data.queue_length"));
    assert!(coverage
        .observed_fields
        .contains("body.data.memory_rss_bytes"));
    assert!(coverage.observed_fields.contains("body.data.user_count"));
    assert!(coverage
        .observed_fields
        .contains("body.data.bound_channel_count"));
    assert!(coverage
        .observed_fields
        .contains("body.data.telegram_bot_statuses[0].name"));
    assert!(coverage
        .observed_fields
        .contains("body.data.telegram_bot_statuses[0].status"));
}

#[test]
fn http_basic_json_wrapper_body_counts_as_generic_content_excerpt_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-web-summary-http-basic-json",
        "ask",
        "summarize local health endpoint",
    );
    let route = crate::IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Url,
        locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
        ..crate::IntentOutputContract::default()
    };
    journal.record_output_contract(&route.clone());
    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.7",
            "worker_state": "running",
            "uptime_seconds": 53,
            "channel_gateway_healthy": false,
            "telegram_bot_statuses": [
                {
                    "name": "primary",
                    "healthy": false,
                    "status": "stale"
                }
            ]
        }
    })
    .to_string();
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "get",
                    "url": "http://127.0.0.1:8787/v1/health",
                    "status_code": 200,
                    "success_status": true,
                    "body_preview": body.clone(),
                },
                "text": format!("status=200\n{body}")
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage
        .observed_fields
        .contains("body.data.channel_gateway_healthy"));
    assert!(coverage.observed_fields.contains("body.data.version"));
    assert!(coverage
        .observed_fields
        .contains("body.data.uptime_seconds"));
}

#[test]
fn raw_command_output_http_basic_text_counts_as_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-raw-command-http-basic",
        "ask",
        "请求 http://127.0.0.1:8787/v1/health",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::RawCommandOutput);
    route.requires_content_evidence = true;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("status=200\n{\"ok\":true,\"service\":\"clawd\"}\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn raw_command_output_file_read_excerpt_counts_as_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-raw-command-file-read",
        "ask",
        "读取 README.md 前 4 行",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::RawCommandOutput);
    route.requires_content_evidence = true;
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            r#"{"action":"read_range","path":"/tmp/README.md","excerpt":"1|# Demo\n2|body"}"#
                .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("content_excerpt"));
    assert!(coverage.observed_canonical.contains("command_output"));
}
