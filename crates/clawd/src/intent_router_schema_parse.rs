use super::{
    normalize_schema_token, normalize_structured_field_selector,
    scalar_count_filter::{parse_scalar_count_filter, parse_scalar_count_target_kind},
    turn_analysis::{TargetTaskPolicy, TurnType},
    ExecutionRecipePlanHint,
};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputListSelector, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, ResumeBehavior, ScheduleKind, SelfExtensionContract,
    SelfExtensionMode, SelfExtensionTrigger,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct IntentOutputContractOut {
    #[serde(default)]
    pub(super) response_shape: String,
    #[serde(default)]
    pub(super) exact_sentence_count: Option<Value>,
    #[serde(default)]
    pub(super) requires_content_evidence: bool,
    #[serde(default)]
    pub(super) delivery_required: bool,
    #[serde(default)]
    pub(super) locator_kind: String,
    #[serde(default)]
    pub(super) delivery_intent: String,
    #[serde(default)]
    pub(super) contract_marker: String,
    #[serde(default)]
    pub(super) locator_hint: String,
    #[serde(default)]
    pub(super) scalar_count_filter: Option<Value>,
    #[serde(default)]
    pub(super) list_selector: Option<Value>,
    #[serde(default)]
    pub(super) self_extension: Option<SelfExtensionContractOut>,
}

fn output_contract_marker_token(raw: &IntentOutputContractOut) -> &str {
    raw.contract_marker.trim()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct ListSelectorOut {
    #[serde(default)]
    pub(super) target_kind: String,
    #[serde(default)]
    pub(super) limit: Option<Value>,
    #[serde(default)]
    pub(super) sort_by: String,
    #[serde(default)]
    pub(super) include_metadata: Option<bool>,
    #[serde(default)]
    pub(super) include_hidden: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct SelfExtensionContractOut {
    #[serde(default)]
    pub(super) mode: String,
    #[serde(default)]
    pub(super) trigger: String,
    #[serde(default)]
    pub(super) execute_now: bool,
    #[serde(default)]
    pub(super) structured_field_selector: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct IntentExecutionRecipeOut {
    #[serde(default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) profile: String,
    #[serde(default)]
    pub(super) target_scope: String,
    #[serde(default)]
    pub(super) command: String,
    #[serde(default)]
    pub(super) cmd: String,
    #[serde(default)]
    pub(super) shell_command: String,
    #[serde(default)]
    pub(super) execution_mode: String,
    #[serde(default)]
    pub(super) async_adapter_kind: String,
}

pub(super) fn parse_execution_recipe_plan_hint(
    out: Option<&IntentExecutionRecipeOut>,
) -> Option<ExecutionRecipePlanHint> {
    let raw = out?;
    let command = [&raw.command, &raw.cmd, &raw.shell_command]
        .into_iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string);
    let kind = raw.kind.trim().to_string();
    let execution_mode =
        (!raw.execution_mode.trim().is_empty()).then(|| raw.execution_mode.trim().to_string());
    let async_adapter_kind = (!raw.async_adapter_kind.trim().is_empty())
        .then(|| raw.async_adapter_kind.trim().to_string());
    if kind.is_empty()
        && command.is_none()
        && execution_mode.is_none()
        && async_adapter_kind.is_none()
    {
        return None;
    }
    Some(ExecutionRecipePlanHint {
        kind,
        command,
        execution_mode,
        async_adapter_kind,
    })
}

pub(super) fn parse_runtime_async_job_start_plan_hint(
    state_patch: Option<&Value>,
) -> Option<ExecutionRecipePlanHint> {
    let patch = state_patch?;
    let start = patch.get("runtime_async_job_start")?.as_object()?;
    let field_text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| start.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let command = field_text(&["command", "cmd", "shell_command", "command_sequence"])?;
    let kind = field_text(&["kind"]).unwrap_or_else(|| "runtime_async_job_start".to_string());
    let execution_mode = field_text(&["execution_mode", "mode"]);
    let async_adapter_kind = field_text(&["async_adapter_kind", "adapter_kind"]);

    Some(ExecutionRecipePlanHint {
        kind,
        command: Some(command),
        execution_mode,
        async_adapter_kind,
    })
}

pub(super) fn parse_resume_behavior(s: &str) -> ResumeBehavior {
    match s.trim().to_ascii_lowercase().as_str() {
        "resume_execute" | "resume" => ResumeBehavior::ResumeExecute,
        "resume_discuss" | "defer" => ResumeBehavior::ResumeDiscuss,
        _ => ResumeBehavior::None,
    }
}

pub(super) fn parse_turn_type(s: &str) -> Option<TurnType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "task_request" => Some(TurnType::TaskRequest),
        "task_append" => Some(TurnType::TaskAppend),
        "task_replace" => Some(TurnType::TaskReplace),
        "task_correct" => Some(TurnType::TaskCorrect),
        "task_scope_update" => Some(TurnType::TaskScopeUpdate),
        "run_control" => Some(TurnType::RunControl),
        "approval_decision" => Some(TurnType::ApprovalDecision),
        "status_query" | "runtime_status_query" => Some(TurnType::StatusQuery),
        "feedback_or_error" => Some(TurnType::FeedbackOrError),
        "preference_or_memory" => Some(TurnType::PreferenceOrMemory),
        _ => None,
    }
}

