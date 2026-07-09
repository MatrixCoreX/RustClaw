#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum FinalizerRendererShapeClass {
    CapabilityResult,
    FinalAnswerShape,
    ArtifactDelivery,
    TaskLifecycle,
    CompatibilityFallback,
}

impl FinalizerRendererShapeClass {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::CapabilityResult => "capability_result",
            Self::FinalAnswerShape => "final_answer_shape",
            Self::ArtifactDelivery => "artifact_delivery",
            Self::TaskLifecycle => "task_lifecycle",
            Self::CompatibilityFallback => "compatibility_fallback",
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
        key: "service_status_observed_fields",
        shape_class: FinalizerRendererShapeClass::CapabilityResult,
        owner_module: "finalize::loop_reply_service_status",
        entrypoint: "replace_delivery_with_service_status_observed_answer",
        summary_contract: "finalizer_summary",
    },
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
        key: "raw_command_machine_field_projection",
        shape_class: FinalizerRendererShapeClass::FinalAnswerShape,
        owner_module: "finalize::loop_reply_raw_command",
        entrypoint: "replace_final_delivery_with_raw_command_machine_field_projection",
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
        key: "config_edit_observed_answer",
        shape_class: FinalizerRendererShapeClass::CapabilityResult,
        owner_module: "finalize::loop_reply_config_edit",
        entrypoint: "direct_config_edit_observed_answer",
        summary_contract: "finalizer_summary",
    },
    FinalizerRendererDescriptor {
        key: "scalar_placeholder_terminal_direct_answer",
        shape_class: FinalizerRendererShapeClass::CompatibilityFallback,
        owner_module: "finalize::loop_reply",
        entrypoint: "replace_scalar_placeholder_delivery_with_direct_scalar_answer",
        summary_contract: "finalizer_summary",
    },
];
