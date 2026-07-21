use super::*;

pub(crate) fn observed_evidence_for_step_trace(step: &TaskJournalStepTrace) -> Option<Value> {
    observed_evidence_from_step_output(step)
        .or_else(|| observed_evidence_from_error(step.error_excerpt.as_deref()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EvidenceObservationSource {
    StepOutput,
    StepError,
}

impl EvidenceObservationSource {
    fn as_str(self) -> &'static str {
        match self {
            EvidenceObservationSource::StepOutput => "step_output",
            EvidenceObservationSource::StepError => "step_error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EvidenceExtractorKind {
    StructuredJson,
    TextLegacy,
}

impl EvidenceExtractorKind {
    fn as_str(self) -> &'static str {
        match self {
            EvidenceExtractorKind::StructuredJson => "structured_json",
            EvidenceExtractorKind::TextLegacy => "text_legacy",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct EvidenceExtractorSpec {
    pub(super) observation_source: EvidenceObservationSource,
    pub(super) extractor_ref: &'static str,
    pub(super) kind: EvidenceExtractorKind,
    pub(super) format: &'static str,
    pub(super) schema_version: u64,
    pub(super) source_action_ref: Option<&'static str>,
    pub(super) provided_evidence: &'static [&'static str],
    pub(super) strict_shape_eligible: bool,
    pub(super) fallback: bool,
}

impl EvidenceExtractorSpec {
    fn to_trace_json(self) -> Value {
        json!({
            "schema_version": self.schema_version,
            "extractor_ref": self.extractor_ref,
            "kind": self.kind.as_str(),
            "observation_source": self.observation_source.as_str(),
            "format": self.format,
            "source_action_ref": self.source_action_ref,
            "provided_evidence": self.provided_evidence,
            "strict_shape_eligible": self.strict_shape_eligible,
            "fallback": self.fallback,
            "provider_safety": extractor_provider_safety_trace_json(),
        })
    }
}

pub(super) fn extractor_provider_safety_trace_json() -> Value {
    json!({
        "provider_evidence_view": "provider_safe_redacted",
        "raw_excerpt_policy": "no_full_raw_excerpt",
        "storage": "redacted_excerpt_hash",
        "sensitive_field_policy": "redact_sensitive_keys_and_secret_like_values",
    })
}

const EVIDENCE_EXTRACTOR_REGISTRY: &[EvidenceExtractorSpec] = &[
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "step_output.structured_json_v1",
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["generic_json_fields"],
        strict_shape_eligible: false,
        fallback: true,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "step_output.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["legacy_text_excerpt"],
        strict_shape_eligible: false,
        fallback: true,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepError,
        extractor_ref: "step_error.structured_json_v1",
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &[
            "command_output",
            "error_kind",
            "exit_code",
            "field_value",
            "generic_json_fields",
        ],
        strict_shape_eligible: false,
        fallback: true,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepError,
        extractor_ref: "step_error.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: None,
        provided_evidence: &["legacy_error_excerpt"],
        strict_shape_eligible: false,
        fallback: true,
    },
];

const EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY: &[EvidenceExtractorSpec] = &[
    step_json_extractor(
        "workspace_patch",
        "workspace_patch.structured_json_v1",
        &[
            "additions",
            "artifact_refs",
            "changed_files",
            "checkpoint_id",
            "deletions",
            "field_value",
            "patch_id",
            "status",
        ],
    ),
    step_json_extractor(
        "fs_basic",
        "fs_basic.structured_json_v1",
        &[
            "candidates",
            "count",
            "exists",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
            "sort_by",
        ],
    ),
    step_json_extractor(
        "fs_basic.stat_paths",
        "fs_basic.stat_paths.structured_json_v1",
        &[
            "exists",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
        ],
    ),
    step_json_extractor(
        "fs_basic.compare_paths",
        "fs_basic.compare_paths.structured_json_v1",
        &["field_value", "modified_ts", "path", "size_bytes"],
    ),
    step_json_extractor(
        "fs_basic.list_dir",
        "fs_basic.list_dir.structured_json_v1",
        &[
            "candidates",
            "count",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
            "sort_by",
        ],
    ),
    step_json_extractor(
        "fs_basic.find_entries",
        "fs_basic.find_entries.structured_json_v1",
        &["candidates", "count", "path"],
    ),
    step_json_extractor(
        "fs_basic.count_entries",
        "fs_basic.count_entries.structured_json_v1",
        &["count", "field_value", "size_bytes"],
    ),
    step_json_extractor(
        "system_basic.tree_summary",
        "system_basic.tree_summary.structured_json_v1",
        &["candidates", "count", "kind", "path", "size_bytes"],
    ),
    step_json_extractor(
        "system_basic.inventory_dir",
        "system_basic.inventory_dir.structured_json_v1",
        &[
            "candidates",
            "count",
            "field_value",
            "kind",
            "modified_ts",
            "path",
            "size_bytes",
            "sort_by",
        ],
    ),
    step_json_extractor(
        "system_basic.read_range",
        "system_basic.read_range.structured_json_v1",
        &[
            "content_excerpt",
            "first_line",
            "line_count",
            "path",
            "total_lines",
        ],
    ),
    step_json_extractor(
        "system_basic.extract_field",
        "system_basic.extract_field.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "system_basic.extract_fields",
        "system_basic.extract_fields.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "system_basic.path_batch_facts",
        "system_basic.path_batch_facts.structured_json_v1",
        &[
            "candidates",
            "count",
            "exists",
            "kind",
            "path",
            "size_bytes",
        ],
    ),
    step_json_extractor(
        "system_basic.info",
        "system_basic.info.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "system_basic.runtime_status",
        "system_basic.runtime_status.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "fs_basic.grep_text",
        "fs_basic.grep_text.structured_json_v1",
        &["candidates", "command_output", "content_excerpt", "path"],
    ),
    step_json_extractor(
        "fs_basic.read_text_range",
        "fs_basic.read_text_range.structured_json_v1",
        &[
            "content_excerpt",
            "field_value",
            "first_line",
            "line_count",
            "path",
            "total_lines",
        ],
    ),
    step_json_extractor(
        "fs_basic.write_text",
        "fs_basic.write_text.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "fs_basic.append_text",
        "fs_basic.append_text.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "fs_basic.make_dir",
        "fs_basic.make_dir.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "fs_basic.remove_path",
        "fs_basic.remove_path.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "config_basic.read_field",
        "config_basic.read_field.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_basic.read_fields",
        "config_basic.read_fields.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_basic.list_keys",
        "config_basic.list_keys.structured_json_v1",
        &["count", "field_value"],
    ),
    step_json_extractor(
        "config_basic.validate",
        "config_basic.validate.structured_json_v1",
        &["field_value", "valid"],
    ),
    step_json_extractor(
        "config_basic.guard_rustclaw_config",
        "config_basic.guard_rustclaw_config.structured_json_v1",
        &["candidates", "count", "valid"],
    ),
    step_json_extractor(
        "config_basic",
        "config_basic.structured_json_v1",
        &["count", "field_value", "valid"],
    ),
    step_json_extractor(
        "config_edit.guard_config",
        "config_edit.guard_config.structured_json_v1",
        &["candidates", "count", "valid"],
    ),
    step_json_extractor(
        "config_edit.plan_config_change",
        "config_edit.plan_config_change.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_edit.apply_config_change",
        "config_edit.apply_config_change.structured_json_v1",
        &["field_value", "path", "valid"],
    ),
    step_json_extractor(
        "config_edit.validate_config",
        "config_edit.validate_config.structured_json_v1",
        &["field_value", "valid"],
    ),
    step_json_extractor(
        "config_edit.read_back",
        "config_edit.read_back.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "config_edit.restart_if_requested",
        "config_edit.restart_if_requested.structured_json_v1",
        &["field_value"],
    ),
    step_json_extractor(
        "config_guard",
        "config_guard.structured_json_v1",
        &["candidates", "count", "valid"],
    ),
    step_json_extractor(
        "db_basic",
        "db_basic.structured_json_v1",
        &["candidates", "count", "field_value"],
    ),
    step_json_extractor(
        "doc_parse",
        "doc_parse.structured_json_v1",
        &["content_excerpt", "path"],
    ),
    step_json_extractor(
        "git_basic",
        "git_basic.structured_json_v1",
        &[
            "command_output",
            "content_excerpt",
            "field_value",
            "status",
            "subject",
        ],
    ),
    step_json_extractor(
        "health_check",
        "health_check.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "http_basic",
        "http_basic.structured_json_v1",
        &[
            "content_excerpt",
            "field_value",
            "status",
            "status_code",
            "url",
        ],
    ),
    step_json_extractor(
        "http_basic.get",
        "http_basic.get.structured_json_v1",
        &[
            "content_excerpt",
            "field_value",
            "status",
            "status_code",
            "url",
        ],
    ),
    step_json_extractor(
        "log_analyze",
        "log_analyze.structured_json_v1",
        &["field_value", "content_excerpt"],
    ),
    step_json_extractor(
        "package_manager.detect",
        "package_manager.detect.structured_json_v1",
        &["field_value"],
    ),
    step_json_extractor(
        "package_manager.smart_install",
        "package_manager.smart_install.structured_json_v1",
        &["command_output", "field_value", "status"],
    ),
    step_json_extractor(
        "process_basic",
        "process_basic.structured_json_v1",
        &[
            "count",
            "field_value",
            "listeners",
            "ports",
            "all_interface_listeners",
            "all_interface_ports",
            "status",
        ],
    ),
    step_json_extractor(
        "docker_basic",
        "docker_basic.structured_json_v1",
        &["candidates", "field_value", "status"],
    ),
    step_json_extractor(
        "service_control",
        "service_control.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "transform",
        "transform.structured_json_v1",
        &["field_value", "content_excerpt"],
    ),
    step_json_extractor(
        "transform.transform_data",
        "transform.transform_data.structured_json_v1",
        &["field_value", "content_excerpt"],
    ),
    step_json_extractor(
        "audio_synthesize",
        "audio_synthesize.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "audio_synthesize.synthesize",
        "audio_synthesize.synthesize.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "audio_synthesize.poll",
        "audio_synthesize.poll.structured_json_v1",
        &["field_value", "path", "status"],
    ),
    step_json_extractor(
        "audio_synthesize.cancel",
        "audio_synthesize.cancel.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "rss_fetch",
        "rss_fetch.structured_json_v1",
        &["candidates", "content_excerpt", "field_value"],
    ),
    step_json_extractor("x", "x.structured_json_v1", &["field_value"]),
    step_json_extractor(
        "image_generate",
        "image_generate.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "image_generate.generate",
        "image_generate.generate.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "image_generate.poll",
        "image_generate.poll.structured_json_v1",
        &["field_value", "path", "status"],
    ),
    step_json_extractor(
        "image_generate.cancel",
        "image_generate.cancel.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "image_edit",
        "image_edit.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "image_edit.edit",
        "image_edit.edit.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "image_edit.outpaint",
        "image_edit.outpaint.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "image_edit.restyle",
        "image_edit.restyle.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "archive_basic",
        "archive_basic.structured_json_v1",
        &[
            "artifact_refs",
            "candidates",
            "content_excerpt",
            "count",
            "field_value",
            "path",
        ],
    ),
    step_json_extractor(
        "archive_basic.list",
        "archive_basic.list.structured_json_v1",
        &["candidates", "count", "path"],
    ),
    step_json_extractor(
        "archive_basic.read",
        "archive_basic.read.structured_json_v1",
        &["content_excerpt", "path"],
    ),
    step_json_extractor(
        "browser_web",
        "browser_web.structured_json_v1",
        &["content_excerpt", "field_value", "path"],
    ),
    step_json_extractor(
        "browser_web.open_extract",
        "browser_web.open_extract.structured_json_v1",
        &["content_excerpt", "field_value", "path"],
    ),
    step_json_extractor(
        "web_search_extract",
        "web_search_extract.structured_json_v1",
        &["candidates", "field_value"],
    ),
    step_json_extractor(
        "web_search_extract.search",
        "web_search_extract.search.structured_json_v1",
        &["candidates", "field_value"],
    ),
    step_json_extractor(
        "web_search_extract.search_extract",
        "web_search_extract.search_extract.structured_json_v1",
        &["candidates", "field_value"],
    ),
    step_json_extractor(
        "weather",
        "weather.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "weather.query",
        "weather.query.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "stock",
        "stock.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "stock.quote",
        "stock.quote.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto",
        "crypto.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto.quote",
        "crypto.quote.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto.multi_quote",
        "crypto.multi_quote.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "crypto.positions",
        "crypto.positions.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision",
        "image_vision.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.describe",
        "image_vision.describe.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.analyze",
        "image_vision.analyze.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.extract",
        "image_vision.extract.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.compare",
        "image_vision.compare.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "image_vision.screenshot_summary",
        "image_vision.screenshot_summary.structured_json_v1",
        &["content_excerpt", "field_value"],
    ),
    step_json_extractor(
        "video_generate",
        "video_generate.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "video_generate.generate",
        "video_generate.generate.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "video_generate.poll",
        "video_generate.poll.structured_json_v1",
        &["field_value", "path", "status"],
    ),
    step_json_extractor(
        "video_generate.cancel",
        "video_generate.cancel.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "music_generate",
        "music_generate.structured_json_v1",
        &["path"],
    ),
    step_json_extractor(
        "music_generate.generate",
        "music_generate.generate.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "music_generate.poll",
        "music_generate.poll.structured_json_v1",
        &["field_value", "path", "status"],
    ),
    step_json_extractor(
        "music_generate.cancel",
        "music_generate.cancel.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "photo_organize.prepare",
        "photo_organize.prepare.structured_json_v1",
        &["candidates", "field_value", "path"],
    ),
    step_json_extractor(
        "photo_organize.preview",
        "photo_organize.preview.structured_json_v1",
        &["candidates", "field_value", "path"],
    ),
    step_json_extractor(
        "photo_organize.organize",
        "photo_organize.organize.structured_json_v1",
        &["field_value", "path", "status"],
    ),
    step_json_extractor(
        "archive_basic.pack",
        "archive_basic.pack.structured_json_v1",
        &["artifact_refs", "field_value", "path"],
    ),
    step_json_extractor(
        "archive_basic.unpack",
        "archive_basic.unpack.structured_json_v1",
        &["field_value", "path"],
    ),
    step_json_extractor(
        "kb.ingest",
        "kb.ingest.structured_json_v1",
        &["count", "field_value", "path"],
    ),
    step_json_extractor(
        "kb.search",
        "kb.search.structured_json_v1",
        &[
            "candidates",
            "content_excerpt",
            "count",
            "field_value",
            "path",
        ],
    ),
    step_json_extractor(
        "kb.list_namespaces",
        "kb.list_namespaces.structured_json_v1",
        &["candidates", "count", "field_value", "status"],
    ),
    step_json_extractor(
        "kb.stats",
        "kb.stats.structured_json_v1",
        &["count", "field_value", "status"],
    ),
    step_json_extractor(
        "task_control.list",
        "task_control.list.structured_json_v1",
        &["content_excerpt", "field_value", "status"],
    ),
    step_json_extractor(
        "task_control.list_with_first_detail",
        "task_control.list_with_first_detail.structured_json_v1",
        &["content_excerpt", "field_value", "status"],
    ),
    step_json_extractor(
        "task_control.get",
        "task_control.get.structured_json_v1",
        &["field_value", "status"],
    ),
    step_json_extractor(
        "schedule.preview",
        "schedule.preview.structured_json_v1",
        &["datetime", "timezone", "title"],
    ),
    step_text_extractor(
        "http_basic",
        "http_basic.text_legacy_v1",
        &["command_output", "content_excerpt", "field_value", "status"],
    ),
    step_text_extractor(
        "write_file",
        "write_file.text_legacy_v1",
        &["legacy_machine_tokens", "path"],
    ),
    step_text_extractor(
        "x",
        "x.text_legacy_v1",
        &["field_value", "legacy_machine_tokens"],
    ),
    step_text_extractor(
        "task_control.list",
        "task_control.list.text_legacy_v1",
        &["content_excerpt", "field_value", "status"],
    ),
    step_text_extractor(
        "task_control.get",
        "task_control.get.text_legacy_v1",
        &["field_value", "status"],
    ),
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "run_cmd.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: Some("run_cmd"),
        provided_evidence: &["command_output", "legacy_machine_tokens"],
        strict_shape_eligible: true,
        fallback: false,
    },
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "list_dir.text_legacy_v1",
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: Some("list_dir"),
        provided_evidence: &["candidates", "count", "legacy_machine_tokens"],
        strict_shape_eligible: true,
        fallback: false,
    },
];

const MATRIX_ADMITTED_EXTERNAL_STRUCTURED_JSON_EXTRACTOR: EvidenceExtractorSpec =
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref: "matrix_admitted_external.structured_json_v1",
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: Some("matrix_admitted_external"),
        provided_evidence: &["admitted_extra_fields"],
        strict_shape_eligible: true,
        fallback: false,
    };

const fn step_json_extractor(
    source_action_ref: &'static str,
    extractor_ref: &'static str,
    provided_evidence: &'static [&'static str],
) -> EvidenceExtractorSpec {
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref,
        kind: EvidenceExtractorKind::StructuredJson,
        format: "json",
        schema_version: 1,
        source_action_ref: Some(source_action_ref),
        provided_evidence,
        strict_shape_eligible: true,
        fallback: false,
    }
}