pub(super) fn parse_target_task_policy(s: &str) -> Option<TargetTaskPolicy> {
    match s.trim().to_ascii_lowercase().as_str() {
        "reuse_active" => Some(TargetTaskPolicy::ReuseActive),
        "replace_active" => Some(TargetTaskPolicy::ReplaceActive),
        "pause_and_queue" => Some(TargetTaskPolicy::PauseAndQueue),
        "standalone" => Some(TargetTaskPolicy::Standalone),
        _ => None,
    }
}

pub(super) fn infer_missing_turn_type_from_policy(
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    needs_clarify: bool,
    schedule_kind: ScheduleKind,
    should_refresh_long_term_memory: bool,
) -> Option<TurnType> {
    if turn_type.is_some()
        || needs_clarify
        || should_refresh_long_term_memory
        || !matches!(schedule_kind, ScheduleKind::None)
    {
        return turn_type;
    }
    match target_task_policy {
        Some(TargetTaskPolicy::Standalone) => Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::ReuseActive) => Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReplaceActive) => Some(TurnType::TaskReplace),
        Some(TargetTaskPolicy::PauseAndQueue) | None => None,
    }
}

pub(super) fn parse_schedule_kind(s: &str) -> ScheduleKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "create" => ScheduleKind::Create,
        "update" | "pause" | "resume" => ScheduleKind::Update,
        "delete" => ScheduleKind::Delete,
        "query" | "list" => ScheduleKind::Query,
        _ => ScheduleKind::None,
    }
}

pub(super) fn parse_output_response_shape(s: &str) -> OutputResponseShape {
    match s.trim().to_ascii_lowercase().as_str() {
        "one_sentence" => OutputResponseShape::OneSentence,
        "strict"
        | "exact"
        | "exact_text"
        | "strict_text"
        | "list"
        | "array"
        | "list_only"
        | "names_list"
        | "exact_format"
        | "one_line"
        | "single_line"
        | "line_only"
        | "one_line_string"
        | "single_line_string"
        | "one_line_text"
        | "single_line_text"
        | "one_line_result"
        | "single_line_result"
        | "one_line_comparison"
        | "single_line_comparison" => OutputResponseShape::Strict,
        "scalar" => OutputResponseShape::Scalar,
        "file_token" => OutputResponseShape::FileToken,
        _ => OutputResponseShape::Free,
    }
}

pub(super) fn parse_output_locator_kind(s: &str) -> OutputLocatorKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "path" => OutputLocatorKind::Path,
        "current_workspace" => OutputLocatorKind::CurrentWorkspace,
        "url" => OutputLocatorKind::Url,
        "filename" => OutputLocatorKind::Filename,
        _ => OutputLocatorKind::None,
    }
}

