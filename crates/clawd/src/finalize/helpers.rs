use std::path::Path;

#[cfg(test)]
use serde::Deserialize;
use serde_json::Value;
#[cfg(test)]
use std::collections::HashSet;

#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(crate) struct FinalizerSchemaOut {
    #[serde(default)]
    pub(crate) answer: String,
    #[serde(default)]
    pub(crate) completion_ok: bool,
    #[serde(default)]
    pub(crate) grounded_ok: bool,
    #[serde(default)]
    pub(crate) format_ok: bool,
    #[serde(default)]
    pub(crate) needs_clarify: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) confidence: f64,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) used_evidence_ids: Vec<String>,
    #[serde(default)]
    pub(crate) evidence_quotes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalizerDisposition {
    QualifiedCompletion,
    AllowFallback,
    #[cfg(test)]
    MustFail,
}

impl Default for FinalizerDisposition {
    fn default() -> Self {
        Self::AllowFallback
    }
}

impl FinalizerDisposition {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::QualifiedCompletion => "qualified_completion",
            Self::AllowFallback => "allow_fallback",
            #[cfg(test)]
            Self::MustFail => "must_fail",
        }
    }
}

pub(crate) fn should_attempt_observed_fallback(
    has_tool_or_skill_output: bool,
    has_recoverable_failure_context: bool,
) -> bool {
    has_tool_or_skill_output || has_recoverable_failure_context
}

pub(crate) const EXECUTION_SUMMARY_MESSAGE_PREFIX: &str = "**执行过程**";
pub(crate) const EXECUTION_SUMMARY_MESSAGE_PREFIX_EN: &str = "**Execution**";

pub(crate) fn is_execution_summary_message(message: &str) -> bool {
    let trimmed = message.trim_start();
    trimmed.starts_with(EXECUTION_SUMMARY_MESSAGE_PREFIX)
        || trimmed.starts_with(EXECUTION_SUMMARY_MESSAGE_PREFIX_EN)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedOutputKind {
    Empty,
    PlannerArtifact,
    DeliveryToken,
    Structured,
    Content,
    Error,
}

impl ObservedOutputKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::PlannerArtifact => "planner_artifact",
            Self::DeliveryToken => "delivery_token",
            Self::Structured => "structured",
            Self::Content => "content",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedContentStatus {
    NoContent,
    MentionedOnly,
    ContentAvailable,
    Failed,
}

impl ObservedContentStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoContent => "no_content",
            Self::MentionedOnly => "mentioned_only",
            Self::ContentAvailable => "content_available",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileTargetKind {
    LogFile,
    JsonFile,
    DbFile,
    ArchiveFile,
    Directory,
    File,
}

impl FileTargetKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LogFile => "log_file",
            Self::JsonFile => "json_file",
            Self::DbFile => "db_file",
            Self::ArchiveFile => "archive_file",
            Self::Directory => "directory",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryTokenKind {
    File,
    ImageFile,
    ImageUrl,
    VideoUrl,
    FileUrl,
    MediaUrl,
}

impl DeliveryTokenKind {
    pub(crate) fn prefix(self) -> &'static str {
        match self {
            Self::File => "FILE:",
            Self::ImageFile => "IMAGE_FILE:",
            Self::ImageUrl => "IMAGE_URL:",
            Self::VideoUrl => "VIDEO_URL:",
            Self::FileUrl => "FILE_URL:",
            Self::MediaUrl => "MEDIA_URL:",
        }
    }

    pub(crate) fn canonical_prefix(self) -> &'static str {
        match self {
            Self::File | Self::ImageFile => "FILE:",
            Self::ImageUrl => "IMAGE_URL:",
            Self::VideoUrl => "VIDEO_URL:",
            Self::FileUrl => "FILE_URL:",
            Self::MediaUrl => "MEDIA_URL:",
        }
    }

    pub(crate) fn is_file_path(self) -> bool {
        matches!(self, Self::File | Self::ImageFile)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlannerArtifactKind {
    ToolCallTag,
    LegacyToolTrace,
    ActionObject,
    PlannerObject,
}

pub(crate) fn parse_delivery_token(text: &str) -> Option<(DeliveryTokenKind, &str)> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("FILE:") {
        Some((DeliveryTokenKind::File, rest))
    } else if let Some(rest) = trimmed.strip_prefix("IMAGE_FILE:") {
        Some((DeliveryTokenKind::ImageFile, rest))
    } else if let Some(rest) = trimmed.strip_prefix("IMAGE_URL:") {
        Some((DeliveryTokenKind::ImageUrl, rest))
    } else if let Some(rest) = trimmed.strip_prefix("VIDEO_URL:") {
        Some((DeliveryTokenKind::VideoUrl, rest))
    } else if let Some(rest) = trimmed.strip_prefix("FILE_URL:") {
        Some((DeliveryTokenKind::FileUrl, rest))
    } else if let Some(rest) = trimmed.strip_prefix("MEDIA_URL:") {
        Some((DeliveryTokenKind::MediaUrl, rest))
    } else {
        None
    }
}