const fn step_text_extractor(
    source_action_ref: &'static str,
    extractor_ref: &'static str,
    provided_evidence: &'static [&'static str],
) -> EvidenceExtractorSpec {
    EvidenceExtractorSpec {
        observation_source: EvidenceObservationSource::StepOutput,
        extractor_ref,
        kind: EvidenceExtractorKind::TextLegacy,
        format: "text",
        schema_version: 1,
        source_action_ref: Some(source_action_ref),
        provided_evidence,
        strict_shape_eligible: true,
        fallback: false,
    }
}

pub(super) fn evidence_extractor_spec(
    observation_source: EvidenceObservationSource,
    kind: EvidenceExtractorKind,
) -> EvidenceExtractorSpec {
    EVIDENCE_EXTRACTOR_REGISTRY
        .iter()
        .copied()
        .find(|spec| spec.observation_source == observation_source && spec.kind == kind)
        .expect("evidence extractor registry contains all built-in extractor specs")
}

pub(crate) fn evidence_extractor_registry_trace(
    source_action_ref: &str,
    extractor_kind: &str,
) -> Option<Value> {
    explicit_evidence_extractor_spec(source_action_ref, extractor_kind).map(|spec| {
        json!({
            "extractor_ref": spec.extractor_ref,
            "source_action_ref": spec.source_action_ref,
            "provided_evidence": spec.provided_evidence,
            "strict_shape_eligible": spec.strict_shape_eligible,
            "fallback": spec.fallback,
            "provider_safety": extractor_provider_safety_trace_json(),
        })
    })
}

