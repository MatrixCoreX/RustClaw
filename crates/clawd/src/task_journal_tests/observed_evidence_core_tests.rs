use super::*;

#[test]
fn trace_json_includes_redacted_observed_evidence_for_json_output() {
    let mut journal = TaskJournal::for_task("task-observed-evidence", "ask", "读取配置");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_fields",
                "count": 2,
                "extra": {
                    "field_value": "enabled",
                    "api_key": "sk-test-super-secret-token-value-1234567890"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(observed.get("format").and_then(Value::as_str), Some("json"));
    assert_eq!(
        observed.pointer("/extractor/kind").and_then(Value::as_str),
        Some("structured_json")
    );
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("config_basic.read_fields.structured_json_v1")
    );
    assert_eq!(
        observed
            .pointer("/extractor/source_action_ref")
            .and_then(Value::as_str),
        Some("config_basic.read_fields")
    );
    assert_eq!(
        observed
            .pointer("/extractor/strict_shape_eligible")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        observed
            .pointer("/extractor/fallback")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        observed
            .pointer("/extractor/provider_safety/provider_evidence_view")
            .and_then(Value::as_str),
        Some("provider_safe_redacted")
    );
    assert_eq!(
        observed
            .pointer("/extractor/provider_safety/raw_excerpt_policy")
            .and_then(Value::as_str),
        Some("no_full_raw_excerpt")
    );
    assert_eq!(
        observed
            .pointer("/extractor/provider_safety/sensitive_field_policy")
            .and_then(Value::as_str),
        Some("redact_sensitive_keys_and_secret_like_values")
    );
    assert_eq!(
        observed
            .pointer("/extractor/observation_source")
            .and_then(Value::as_str),
        Some("step_output")
    );
    assert_eq!(
        observed.get("storage").and_then(Value::as_str),
        Some("redacted_excerpt_hash")
    );

    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.field_value")
            && item.get("excerpt").and_then(Value::as_str) == Some("enabled")
            && item.get("hash").and_then(Value::as_str).is_some()
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.api_key")
            && item.get("redacted").and_then(Value::as_bool) == Some(true)
            && item.get("excerpt").is_none()
    }));
    assert!(!serde_json::to_string(observed)
        .expect("serialize observed evidence")
        .contains("sk-test-super-secret-token-value"));
}