pub(super) fn parse_output_delivery_intent(s: &str) -> OutputDeliveryIntent {
    match s.trim().to_ascii_lowercase().as_str() {
        "file_single" | "single_file" | "file" => OutputDeliveryIntent::FileSingle,
        "directory_lookup" | "dir_lookup" => OutputDeliveryIntent::DirectoryLookup,
        "directory_batch_files" | "batch_directory_delivery" | "dir_batch" => {
            OutputDeliveryIntent::DirectoryBatchFiles
        }
        _ => OutputDeliveryIntent::None,
    }
}

pub(super) fn semantic_kind_token_requests_scalar_response_shape(s: &str) -> bool {
    matches!(
        normalize_schema_token(s).as_str(),
        "scalar"
            | "scalar_value"
            | "scalar_only"
            | "value"
            | "value_only"
            | "single_value"
            | "field_value"
            | "file_field_value"
    )
}

fn parse_output_semantic_kind_token(s: &str) -> OutputSemanticKind {
    match s.trim().to_ascii_lowercase().as_str() {
        "raw_command_output"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "combined_command_output"
        | "command_execution_result" => OutputSemanticKind::RawCommandOutput,
        "command_output_summary"
        | "command_result_summary"
        | "command_output_synthesis"
        | "command_result_synthesis" => OutputSemanticKind::CommandOutputSummary,
        "service_status"
        | "service_state"
        | "service_running_status"
        | "process_status"
        | "process_state"
        | "process_running_status"
        | "daemon_status"
        | "daemon_state" => OutputSemanticKind::ServiceStatus,
        "hidden_entries_check"
        | "hidden_entry_check"
        | "hidden_files_check"
        | "hidden_file_check"
        | "hidden_files_example"
        | "hidden_entries_example"
        | "hidden_entries"
        | "hidden_files" => OutputSemanticKind::HiddenEntriesCheck,
        "file_names" | "file_names_only" | "file_name_only" | "filename_only"
        | "filenames_only" | "names_only" => OutputSemanticKind::FileNames,
        "directory_names"
        | "directory_names_only"
        | "directory_name_only"
        | "dir_names"
        | "dir_names_only"
        | "folder_names"
        | "folder_names_only"
        | "folders_only" => OutputSemanticKind::DirectoryNames,
        "directory_entry_groups"
        | "directory_file_groups"
        | "file_directory_groups"
        | "entry_kind_groups"
        | "entry_names"
        | "directory_entry_names"
        | "entries_by_kind"
        | "grouped_entries"
        | "grouped_entry_names" => OutputSemanticKind::DirectoryEntryGroups,
        "file_paths"
        | "file_paths_only"
        | "path_list"
        | "paths_list"
        | "file_path_list"
        | "repository_file_paths"
        | "workspace_file_paths" => OutputSemanticKind::FilePaths,
        "directory_purpose_summary" | "listing_purpose_summary" | "directory_listing_summary" => {
            OutputSemanticKind::DirectoryPurposeSummary
        }
        "content_excerpt_summary" | "document_excerpt_summary" | "file_excerpt_summary" => {
            OutputSemanticKind::ContentExcerptSummary
        }
        "document_heading" | "document_title" | "markdown_heading" | "markdown_title"
        | "file_heading" | "file_title" => OutputSemanticKind::DocumentHeading,
        "content_excerpt_with_summary"
        | "excerpt_with_summary"
        | "raw_excerpt_with_summary"
        | "bounded_excerpt_with_summary" => OutputSemanticKind::ContentExcerptWithSummary,
        "content_presence_check"
        | "content_contains_check"
        | "content_match_check"
        | "identifier_presence_check"
        | "field_presence_check"
        | "text_presence_check" => OutputSemanticKind::ContentPresenceCheck,
        "excerpt_kind_judgment" | "content_excerpt_judgment" | "log_vs_checklist" => {
            OutputSemanticKind::ExcerptKindJudgment
        }
        "recent_artifacts_judgment" | "artifact_style_classification" => {
            OutputSemanticKind::RecentArtifactsJudgment
        }
        "workspace_project_summary" | "project_overview" | "workspace_overview_summary" => {
            OutputSemanticKind::WorkspaceProjectSummary
        }
        "scalar" => OutputSemanticKind::None,
        "scalar_count" | "count" => OutputSemanticKind::ScalarCount,
        "quantity_comparison" | "comparison" => OutputSemanticKind::QuantityComparison,
        "execution_failed_step"
        | "failed_step"
        | "failed_command_step"
        | "execution_failure_step" => OutputSemanticKind::ExecutionFailedStep,
        "generated_file_delivery"
        | "new_file_delivery"
        | "created_file_delivery"
        | "write_then_send_file" => OutputSemanticKind::GeneratedFileDelivery,
        "generated_file_path_report"
        | "new_file_path_report"
        | "created_file_path_report"
        | "write_then_report_path"
        | "saved_file_path_report" => OutputSemanticKind::GeneratedFilePathReport,
        "filesystem_mutation_result"
        | "filesystem_mutation"
        | "fs_mutation_result"
        | "file_mutation_result"
        | "path_mutation_result" => OutputSemanticKind::FilesystemMutationResult,
        "scalar_path_only" | "path_only" => OutputSemanticKind::ScalarPathOnly,
        "file_basename" | "single_file_basename" | "bound_file_basename" => {
            OutputSemanticKind::FileBasename
        }
        "existence_with_path" | "exists_with_path" => OutputSemanticKind::ExistenceWithPath,
        "existence_with_path_summary"
        | "exists_with_path_summary"
        | "existence_with_path_purpose"
        | "exists_with_path_purpose" => OutputSemanticKind::ExistenceWithPathSummary,
        "recent_scalar_equality_check"
        | "same_or_different"
        | "equality_check"
        | "scalar_equality"
        | "value_equality"
        | "value_comparison"
        | "field_equality"
        | "field_value_equality"
        | "key_value_comparison" => OutputSemanticKind::RecentScalarEqualityCheck,
        "git_commit_subject"
        | "git_commit_title"
        | "commit_subject"
        | "commit_title"
        | "latest_commit_subject"
        | "latest_commit_title" => OutputSemanticKind::GitCommitSubject,
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files"
        | "git_rev_parse" => OutputSemanticKind::GitRepositoryState,
        "structured_keys"
        | "structured_key_names"
        | "structured_top_level_keys"
        | "top_level_keys"
        | "object_keys"
        | "config_keys" => OutputSemanticKind::StructuredKeys,
        "config_validation" | "structured_config_validation" | "structured_file_validation" => {
            OutputSemanticKind::ConfigValidation
        }
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            OutputSemanticKind::ConfigMutation
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            OutputSemanticKind::ConfigRiskAssessment
        }
        "sqlite_table_listing" | "sqlite_tables_listing" | "sqlite_tables_summary" => {
            OutputSemanticKind::SqliteTableListing
        }
        "sqlite_table_names_only" | "sqlite_table_names" | "sqlite_names_only" => {
            OutputSemanticKind::SqliteTableNamesOnly
        }
        "sqlite_database_kind_judgment" | "sqlite_db_kind" | "database_kind_judgment" => {
            OutputSemanticKind::SqliteDatabaseKindJudgment
        }
        "sqlite_schema_version" | "sqlite_db_schema_version" => {
            OutputSemanticKind::SqliteSchemaVersion
        }
        "rss_news_fetch" | "rss_latest_news" | "rss_feed_fetch" | "external_news_fetch" => {
            OutputSemanticKind::RssNewsFetch
        }
        "web_page_summary"
        | "webpage_summary"
        | "web_content_summary"
        | "url_content_summary"
        | "browser_page_summary" => OutputSemanticKind::WebPageSummary,
        "web_search_summary" | "web_search_results" | "search_results_summary" => {
            OutputSemanticKind::WebSearchSummary
        }
        "weather_query" | "weather_current" | "weather_forecast" | "weather_report" => {
            OutputSemanticKind::WeatherQuery
        }
        "market_quote" | "stock_quote" | "crypto_quote" | "asset_quote" | "market_price" => {
            OutputSemanticKind::MarketQuote
        }
        "image_understanding"
        | "image_description"
        | "image_describe"
        | "image_vision"
        | "image_extract"
        | "image_compare"
        | "screenshot_summary" => OutputSemanticKind::ImageUnderstanding,
        "photo_organization"
        | "photo_organize"
        | "photo_organizing"
        | "photo_source_candidates"
        | "photo_discovery"
        | "photo_organization_preview" => OutputSemanticKind::PhotoOrganization,
        "publishing_preview" | "social_post_preview" | "channel_draft_preview" => {
            OutputSemanticKind::PublishingPreview
        }
        "package_manager_detection" | "package_manager_detect" | "package_detect_manager" => {
            OutputSemanticKind::PackageManagerDetection
        }
        "tool_discovery"
        | "capability_discovery"
        | "capability_inventory"
        | "skill_discovery"
        | "skill_inventory" => OutputSemanticKind::ToolDiscovery,
        "archive_list" | "archive_listing" | "archive_contents" => OutputSemanticKind::ArchiveList,
        "archive_read" | "archive_member_read" | "archive_file_read" => {
            OutputSemanticKind::ArchiveRead
        }
        "archive_pack" | "archive_create" | "archive_compress" => OutputSemanticKind::ArchivePack,
        "archive_unpack" | "archive_extract" | "archive_decompress" => {
            OutputSemanticKind::ArchiveUnpack
        }
        "docker_ps" | "docker_containers" | "docker_container_list" => OutputSemanticKind::DockerPs,
        "docker_images" | "docker_image_list" => OutputSemanticKind::DockerImages,
        "docker_logs" => OutputSemanticKind::DockerLogs,
        "docker_container_lifecycle" | "docker_lifecycle" => {
            OutputSemanticKind::DockerContainerLifecycle
        }
        _ => OutputSemanticKind::None,
    }
}