pub(crate) fn evidence_extractor_registry_contains(
    source_action_ref: &str,
    extractor_kind: &str,
) -> bool {
    explicit_evidence_extractor_spec(source_action_ref, extractor_kind).is_some()
}

pub(super) fn explicit_text_extractor_provides(extractor_ref: &str, field: &str) -> bool {
    EVIDENCE_EXTRACTOR_REGISTRY
        .iter()
        .chain(EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY)
        .chain(std::iter::once(
            &MATRIX_ADMITTED_EXTERNAL_STRUCTURED_JSON_EXTRACTOR,
        ))
        .any(|spec| {
            spec.extractor_ref == extractor_ref
                && spec.kind == EvidenceExtractorKind::TextLegacy
                && !spec.fallback
                && spec
                    .provided_evidence
                    .iter()
                    .any(|provided| *provided == field)
        })
}

pub(super) fn explicit_evidence_extractor_spec(
    source_action_ref: &str,
    extractor_kind: &str,
) -> Option<EvidenceExtractorSpec> {
    let source_action_ref = normalize_source_action_ref(source_action_ref)?;
    let kind = parse_evidence_extractor_kind(extractor_kind)?;
    EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY
        .iter()
        .copied()
        .find(|spec| {
            spec.kind == kind
                && spec
                    .source_action_ref
                    .is_some_and(|value| value == source_action_ref)
        })
}