#[test]
fn multiline_excerpt_sampling_keeps_diagnostic_severity_lines() {
    let mut journal = TaskJournal::for_task("task-log-evidence", "ask", "分析日志");
    let log_excerpt = [
        "1|2026-04-01 10:00:01 INFO service boot completed",
        "2|2026-04-01 10:00:04 INFO config loaded",
        "3|2026-04-01 10:00:09 INFO sqlite connection opened",
        "4|2026-04-01 10:01:11 INFO request queued",
        "5|2026-04-01 10:01:12 INFO worker claimed task",
        "6|2026-04-01 10:01:13 INFO read_file success",
        "7|2026-04-01 10:02:20 WARN upstream latency increased",
        "8|2026-04-01 10:02:22 INFO retry policy kept request safe",
        "9|2026-04-01 10:03:40 INFO request queued",
        "10|2026-04-01 10:03:41 INFO summary generated",
        "11|2026-04-01 10:05:01 INFO health check ok",
        "12|2026-04-01 10:05:33 WARN cache miss ratio above baseline",
        "13|2026-04-01 10:06:02 INFO fallback path used",
        "14|2026-04-01 10:06:08 INFO delivery token emitted",
        "15|2026-04-01 10:07:15 INFO db query finished",
        "16|2026-04-01 10:08:44 ERROR provider timeout",
        "17|2026-04-01 10:08:46 INFO provider retry succeeded",
        "18|2026-04-01 10:09:20 INFO answer published",
        "19|2026-04-01 10:10:01 INFO maintenance sweep finished",
        "20|2026-04-01 10:10:30 INFO service idle",
    ]
    .join("\n");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "excerpt": log_excerpt,
                    "path": "logs/app.log"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let items = trace
        .pointer("/step_results/0/observed_evidence/items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    let content_excerpt = items
        .iter()
        .filter(|item| item.get("field").and_then(Value::as_str) == Some("content_excerpt"))
        .filter_map(|item| item.get("excerpt").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(content_excerpt.contains("7|2026-04-01 10:02:20 WARN"));
    assert!(content_excerpt.contains("12|2026-04-01 10:05:33 WARN"));
    assert!(content_excerpt.contains("16|2026-04-01 10:08:44 ERROR"));
}

#[test]
fn http_health_body_prioritizes_status_scalars_for_answer_verifier() {
    let mut journal = TaskJournal::for_task("task-http-health-evidence", "ask", "health");
    let body = json!({
        "ok": true,
        "data": {
            "version": "0.1.8",
            "queue_length": 0,
            "worker_state": "running",
            "uptime_seconds": 703,
            "memory_rss_bytes": 101871616,
            "running_length": 1,
            "telegramd_healthy": false,
            "telegramd_process_count": 0,
            "channel_gateway_healthy": false,
            "channel_gateway_process_count": 0,
            "whatsappd_healthy": false,
            "whatsappd_process_count": 0,
            "webd_healthy": false,
            "webd_process_count": 0,
            "wechatd_healthy": false,
            "wechatd_process_count": 0,
            "feishud_healthy": false,
            "feishud_process_count": 0,
            "larkd_healthy": false,
            "larkd_process_count": 0,
            "user_count": 2,
            "bound_channel_count": 3,
            "gateway_instance_statuses": [
                {"kind": "telegram", "name": "primary", "healthy": false, "status": "stale"},
                {"kind": "feishu", "name": "primary", "healthy": false, "status": "stopped"}
            ]
        },
        "error": null
    });
    let body_text = body.to_string();
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "http_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "get",
                    "body_preview": body_text,
                    "status_code": 200,
                    "success_status": true,
                    "url": "http://127.0.0.1:8787/v1/health"
                },
                "text": format!("status=200\n{}", body)
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let items = trace
        .pointer("/step_results/0/observed_evidence/items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    let fields = items
        .iter()
        .filter_map(|item| item.get("field").and_then(Value::as_str))
        .collect::<Vec<_>>();

    for expected in [
        "body.ok",
        "body.data.version",
        "body.data.worker_state",
        "body.data.uptime_seconds",
        "body.data.queue_length",
        "body.data.telegramd_healthy",
        "body.data.telegramd_process_count",
        "body.data.whatsappd_healthy",
        "body.data.webd_healthy",
        "body.data.wechatd_healthy",
        "body.data.feishud_healthy",
        "body.data.larkd_healthy",
    ] {
        assert!(
            fields.contains(&expected),
            "expected {expected} in evidence fields: {fields:?}"
        );
    }
}

#[test]
fn image_generate_extra_outputs_path_counts_as_structured_path_evidence() {
    let mut journal = TaskJournal::for_task("task-image-extra-evidence", "ask", "生成图片");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "image_generate".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "text": "FILE:/tmp/rustclaw-image.png",
                "extra": {
                    "provider": "local_fallback",
                    "model": "local-placeholder",
                    "model_kind": "local_fallback",
                    "outputs": [{
                        "type": "image_file",
                        "path": "/tmp/rustclaw-image.png"
                    }]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("image_generate.structured_json_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.outputs[0].path")
            && item.get("excerpt").and_then(Value::as_str) == Some("/tmp/rustclaw-image.png")
    }));

    let route = route_for_semantic(crate::OutputSemanticKind::GeneratedFileDelivery);
    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.observed_canonical.contains("path"));
    assert!(
        coverage.missing_evidence.is_empty(),
        "{:?}",
        coverage.missing_evidence
    );
}