pub(super) fn parse_output_semantic_kind(s: &str) -> OutputSemanticKind {
    let mut parsed = OutputSemanticKind::None;
    let mut saw_separator = false;
    for token in s.split(['|', ',', ';']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        saw_separator = true;
        let candidate = parse_output_semantic_kind_token(token);
        if candidate != OutputSemanticKind::None {
            parsed = candidate;
        }
    }
    if saw_separator && parsed != OutputSemanticKind::None {
        parsed
    } else {
        parse_output_semantic_kind_token(s)
    }
}

fn parse_list_selector_sort_by(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "name" | "name_desc" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
    )
    .then_some(normalized)
}

fn parse_list_selector_limit(raw: Option<&Value>) -> Option<u64> {
    parse_positive_usize_value(raw?).map(|value| (value as u64).clamp(1, 1000))
}

pub(super) fn parse_list_selector(raw: Option<Value>) -> OutputListSelector {
    let Some(raw @ Value::Object(_)) = raw else {
        return OutputListSelector::default();
    };
    let target_kind_specified = raw.get("target_kind").is_some();
    let Ok(raw) = serde_json::from_value::<ListSelectorOut>(raw) else {
        return OutputListSelector::default();
    };
    OutputListSelector {
        target_kind: parse_scalar_count_target_kind(&raw.target_kind).unwrap_or_default(),
        target_kind_specified,
        limit: parse_list_selector_limit(raw.limit.as_ref()),
        sort_by: parse_list_selector_sort_by(&raw.sort_by),
        include_metadata: raw.include_metadata,
        include_hidden: raw.include_hidden,
    }
}