pub(super) fn parse_evidence_extractor_kind(extractor_kind: &str) -> Option<EvidenceExtractorKind> {
    match normalize_machine_token(extractor_kind).as_str() {
        "structured_json" => Some(EvidenceExtractorKind::StructuredJson),
        "text_legacy" => Some(EvidenceExtractorKind::TextLegacy),
        _ => None,
    }
}

pub(crate) fn observed_evidence_from_output(output: Option<&str>) -> Option<Value> {
    let output = output.map(str::trim).filter(|value| !value.is_empty())?;
    let (collector, extractor) = collect_observed_evidence_from_output(output);
    observed_evidence_from_collector(collector, extractor)
}

pub(super) fn observed_evidence_from_step_output(step: &TaskJournalStepTrace) -> Option<Value> {
    let output = step
        .output_excerpt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let fallback_extractor = match serde_json::from_str::<Value>(output) {
        Ok(value) => {
            let mut collector = ObservedEvidenceCollector::default();
            collect_embedded_http_json_body_evidence(&mut collector, &value);
            collect_priority_json_status_scalar_evidence(
                &mut collector,
                "json_output",
                "",
                &value,
                0,
            );
            collect_json_observed_evidence(&mut collector, "json_output", "", &value, 0);
            if normalize_machine_token(&step.skill).replace('-', "_") == "schedule" {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    collect_text_observed_evidence_fields(&mut collector, text);
                }
            }
            let fallback_extractor = evidence_extractor_spec(
                EvidenceObservationSource::StepOutput,
                EvidenceExtractorKind::StructuredJson,
            );
            let extractor =
                explicit_step_output_extractor_spec(step, output, fallback_extractor.kind)
                    .unwrap_or(fallback_extractor);
            return observed_evidence_from_collector(collector, extractor);
        }
        Err(_) => evidence_extractor_spec(
            EvidenceObservationSource::StepOutput,
            EvidenceExtractorKind::TextLegacy,
        ),
    };
    let extractor = explicit_step_output_extractor_spec(step, output, fallback_extractor.kind)
        .unwrap_or(fallback_extractor);
    let mut collector = ObservedEvidenceCollector::default();
    collect_text_observed_evidence_for_extractor(&mut collector, output, extractor);
    observed_evidence_from_collector(collector, extractor)
}