#[test]
fn rss_fetch_extra_field_value_counts_as_structured_rss_evidence() {
    let mut journal = TaskJournal::for_task("task-rss-extra-evidence", "ask", "抓取 RSS 新闻");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "rss_fetch".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "text": "sources_ok=2 sources_failed=0 items=3\n1. Example item",
                "extra": {
                    "schema_version": 1,
                    "action": "latest",
                    "category": "general",
                    "source_count": 2,
                    "sources_ok": 2,
                    "sources_failed": 0,
                    "item_count": 3,
                    "field_value": {
                        "sources_ok": 2,
                        "sources_failed": 0,
                        "items": 3,
                        "titles": [
                            "Example item",
                            "Louisiana ICE Facility Mistreated Immigrants, Federal Investigators Say",
                            "Third item"
                        ]
                    },
                    "items": [{
                        "title": "Example item",
                        "link": "https://example.com/news/1",
                        "source_host": "example.com"
                    }]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("rss_fetch.structured_json_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.field_value")
            && item
                .get("keys")
                .and_then(Value::as_array)
                .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some("titles")))
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.field_value.titles")
            && item
                .get("sample_values")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().any(|value| {
                    value.as_str()
                        == Some(
                            "Louisiana ICE Facility Mistreated Immigrants, Federal Investigators Say",
                        )
                }))
            && item.get("redacted_sample_values").is_none()
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.field_value.titles[1]")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("Louisiana ICE Facility Mistreated Immigrants, Federal Investigators Say")
            && item.get("redacted").is_none()
    }));

    let route = route_for_semantic(crate::OutputSemanticKind::RssNewsFetch);
    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(
        coverage.missing_evidence.is_empty(),
        "{:?}",
        coverage.missing_evidence
    );
}

#[test]
fn trace_json_includes_observed_evidence_for_text_output() {
    let mut journal = TaskJournal::for_task("task-observed-text", "ask", "运行命令");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("first line\nsecond line".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .expect("observed evidence should be present");
    assert_eq!(observed.get("format").and_then(Value::as_str), Some("text"));
    assert_eq!(
        observed.pointer("/extractor/kind").and_then(Value::as_str),
        Some("text_legacy")
    );
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("run_cmd.text_legacy_v1")
    );
    assert_eq!(
        observed
            .pointer("/extractor/source_action_ref")
            .and_then(Value::as_str),
        Some("run_cmd")
    );
    assert_eq!(
        observed
            .pointer("/extractor/fallback")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert!(observed
        .get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("field").and_then(Value::as_str) == Some("text_excerpt")
                    && item.get("excerpt").and_then(Value::as_str) == Some("first line second line")
                    && item.get("hash").and_then(Value::as_str).is_some()
            })
        }));
}

#[test]
fn explicit_extractor_registry_canonicalizes_virtual_tool_outputs() {
    let mut journal = TaskJournal::for_task("task-explicit-extractor", "ask", "列出文件");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["Cargo.toml"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let extractor = trace
        .pointer("/step_results/0/observed_evidence/extractor")
        .expect("observed evidence extractor");
    assert_eq!(
        extractor.get("extractor_ref").and_then(Value::as_str),
        Some("fs_basic.list_dir.structured_json_v1")
    );
    assert_eq!(
        extractor.get("source_action_ref").and_then(Value::as_str),
        Some("fs_basic.list_dir")
    );
    assert!(extractor
        .get("provided_evidence")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("candidates"))));
    assert!(extractor
        .get("provided_evidence")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("modified_ts"))));
    assert!(extractor
        .get("provided_evidence")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("sort_by"))));
}