pub(super) fn parse_positive_usize_value(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().and_then(|n| usize::try_from(n).ok()),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
    .filter(|n| *n > 0)
}

pub(super) fn parse_output_contract(
    out: Option<IntentOutputContractOut>,
    wants_file_delivery: bool,
) -> IntentOutputContract {
    let mut contract = IntentOutputContract::default();
    if let Some(raw) = out {
        let contract_marker = output_contract_marker_token(&raw);
        let semantic_token_requests_scalar_shape =
            semantic_kind_token_requests_scalar_response_shape(contract_marker);
        contract.response_shape = parse_output_response_shape(&raw.response_shape);
        contract.exact_sentence_count = raw
            .exact_sentence_count
            .as_ref()
            .and_then(parse_positive_usize_value);
        contract.requires_content_evidence = raw.requires_content_evidence;
        contract.delivery_required = raw.delivery_required;
        contract.locator_kind = parse_output_locator_kind(&raw.locator_kind);
        contract.delivery_intent = parse_output_delivery_intent(&raw.delivery_intent);
        contract.semantic_kind = parse_output_semantic_kind(contract_marker);
        contract.locator_hint = raw.locator_hint.trim().to_string();
        contract.self_extension.scalar_count_filter =
            parse_scalar_count_filter(raw.scalar_count_filter);
        contract.self_extension.list_selector = parse_list_selector(raw.list_selector);
        if semantic_token_requests_scalar_shape
            && !matches!(contract.response_shape, OutputResponseShape::FileToken)
        {
            contract.response_shape = OutputResponseShape::Scalar;
            contract.semantic_kind = OutputSemanticKind::None;
        }
        if let Some(self_extension) = raw.self_extension {
            let structured_field_selector = normalize_structured_field_selector(
                self_extension.structured_field_selector.as_deref(),
            )
            .or_else(|| contract.self_extension.structured_field_selector.clone());
            contract.self_extension = SelfExtensionContract {
                mode: parse_self_extension_mode(&self_extension.mode),
                trigger: parse_self_extension_trigger(&self_extension.trigger),
                execute_now: self_extension.execute_now,
                scalar_count_filter: contract.self_extension.scalar_count_filter.clone(),
                list_selector: contract.self_extension.list_selector.clone(),
                structured_field_selector,
            };
        }
    }
    if contract.exact_sentence_count.is_some_and(|count| count > 1)
        && matches!(contract.response_shape, OutputResponseShape::OneSentence)
    {
        contract.response_shape = OutputResponseShape::Strict;
    }
    if wants_file_delivery {
        contract.delivery_required = true;
        if matches!(contract.response_shape, OutputResponseShape::Free) {
            contract.response_shape = OutputResponseShape::FileToken;
        }
        if matches!(contract.locator_kind, OutputLocatorKind::None) {
            contract.locator_kind = OutputLocatorKind::Path;
        }
        if matches!(contract.delivery_intent, OutputDeliveryIntent::None) {
            contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        }
    } else if contract.delivery_required
        && !matches!(contract.response_shape, OutputResponseShape::FileToken)
    {
        if matches!(contract.delivery_intent, OutputDeliveryIntent::None)
            || (matches!(contract.delivery_intent, OutputDeliveryIntent::FileSingle)
                && !contract.semantic_kind_is_unclassified())
        {
            contract.delivery_required = false;
            contract.delivery_intent = OutputDeliveryIntent::None;
        }
    }
    contract
}

pub(super) fn parse_self_extension_mode(s: &str) -> SelfExtensionMode {
    match s.trim().to_ascii_lowercase().as_str() {
        "temporary_fix" => SelfExtensionMode::TemporaryFix,
        "permanent_extension" => SelfExtensionMode::PermanentExtension,
        _ => SelfExtensionMode::None,
    }
}

pub(super) fn parse_self_extension_trigger(s: &str) -> SelfExtensionTrigger {
    match s.trim().to_ascii_lowercase().as_str() {
        "explicit_user_request" => SelfExtensionTrigger::ExplicitUserRequest,
        "capability_gap" => SelfExtensionTrigger::CapabilityGap,
        _ => SelfExtensionTrigger::None,
    }
}