pub(super) fn collect_observed_evidence_from_output(
    output: &str,
) -> (ObservedEvidenceCollector, EvidenceExtractorSpec) {
    let mut collector = ObservedEvidenceCollector::default();
    let extractor = match serde_json::from_str::<Value>(output) {
        Ok(value) => {
            collect_embedded_http_json_body_evidence(&mut collector, &value);
            collect_priority_json_status_scalar_evidence(
                &mut collector,
                "json_output",
                "",
                &value,
                0,
            );
            collect_json_observed_evidence(&mut collector, "json_output", "", &value, 0);
            evidence_extractor_spec(
                EvidenceObservationSource::StepOutput,
                EvidenceExtractorKind::StructuredJson,
            )
        }
        Err(_) => {
            collect_text_observed_evidence(&mut collector, output);
            evidence_extractor_spec(
                EvidenceObservationSource::StepOutput,
                EvidenceExtractorKind::TextLegacy,
            )
        }
    };
    (collector, extractor)
}

pub(super) fn observed_evidence_from_collector(
    mut collector: ObservedEvidenceCollector,
    extractor: EvidenceExtractorSpec,
) -> Option<Value> {
    if collector.items.is_empty() {
        return None;
    }
    let item_count = collector.total_count;
    prioritize_observed_evidence_for_storage(&mut collector.items);
    Some(json!({
        "schema_version": 1,
        "source": "step_output",
        "format": extractor.format,
        "extractor": extractor.to_trace_json(),
        "storage": "redacted_excerpt_hash",
        "item_count": item_count,
        "truncated": item_count > collector.items.len(),
        "items": collector.items,
    }))
}

