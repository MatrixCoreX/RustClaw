#![allow(dead_code)]

use serde_json::{json, Value};

use crate::agent_engine::LoopState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum FinalizerRendererShapeClass {
    FinalAnswerShape,
    ArtifactDelivery,
    TaskLifecycle,
    DeterministicFallback,
}

impl FinalizerRendererShapeClass {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::FinalAnswerShape => "final_answer_shape",
            Self::ArtifactDelivery => "artifact_delivery",
            Self::TaskLifecycle => "task_lifecycle",
            Self::DeterministicFallback => "deterministic_fallback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FinalizerRendererDescriptor {
    pub(super) key: &'static str,
    pub(super) shape_class: FinalizerRendererShapeClass,
    pub(super) owner_module: &'static str,
    pub(super) entrypoint: &'static str,
    pub(super) summary_contract: &'static str,
}

pub(super) const FINALIZER_RENDERER_REGISTRY: &[FinalizerRendererDescriptor] = &[
    FinalizerRendererDescriptor {
        key: "matrix_observed_shape",
        shape_class: FinalizerRendererShapeClass::FinalAnswerShape,
        owner_module: "finalize::loop_reply_matrix_shape",
        entrypoint: "replace_delivery_with_matrix_observed_shape_answer",
        summary_contract: "finalizer_summary",
    },
    FinalizerRendererDescriptor {
        key: "machine_kv_summary",
        shape_class: FinalizerRendererShapeClass::FinalAnswerShape,
        owner_module: "finalize::loop_reply_machine_kv",
        entrypoint: "replace_delivery_with_requested_machine_kv_summary",
        summary_contract: "finalizer_summary",
    },
    FinalizerRendererDescriptor {
        key: "exact_observation_machine_field_projection",
        shape_class: FinalizerRendererShapeClass::FinalAnswerShape,
        owner_module: "finalize::loop_reply_exact_observation",
        entrypoint: "replace_final_delivery_with_exact_observation_machine_field_projection",
        summary_contract: "finalizer_summary",
    },
    FinalizerRendererDescriptor {
        key: "file_token_delivery",
        shape_class: FinalizerRendererShapeClass::ArtifactDelivery,
        owner_module: "finalize::loop_reply_file_delivery",
        entrypoint: "normalize_file_token_delivery_from_observed_paths",
        summary_contract: "delivery_token",
    },
    FinalizerRendererDescriptor {
        key: "agent_loop_clarify_machine_line",
        shape_class: FinalizerRendererShapeClass::TaskLifecycle,
        owner_module: "finalize::loop_reply_clarify_envelope",
        entrypoint: "attach_agent_loop_clarify_machine_line",
        summary_contract: "terminal_clarify_machine_line",
    },
    FinalizerRendererDescriptor {
        key: "route_clarify_machine_envelope",
        shape_class: FinalizerRendererShapeClass::TaskLifecycle,
        owner_module: "finalize::loop_reply_clarify_envelope",
        entrypoint: "attach_route_clarify_machine_envelope",
        summary_contract: "terminal_clarify",
    },
    FinalizerRendererDescriptor {
        key: "control_machine_envelope",
        shape_class: FinalizerRendererShapeClass::TaskLifecycle,
        owner_module: "finalize::loop_reply_control_envelope",
        entrypoint: "attach_requested_control_machine_envelope",
        summary_contract: "control_intent",
    },
    FinalizerRendererDescriptor {
        key: "scalar_placeholder_terminal_direct_answer",
        shape_class: FinalizerRendererShapeClass::DeterministicFallback,
        owner_module: "finalize::loop_reply_scalar_placeholder",
        entrypoint: "replace_scalar_placeholder_delivery_with_direct_scalar_answer",
        summary_contract: "finalizer_summary",
    },
];

pub(super) fn renderers_for_shape_class(
    shape_class: FinalizerRendererShapeClass,
) -> impl Iterator<Item = &'static FinalizerRendererDescriptor> {
    FINALIZER_RENDERER_REGISTRY
        .iter()
        .filter(move |renderer| renderer.shape_class == shape_class)
}

pub(super) fn record_renderer_trace(
    loop_state: &mut LoopState,
    renderer: &FinalizerRendererDescriptor,
    rendered: bool,
    evidence_refs: Vec<String>,
    failure_reason: Option<&'static str>,
) {
    let payload = json!({
        "schema_version": 1,
        "kind": "finalizer_renderer_trace",
        "renderer_key": renderer.key,
        "shape": renderer.shape_class.as_str(),
        "summary_contract": renderer.summary_contract,
        "owner_module": renderer.owner_module,
        "entrypoint": renderer.entrypoint,
        "disposition": if rendered { "rendered" } else { "skipped" },
        "failure_reason": failure_reason,
        "evidence_refs": evidence_refs,
    });
    loop_state.output_vars.insert(
        format!("finalizer.renderer_trace.{}", renderer.key),
        payload.to_string(),
    );
    upsert_renderer_trace_index(loop_state, payload.clone());
    loop_state.task_observations.push(payload);
}

fn upsert_renderer_trace_index(loop_state: &mut LoopState, payload: Value) {
    let renderer_key = payload
        .get("renderer_key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut traces = loop_state
        .output_vars
        .get("finalizer.renderer_traces")
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(raw).ok())
        .unwrap_or_default();
    traces.retain(|trace| {
        trace
            .get("renderer_key")
            .and_then(Value::as_str)
            .map(|existing| existing != renderer_key)
            .unwrap_or(true)
    });
    traces.push(payload);
    loop_state.output_vars.insert(
        "finalizer.renderer_traces".to_string(),
        json!(traces).to_string(),
    );
}