#[test]
fn process_basic_port_list_evidence_keeps_public_port_samples() {
    let mut listeners = (0..30)
        .map(|idx| {
            let port = 40_000 + idx;
            json!({
                "local_endpoint": format!("127.0.0.1:{port}"),
                "local_address": "127.0.0.1",
                "port": port.to_string(),
                "bind_scope": "localhost",
                "is_wildcard": false,
                "is_loopback": true,
                "process_name": "local-only",
                "pid": 1000 + idx,
            })
        })
        .collect::<Vec<_>>();
    listeners.push(json!({
        "local_endpoint": "0.0.0.0:8787",
        "local_address": "0.0.0.0",
        "port": "8787",
        "bind_scope": "all_interfaces",
        "is_wildcard": true,
        "is_loopback": false,
        "process_name": "clawd",
        "pid": 878474,
    }));
    let public_listeners = vec![listeners.last().cloned().expect("public listener")];
    let mut journal = TaskJournal::for_task("task-port-list", "ask", "ports");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "process_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "text": "exit=0\nLISTEN ...",
                "extra": {
                    "action": "port_list",
                    "command_tool": "ss",
                    "exit_code": 0,
                    "filter": null,
                    "listener_count": listeners.len(),
                    "listeners": listeners,
                    "listeners_truncated": false,
                    "localhost_listener_count": 30,
                    "ports": ["40000", "8787"],
                    "public_listener_count": 1,
                    "public_listeners": public_listeners,
                    "public_listeners_truncated": false,
                    "public_ports": ["8787"]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .pointer("/step_results/0/observed_evidence")
        .expect("observed evidence");
    let provided = observed
        .pointer("/extractor/provided_evidence")
        .and_then(Value::as_array)
        .expect("provided evidence");
    assert!(provided
        .iter()
        .any(|item| item.as_str() == Some("public_ports")));
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("evidence items");
    let public_ports = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("extra.public_ports"))
        .expect("public ports evidence item");
    assert!(public_ports
        .get("sample_values")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == Some("8787"))));
    let public_listener = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("extra.public_listeners"))
        .expect("public listener evidence item");
    assert!(public_listener
        .get("sample_values")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| {
            value.get("local_endpoint").and_then(Value::as_str) == Some("0.0.0.0:8787")
                && value.get("port").and_then(Value::as_str) == Some("8787")
                && value.get("process_name").and_then(Value::as_str) == Some("clawd")
                && value.get("pid").and_then(Value::as_i64) == Some(878474)
        })));
}

#[test]
fn matrix_admitted_external_marker_enables_strict_structured_evidence() {
    let mut journal =
        TaskJournal::for_task("task-external-admission-evidence", "ask", "external count");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "external_counter".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count",
                "text": "3",
                "extra": {
                    "action": "count",
                    "count": 3,
                    "results": ["a", "b", "c"]
                },
                "_matrix_admission": {
                    "schema_version": 1,
                    "source": "skills_registry",
                    "skill": "external_counter",
                    "eligible": true,
                    "extractor_kind": "structured_json",
                    "required_extra_fields": ["extra.count"]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let observed = trace
        .pointer("/step_results/0/observed_evidence")
        .expect("observed evidence");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("matrix_admitted_external.structured_json_v1")
    );
    assert_eq!(
        observed
            .pointer("/extractor/strict_shape_eligible")
            .and_then(Value::as_bool),
        Some(true)
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("extra.count")
            && item.get("excerpt").and_then(Value::as_str) == Some("3")
    }));
    assert!(!items.iter().any(|item| {
        item.get("field")
            .and_then(Value::as_str)
            .is_some_and(|field| field.starts_with("_matrix_admission"))
    }));
}

#[test]
fn text_observed_evidence_extracts_count_path_and_candidates() {
    let archive_listing = "exit=0\nArchive: /tmp/test.zip\n  Length Name\n  22 notes.txt\n  20 nested/config.ini\n  42 2 files";
    let observed = observed_evidence_from_output(Some(archive_listing))
        .expect("text evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("count")
            && item.get("excerpt").and_then(Value::as_str) == Some("2")
    }));

    let observed = observed_evidence_from_output(Some("/home/guagua/rustclaw/Cargo.toml"))
        .expect("path evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("path evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("path")
            && item.get("source").and_then(Value::as_str) == Some("text_output.extractor")
    }));
    let observed = observed_evidence_from_output(Some(
        "written 40 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
    ))
    .expect("path token evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("path token evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("path")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("/home/guagua/rustclaw/document/pwd_line.txt")
    }));
    let observed = observed_evidence_from_output(Some(
        "archive_path=/home/guagua/rustclaw/tmp/bundle.zip\nexit=0\n  adding: /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md (deflated 32%)",
    ))
    .expect("labeled archive path evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("labeled path evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("path")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("/home/guagua/rustclaw/tmp/bundle.zip")
    }));

    let mut git_journal =
        TaskJournal::for_task("task-text-git-subject", "ask", "latest git subject");
    git_journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("exit=0\n09342a6a fix: expose nl execution and locator flows\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let observed = git_journal
        .to_trace_json()
        .pointer("/step_results/0/observed_evidence")
        .cloned()
        .expect("git subject evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("git_basic.text_legacy_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("git subject evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("subject")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("fix: expose nl execution and locator flows")
    }));

    let mut git_json_journal =
        TaskJournal::for_task("task-json-git-subjects", "ask", "write a release note");
    git_json_journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "git_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "log",
                    "exit_code": 0,
                    "output": "exit=0\nf77577da Tighten NL verifier recovery\na30c49fb Tighten grounded channel setup rewrites\n",
                    "raw_action": "log",
                    "subcommand": "log"
                },
                "text": "exit=0\nf77577da Tighten NL verifier recovery\na30c49fb Tighten grounded channel setup rewrites\n"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let observed = git_json_journal
        .to_trace_json()
        .pointer("/step_results/0/observed_evidence")
        .cloned()
        .expect("structured git log evidence should be present");
    assert_eq!(
        observed
            .pointer("/extractor/extractor_ref")
            .and_then(Value::as_str),
        Some("git_basic.structured_json_v1")
    );
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("structured git log evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("content_excerpt")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("Tighten NL verifier recovery"))
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("subject")
            && item.get("excerpt").and_then(Value::as_str) == Some("Tighten NL verifier recovery")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("git_subjects")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|excerpt| excerpt.contains("Tighten grounded channel setup rewrites"))
    }));

    let mut journal = TaskJournal::for_task("task-text-candidates", "ask", "列出文件名");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("Cargo.toml\nREADME.md".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete());
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("count"));

    let observed = observed_evidence_from_output(Some(".git\nREADME.md\n.env\nsrc\n"))
        .expect("hidden list evidence should be present");
    let items = observed
        .get("items")
        .and_then(Value::as_array)
        .expect("hidden list evidence items");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("hidden_count")
            && item.get("excerpt").and_then(Value::as_str) == Some("2")
    }));
}