pub(super) fn explicit_step_output_extractor_spec(
    step: &TaskJournalStepTrace,
    output: &str,
    kind: EvidenceExtractorKind,
) -> Option<EvidenceExtractorSpec> {
    step_output_source_action_refs(step, output)
        .into_iter()
        .find_map(|source_action_ref| {
            EXPLICIT_EVIDENCE_EXTRACTOR_REGISTRY
                .iter()
                .copied()
                .find(|spec| {
                    spec.kind == kind
                        && spec
                            .source_action_ref
                            .is_some_and(|value| value == source_action_ref)
                })
        })
        .or_else(|| matrix_admitted_external_extractor_spec(output, kind))
}

pub(super) fn matrix_admitted_external_extractor_spec(
    output: &str,
    kind: EvidenceExtractorKind,
) -> Option<EvidenceExtractorSpec> {
    if kind != EvidenceExtractorKind::StructuredJson {
        return None;
    }
    let value = serde_json::from_str::<Value>(output).ok()?;
    let admission = value.get("_matrix_admission")?;
    if !admission
        .get("eligible")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let extractor_kind = admission
        .get("extractor_kind")
        .and_then(Value::as_str)
        .map(normalize_machine_token)
        .unwrap_or_else(|| "structured_json".to_string());
    if extractor_kind != kind.as_str() {
        return None;
    }
    Some(MATRIX_ADMITTED_EXTERNAL_STRUCTURED_JSON_EXTRACTOR)
}

