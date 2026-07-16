use serde::Deserialize;
use serde_json::Value;

use super::*;

#[test]
fn summary_reports_ordered_pre_planner_llm_metrics_without_prompt_text() {
    let mut journal = TaskJournal::for_task("task-frontdoor", "ask", "inspect workspace");
    journal.record_llm_call_sequence(vec![
        crate::LlmCallSequenceEntry {
            call_index: 1,
            prompt_label: "normalizer".to_string(),
            prompt_bytes: 1_200,
        },
        crate::LlmCallSequenceEntry {
            call_index: 2,
            prompt_label: "contract_repair".to_string(),
            prompt_bytes: 800,
        },
        crate::LlmCallSequenceEntry {
            call_index: 3,
            prompt_label: "plan".to_string(),
            prompt_bytes: 4_000,
        },
    ]);

    let summary = journal.to_summary_json();
    let frontdoor = summary
        .pointer("/task_metrics/frontdoor_llm")
        .and_then(Value::as_object)
        .expect("frontdoor metrics");

    assert_eq!(
        frontdoor.get("first_prompt_label").and_then(Value::as_str),
        Some("normalizer")
    );
    assert_eq!(
        frontdoor
            .get("first_planner_call_index")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        frontdoor
            .get("pre_planner_llm_calls")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        frontdoor
            .get("pre_planner_prompt_bytes")
            .and_then(Value::as_u64),
        Some(2_000)
    );
    assert_eq!(
        frontdoor
            .get("pre_planner_prompt_labels")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert!(!serde_json::to_string(frontdoor)
        .expect("serialize frontdoor metrics")
        .contains("inspect workspace"));
}

#[test]
fn summary_reports_planner_first_target_state() {
    let mut journal = TaskJournal::for_task("task-planner-first", "ask", "hello");
    journal.record_llm_call_sequence(vec![crate::LlmCallSequenceEntry {
        call_index: 1,
        prompt_label: "plan".to_string(),
        prompt_bytes: 2_400,
    }]);

    let summary = journal.to_summary_json();
    assert_eq!(
        summary
            .pointer("/task_metrics/frontdoor_llm/pre_planner_llm_calls")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/frontdoor_llm/first_prompt_label")
            .and_then(Value::as_str),
        Some("plan")
    );
}

#[derive(Debug, Deserialize)]
struct FrontdoorBaselineMetrics {
    schema_version: u32,
    first_semantic_owner: String,
    required_first_prompt_label: String,
    max_pre_planner_llm_calls: u64,
    max_pre_planner_prompt_bytes: u64,
    workloads: Vec<FrontdoorBaselineWorkload>,
}

#[derive(Debug, Deserialize)]
struct FrontdoorBaselineWorkload {
    id: String,
    fixture_kind: String,
    planner_prompt_bytes_fixture: usize,
}

#[test]
fn six_frontdoor_workloads_have_executable_planner_first_baselines() {
    let baseline: FrontdoorBaselineMetrics = toml::from_str(include_str!(
        "../../../../scripts/inventories/frontdoor_baseline_metrics.toml"
    ))
    .expect("frontdoor baseline inventory");
    assert_eq!(baseline.schema_version, 1);
    assert_eq!(baseline.first_semantic_owner, "agent_loop");
    assert_eq!(baseline.required_first_prompt_label, "plan");
    assert_eq!(baseline.max_pre_planner_llm_calls, 0);
    assert_eq!(baseline.max_pre_planner_prompt_bytes, 0);
    assert_eq!(baseline.workloads.len(), 6);

    for workload in baseline.workloads {
        assert!(!workload.fixture_kind.trim().is_empty());
        let mut journal = TaskJournal::for_task(
            format!("task-baseline-{}", workload.id),
            "ask",
            "<redacted-fixture>",
        );
        journal.record_llm_call_sequence(vec![crate::LlmCallSequenceEntry {
            call_index: 1,
            prompt_label: baseline.required_first_prompt_label.clone(),
            prompt_bytes: workload.planner_prompt_bytes_fixture,
        }]);
        let summary = journal.to_summary_json();
        let metrics = summary
            .pointer("/task_metrics/frontdoor_llm")
            .expect("frontdoor metrics");
        assert_eq!(
            metrics.get("first_prompt_label").and_then(Value::as_str),
            Some("plan"),
            "workload {}",
            workload.id
        );
        assert_eq!(
            metrics
                .get("first_planner_call_index")
                .and_then(Value::as_u64),
            Some(1),
            "workload {}",
            workload.id
        );
        assert_eq!(
            metrics.get("pre_planner_llm_calls").and_then(Value::as_u64),
            Some(baseline.max_pre_planner_llm_calls),
            "workload {}",
            workload.id
        );
        assert_eq!(
            metrics
                .get("pre_planner_prompt_bytes")
                .and_then(Value::as_u64),
            Some(baseline.max_pre_planner_prompt_bytes),
            "workload {}",
            workload.id
        );
    }
}