pub(crate) fn parse_delivery_file_token(text: &str) -> Option<(DeliveryTokenKind, &str)> {
    parse_delivery_token(text).filter(|(kind, _)| kind.is_file_path())
}

pub(crate) fn classify_planner_artifact(text: &str) -> Option<PlannerArtifactKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("[TOOL_CALL]") || trimmed.contains("[/TOOL_CALL]") {
        return Some(PlannerArtifactKind::ToolCallTag);
    }
    if trimmed.contains("{tool =>") && trimmed.contains("args =>") {
        return Some(PlannerArtifactKind::LegacyToolTrace);
    }
    if crate::prompt_utils::extract_agent_action_objects(trimmed)
        .into_iter()
        .next()
        .is_some()
    {
        return Some(PlannerArtifactKind::ActionObject);
    }
    let value = crate::parse_llm_json_raw_or_any::<Value>(trimmed)?;
    match value {
        Value::Object(map) => (map.contains_key("type")
            || map.contains_key("tool")
            || map.contains_key("skill")
            || map.contains_key("action")
            || map.get("steps").and_then(|v| v.as_array()).is_some())
        .then_some(PlannerArtifactKind::PlannerObject),
        _ => None,
    }
}

pub(crate) fn classify_observed_output_kind(text: &str) -> ObservedOutputKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        ObservedOutputKind::Empty
    } else if looks_like_planner_artifact(trimmed) {
        ObservedOutputKind::PlannerArtifact
    } else if parse_delivery_file_token(trimmed).is_some() {
        ObservedOutputKind::DeliveryToken
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        ObservedOutputKind::Structured
    } else {
        ObservedOutputKind::Content
    }
}

pub(crate) fn classify_observed_content_status(text: &str) -> ObservedContentStatus {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        ObservedContentStatus::NoContent
    } else if looks_like_planner_artifact(trimmed) || parse_delivery_file_token(trimmed).is_some() {
        ObservedContentStatus::MentionedOnly
    } else {
        ObservedContentStatus::ContentAvailable
    }
}

pub(crate) fn looks_like_planner_artifact(text: &str) -> bool {
    classify_planner_artifact(text).is_some()
}

pub(crate) fn looks_like_tool_call_artifact(text: &str) -> bool {
    matches!(
        classify_planner_artifact(text),
        Some(PlannerArtifactKind::ToolCallTag | PlannerArtifactKind::LegacyToolTrace)
    )
}

pub(crate) fn infer_file_target_kind(path: &str) -> FileTargetKind {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".log") {
        FileTargetKind::LogFile
    } else if lower.ends_with(".json") {
        FileTargetKind::JsonFile
    } else if lower.ends_with(".sqlite") || lower.ends_with(".db") {
        FileTargetKind::DbFile
    } else if lower.ends_with(".zip") || lower.ends_with(".tgz") || lower.ends_with(".tar.gz") {
        FileTargetKind::ArchiveFile
    } else if Path::new(path)
        .file_name()
        .and_then(|v| v.to_str())
        .map(|name| !name.contains('.'))
        .unwrap_or(false)
    {
        FileTargetKind::Directory
    } else {
        FileTargetKind::File
    }
}

pub(crate) fn build_final_delivery_with_priority(
    delivery_messages: &[String],
    last_user_visible_respond: Option<&String>,
) -> (Vec<String>, String, bool) {
    let mut delivery_deduped: Vec<String> = Vec::new();
    for m in delivery_messages {
        let t = normalize_user_visible_text(m).trim();
        if t.is_empty() {
            continue;
        }
        if let Some(pos) = delivery_deduped.iter().position(|x| x.trim() == t) {
            delivery_deduped.remove(pos);
        }
        delivery_deduped.push(t.to_string());
    }
    let used_last_respond = if let Some(last_respond) = last_user_visible_respond {
        let trimmed = normalize_user_visible_text(last_respond).trim();
        if !trimmed.is_empty() {
            delivery_deduped.retain(|x| x.trim() != trimmed);
            delivery_deduped.push(trimmed.to_string());
            true
        } else {
            false
        }
    } else {
        false
    };
    let final_text = delivery_deduped.last().cloned().unwrap_or_default();
    (delivery_deduped, final_text, used_last_respond)
}