pub(super) fn step_output_source_action_refs(
    step: &TaskJournalStepTrace,
    output: &str,
) -> Vec<String> {
    let mut refs = Vec::new();
    let skill = normalize_machine_token(&step.skill).replace('-', "_");
    if skill.is_empty() {
        return refs;
    }
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        let value_has_action = value.get("action").and_then(Value::as_str).is_some();
        let extra_action_preferred =
            matches!(skill.as_str(), "fs_basic" | "config_basic" | "system_basic");
        let extra_value = value
            .get("extra")
            .filter(|extra| extra.is_object())
            .cloned();
        if value_has_action {
            push_source_action_ref(&mut refs, &skill, Some(&value));
        } else if !extra_action_preferred {
            push_source_action_ref(&mut refs, &skill, Some(&value));
        }
        if let Some(extra) = extra_value.as_ref() {
            push_source_action_ref(&mut refs, &skill, Some(extra));
        }
        if skill == "system_basic"
            && value.get("action").and_then(Value::as_str).is_none()
            && value_looks_like_system_basic_info(&value)
        {
            push_unique_source_action_ref(&mut refs, "system_basic.info".to_string());
        }
        if !value_has_action && extra_action_preferred {
            push_source_action_ref(&mut refs, &skill, Some(&value));
        }
        push_canonical_source_action_ref(&mut refs, &skill, value.clone());
        if let Some(extra) = extra_value.as_ref() {
            push_canonical_source_action_ref(&mut refs, &skill, extra.clone());
        }
        if skill == "fs_basic" {
            push_canonical_source_action_ref(&mut refs, "fs_search", value.clone());
            if let Some(extra) = extra_value.as_ref() {
                push_canonical_source_action_ref(&mut refs, "fs_search", extra.clone());
            }
        }
        if matches!(skill.as_str(), "fs_basic" | "config_basic" | "system_basic") {
            push_canonical_source_action_ref(&mut refs, "system_basic", value);
            if let Some(extra) = extra_value.as_ref() {
                push_canonical_source_action_ref(&mut refs, "system_basic", extra.clone());
            }
        }
    }
    push_source_action_ref(&mut refs, &skill, None);
    refs
}

pub(super) fn value_looks_like_system_basic_info(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.contains_key("cwd")
        || object.contains_key("workspace_root")
        || (object.contains_key("hostname")
            && (object.contains_key("os") || object.contains_key("arch")))
    {
        return true;
    }
    object
        .get("extra")
        .is_some_and(value_looks_like_system_basic_info)
}

pub(super) fn push_source_action_ref(refs: &mut Vec<String>, skill: &str, value: Option<&Value>) {
    let source = match value
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str)
    {
        Some(action) => {
            let action = normalize_machine_token(action).replace('-', "_");
            let action = canonical_evidence_source_action_token_for_skill(skill, action);
            format!("{skill}.{action}")
        }
        None => skill.to_string(),
    };
    if let Some(source) = normalize_source_action_ref(&source) {
        push_unique_source_action_ref(refs, source);
    }
}

pub(super) fn push_canonical_source_action_ref(refs: &mut Vec<String>, skill: &str, value: Value) {
    let Some(canonical) = crate::virtual_tools::canonicalize_legacy_tool_call(skill, value) else {
        return;
    };
    let Some(source) = canonical_source_action_ref(&canonical.tool, &canonical.args) else {
        return;
    };
    push_unique_source_action_ref(refs, source);
}

pub(super) fn canonical_source_action_ref(skill: &str, args: &Value) -> Option<String> {
    let skill = normalize_machine_token(skill).replace('-', "_");
    if skill.is_empty() {
        return None;
    }
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_machine_token)
        .map(|value| value.replace('-', "_"))
        .filter(|value| !value.is_empty());
    normalize_source_action_ref(&match action {
        Some(action) => format!("{skill}.{action}"),
        None => skill,
    })
}