#[test]
fn generic_path_content_list_dir_candidates_satisfy_directory_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-directory",
        "ask",
        "summarize selected directory entries",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "list_dir",
                "path": "prompts/schemas",
                "count": 1,
                "entries": [
                    {
                        "name": "intent_normalizer.schema.json",
                        "kind": "file",
                        "size_bytes": 13124
                    }
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("candidates"));
}

#[test]
fn generic_path_content_find_entries_result_path_satisfies_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-find-entry",
        "ask",
        "return the matching path",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_name",
                "root": "scripts/nl_tests/fixtures/locator_smart/stem_unique",
                "patterns": ["abcd"],
                "count": 1,
                "results": ["scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("candidates"));
}

#[test]
fn generic_path_content_wrapped_find_name_result_path_satisfies_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-wrapped-find-name",
        "ask",
        "return the matching path",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "find_name",
                    "root": "scripts/nl_tests/fixtures/locator_smart/stem_unique",
                    "patterns": ["abcd"],
                    "count": 1,
                    "results": ["scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt"]
                },
                "text": "{\"action\":\"find_name\",\"count\":1,\"exact\":false,\"patterns\":[\"abcd\"],\"results\":[\"scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt\"],\"root\":\"scripts/nl_tests/fixtures/locator_smart/stem_unique\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("candidates"));
}

#[test]
fn generic_path_content_name_results_paths_satisfy_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-name-results",
        "ask",
        "return paths matched by structured name search",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "grep_text",
                    "root": "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3",
                    "query": "abcd",
                    "match_count": 0,
                    "matches": [],
                    "name_count": 4,
                    "name_patterns": ["abcd"],
                    "name_results": [
                        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md",
                        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt",
                        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt",
                        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
                    ]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(!coverage
        .missing_evidence
        .iter()
        .any(|field| field == "path"));
}

#[test]
fn file_names_content_search_paths_satisfy_candidate_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-file-names-grep-candidates",
        "ask",
        "search workspace content and list matching files",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::FileNames);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "grep_text",
            "query": "FirstLayerDecision",
            "count": 2,
            "match_count": 3,
            "matches": [
                {"path": "README.md", "line": 54, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/intent_router.rs", "line": 14, "text": "FirstLayerDecision"}
            ]
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("content_match"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage
        .observed_extractors
        .contains("fs_basic.grep_text.structured_json_v1"));
}