pub(crate) fn normalize_user_visible_text(raw: &str) -> &str {
    let trimmed = raw.trim();
    if !trimmed.starts_with("subtask#") {
        return trimmed;
    }
    if let Some((_, body)) = trimmed.split_once('\n') {
        let body = body.trim();
        if !body.is_empty() {
            return body;
        }
    }
    if let Some((_, body)) = trimmed.split_once(": success") {
        let body = body.trim();
        if !body.is_empty() {
            return body;
        }
    }
    if let Some((_, body)) = trimmed.split_once(": failed") {
        let body = body.trim();
        if !body.is_empty() {
            return body;
        }
    }
    ""
}

#[cfg(test)]
fn parse_finalizer_schema_out(raw: &str) -> Option<FinalizerSchemaOut> {
    crate::parse_llm_json_extract_or_any::<FinalizerSchemaOut>(raw)
        .or_else(|| crate::parse_llm_json_raw_or_any::<FinalizerSchemaOut>(raw))
}

#[cfg(test)]
pub(crate) fn finalizer_contract_ok(schema: &FinalizerSchemaOut) -> bool {
    matches!(
        finalizer_contract_disposition(schema),
        FinalizerDisposition::QualifiedCompletion
    )
}

#[cfg(test)]
pub(crate) fn finalizer_contract_disposition(schema: &FinalizerSchemaOut) -> FinalizerDisposition {
    if schema.needs_clarify || !schema.completion_ok || !schema.grounded_ok {
        return FinalizerDisposition::MustFail;
    }
    if schema.format_ok
        && !schema.answer.trim().is_empty()
        && !looks_like_internal_trace_artifact(schema.answer.trim())
    {
        return FinalizerDisposition::QualifiedCompletion;
    }
    FinalizerDisposition::AllowFallback
}

#[cfg(test)]
pub(crate) fn finalizer_schema_answer(raw: &str) -> Option<(String, FinalizerSchemaOut)> {
    let schema = parse_finalizer_schema_out(raw)?;
    let answer = schema.answer.trim().to_string();
    if answer.is_empty() {
        return None;
    }
    Some((answer, schema))
}

pub(crate) fn looks_like_internal_trace_artifact(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("subtask#") || trimmed.starts_with("round=") || trimmed.starts_with("step=")
}

#[cfg(test)]
pub(crate) fn looks_like_structured_blob(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

#[cfg(test)]
fn trim_request_path_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';' | '。' | ')' | '(' | '）' | '（'
            )
        })
        .to_string()
}

#[cfg(test)]
fn extract_explicit_paths_from_request(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for token in input.split_whitespace() {
        let trimmed = trim_request_path_token(token);
        if !(trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("../")) {
            continue;
        }
        if seen.insert(trimmed.clone()) {
            out.push(trimmed);
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn extract_single_explicit_path_from_request(input: &str) -> Option<String> {
    let paths = extract_explicit_paths_from_request(input);
    if paths.len() == 1 {
        paths.into_iter().next()
    } else {
        None
    }
}

#[cfg(test)]
fn normalize_path_for_scope_compare(workspace_root: &Path, raw: &str) -> Option<String> {
    let trimmed = trim_request_path_token(raw);
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized =
        crate::ensure_default_file_path(workspace_root, &trimmed).replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    Some(normalized)
}

#[cfg(test)]
fn paths_equivalent_for_scope(workspace_root: &Path, expected: &str, actual: &str) -> bool {
    let Some(left) = normalize_path_for_scope_compare(workspace_root, expected) else {
        return false;
    };
    let Some(right) = normalize_path_for_scope_compare(workspace_root, actual) else {
        return false;
    };
    left == right
}

#[cfg(test)]
pub(crate) fn observed_quotes_grounded(schema: &FinalizerSchemaOut, observed: &str) -> bool {
    let mut any = false;
    for quote in schema
        .evidence_quotes
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        any = true;
        if !observed.contains(quote) {
            return false;
        }
    }
    any
}

#[cfg(test)]
pub(crate) fn observed_read_path_matches_request(
    workspace_root: &Path,
    user_text: &str,
    observed_read_path: Option<&str>,
) -> bool {
    let Some(expected_path) = extract_single_explicit_path_from_request(user_text) else {
        return true;
    };
    let Some(actual_path) = observed_read_path else {
        return true;
    };
    paths_equivalent_for_scope(workspace_root, &expected_path, actual_path)
}