pub(super) fn canonical_evidence_source_action_token_for_skill(
    skill: &str,
    action: String,
) -> String {
    match (skill, action.as_str()) {
        ("fs_basic", "inventory_dir") => "list_dir".to_string(),
        ("fs_basic", "read_range") => "read_text_range".to_string(),
        _ => action,
    }
}

pub(super) fn normalize_source_action_ref(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (skill, action) = raw
        .split_once('.')
        .map_or((raw, None), |(skill, action)| (skill, Some(action)));
    let skill = normalize_machine_token(skill).replace('-', "_");
    if skill.is_empty() {
        return None;
    }
    let action = action
        .map(normalize_machine_token)
        .map(|value| value.replace('-', "_"))
        .filter(|value| !value.is_empty());
    Some(match action {
        Some(action) => format!("{skill}.{action}"),
        None => skill,
    })
}

pub(super) fn push_unique_source_action_ref(refs: &mut Vec<String>, source: String) {
    if !refs.iter().any(|value| value == &source) {
        refs.push(source);
    }
}

pub(super) fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn observed_evidence_from_error(error: Option<&str>) -> Option<Value> {
    let error = error.map(str::trim).filter(|value| !value.is_empty())?;
    let mut collector = ObservedEvidenceCollector::default();
    let extractor = if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        collector.push(json_observed_evidence_item(
            "structured_error",
            "error_kind",
            &json!(structured.error_kind),
        ));
        if !structured.skill.trim().is_empty() {
            collector.push(json_observed_evidence_item(
                "structured_error",
                "skill",
                &json!(structured.skill),
            ));
        }
        if let Some(extra) = structured.extra.as_ref() {
            collect_json_observed_evidence(&mut collector, "structured_error.extra", "", extra, 0);
        }
        collect_structured_error_not_found_evidence(&mut collector, &structured);
        collect_structured_error_command_output_evidence(&mut collector, &structured);
        evidence_extractor_spec(
            EvidenceObservationSource::StepError,
            EvidenceExtractorKind::StructuredJson,
        )
    } else {
        collect_text_observed_evidence(&mut collector, error);
        evidence_extractor_spec(
            EvidenceObservationSource::StepError,
            EvidenceExtractorKind::TextLegacy,
        )
    };
    if collector.items.is_empty() {
        return None;
    }
    let item_count = collector.total_count;
    Some(json!({
        "schema_version": 1,
        "source": "step_error",
        "format": extractor.format,
        "extractor": extractor.to_trace_json(),
        "storage": "redacted_excerpt_hash",
        "item_count": item_count,
        "truncated": item_count > collector.items.len(),
        "items": collector.items,
    }))
}

pub(super) fn collect_structured_error_not_found_evidence(
    collector: &mut ObservedEvidenceCollector,
    structured: &crate::skills::StructuredSkillError,
) {
    if structured.error_kind != "not_found" {
        return;
    }
    collector.push(json_observed_evidence_item(
        "structured_error",
        "exists",
        &json!(false),
    ));
    collector.push(json_observed_evidence_item(
        "structured_error",
        "kind",
        &json!("missing"),
    ));
    if let Some(path) = structured
        .extra
        .as_ref()
        .and_then(|extra| extra.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        collector.push(json_observed_evidence_item(
            "structured_error",
            "path",
            &json!(path),
        ));
    }
}

pub(super) fn collect_structured_error_command_output_evidence(
    collector: &mut ObservedEvidenceCollector,
    structured: &crate::skills::StructuredSkillError,
) {
    let has_run_cmd_failure_shape = structured.skill.eq_ignore_ascii_case("run_cmd")
        || structured
            .extra
            .as_ref()
            .is_some_and(|extra| extra.get("exit_code").is_some() || extra.get("stderr").is_some());
    if !has_run_cmd_failure_shape {
        return;
    }
    collector.push(json_observed_evidence_item(
        "structured_error",
        "error_kind",
        &json!(structured.error_kind),
    ));
    if let Some(extra) = structured.extra.as_ref() {
        for field in ["exit_code", "exit_category", "stderr", "stdout", "command"] {
            if let Some(value) = extra.get(field) {
                collector.push(json_observed_evidence_item(
                    "structured_error.extra",
                    field,
                    value,
                ));
            }
        }
    }
}