#[test]
fn file_paths_content_search_paths_satisfy_candidate_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-file-paths-grep-candidates",
        "ask",
        "search workspace content and list matching paths",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::FilePaths);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "grep_text",
            "query": "FirstLayerDecision",
            "count": 2,
            "match_count": 3,
            "matches": [
                {"path": "README.md", "line": 54, "text": "FirstLayerDecision"},
                {"path": "crates/clawd/src/intent_router.rs", "line": 14, "text": "FirstLayerDecision"}
            ]
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("content_match"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage
        .observed_extractors
        .contains("fs_basic.grep_text.structured_json_v1"));
}

#[test]
fn raw_command_output_grep_text_satisfies_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-raw-grep-command-output",
        "ask",
        "search a bound file and return matching lines",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::RawCommandOutput);
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/logs/app.log".to_string();
    journal.record_route_result(&route);
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        json!({
            "action": "grep_text",
            "query": "ERROR",
            "count": 1,
            "match_count": 1,
            "matches": [
                {
                    "path": "scripts/nl_tests/fixtures/device_local/logs/app.log",
                    "line": 16,
                    "text": "2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata"
                }
            ]
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_route(&route, &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("command_output"));
    assert!(coverage.observed_canonical.contains("content_match"));
    assert!(coverage
        .observed_extractors
        .contains("fs_basic.grep_text.structured_json_v1"));
}

#[test]
fn content_excerpt_summary_directory_inventory_can_complete_from_listing_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-content-summary-listing",
        "ask",
        "summarize repository layout from directory counts",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ContentExcerptSummary);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "list_dir",
                "path": "crates",
                "counts": {"total": 3, "files": 0, "dirs": 3},
                "entries": [
                    {"name": "clawd", "kind": "dir"},
                    {"name": "skills", "kind": "dir"},
                    {"name": "skill-runner", "kind": "dir"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn excerpt_kind_judgment_directory_counts_can_complete_from_count_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-excerpt-kind-counts",
        "ask",
        "judge repository layout from directory counts",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ExcerptKindJudgment);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_entries",
                "path": "crates",
                "count": 3
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_entries",
                "path": "crates/skills",
                "count": 8
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
}

#[test]
fn generic_path_content_directory_counts_can_complete_from_count_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-generic-path-counts",
        "ask",
        "compare direct directory entry counts",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_entries",
                "path": "crates",
                "count": 3
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_entries",
                "path": "crates/skills",
                "count": 8
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
    assert!(coverage.observed_canonical.contains("path"));
}

#[test]
fn directory_purpose_tree_summary_children_satisfy_candidates_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-directory-purpose-tree-summary",
        "ask",
        "summarize relevant documentation entries",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::DirectoryPurposeSummary);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "tree_summary",
                "path": "document",
                "tree": {
                    "children": [
                        {
                            "kind": "file",
                            "path": "document/README.md",
                            "size_bytes": 128
                        }
                    ]
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage
        .observed_extractors
        .contains("system_basic.tree_summary.structured_json_v1"));
}

#[test]
fn system_basic_info_without_action_uses_info_extractor() {
    let mut journal =
        TaskJournal::for_task("task-system-info", "ask", "return current workspace path");
    let route = route_for_semantic(crate::OutputSemanticKind::ScalarPathOnly);
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "hostname": "devbox",
                "os": "linux",
                "arch": "x86_64",
                "cwd": "/home/guagua/rustclaw",
                "workspace_root": "/home/guagua/rustclaw"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage
        .observed_extractors
        .contains("system_basic.info.structured_json_v1"));
}

#[test]
fn docker_unavailable_text_counts_as_field_value_evidence() {
    let mut journal =
        TaskJournal::for_task("task-docker-unavailable", "ask", "检查 Docker 是否可用");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    route.resolved_intent = "capability_ref=docker.version".to_string();
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "docker_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("docker unavailable: No such file or directory (os error 2)".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    assert!(coverage.observed_canonical.contains("command_output"));
}

#[test]
fn generic_delivery_missing_find_count_satisfies_negative_delivery_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-missing-delivery",
        "ask",
        "send definitely_missing_named_file_golden_001.txt",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::None);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "find_name",
                "count": 0,
                "patterns": ["definitely_missing_named_file_golden_001.txt"],
                "results": [],
                "root": ""
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_route(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("count"));
}
