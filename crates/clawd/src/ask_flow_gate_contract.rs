use super::*;

pub(super) fn parse_direct_answer_gate_decision(raw: &str) -> DirectAnswerGateDecision {
    match raw.trim().to_ascii_lowercase().as_str() {
        "planner_execute" => DirectAnswerGateDecision::PlannerExecute,
        "clarify" => DirectAnswerGateDecision::Clarify,
        _ => DirectAnswerGateDecision::DirectAnswer,
    }
}

pub(super) fn parse_gate_response_shape(raw: &str) -> crate::OutputResponseShape {
    match raw.trim().to_ascii_lowercase().as_str() {
        "one_sentence" => crate::OutputResponseShape::OneSentence,
        "strict" => crate::OutputResponseShape::Strict,
        "scalar" => crate::OutputResponseShape::Scalar,
        "file_token" => crate::OutputResponseShape::FileToken,
        _ => crate::OutputResponseShape::Free,
    }
}

pub(super) fn parse_gate_locator_kind(raw: &str) -> crate::OutputLocatorKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "path" => crate::OutputLocatorKind::Path,
        "current_workspace" => crate::OutputLocatorKind::CurrentWorkspace,
        "url" => crate::OutputLocatorKind::Url,
        "filename" => crate::OutputLocatorKind::Filename,
        _ => crate::OutputLocatorKind::None,
    }
}

pub(super) fn parse_gate_delivery_intent(raw: &str) -> crate::OutputDeliveryIntent {
    match raw.trim().to_ascii_lowercase().as_str() {
        "file_single" => crate::OutputDeliveryIntent::FileSingle,
        "directory_lookup" => crate::OutputDeliveryIntent::DirectoryLookup,
        "directory_batch_files" => crate::OutputDeliveryIntent::DirectoryBatchFiles,
        _ => crate::OutputDeliveryIntent::None,
    }
}

pub(super) fn parse_gate_semantic_kind(raw: &str) -> crate::OutputSemanticKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "raw_command_output"
        | "raw_output"
        | "command_output"
        | "command_result"
        | "command_execution_result" => crate::OutputSemanticKind::RawCommandOutput,
        "command_output_summary" | "command_result_summary" | "command_output_synthesis" => {
            crate::OutputSemanticKind::CommandOutputSummary
        }
        "service_status" => crate::OutputSemanticKind::ServiceStatus,
        "hidden_entries_check" => crate::OutputSemanticKind::HiddenEntriesCheck,
        "file_names" => crate::OutputSemanticKind::FileNames,
        "directory_names" => crate::OutputSemanticKind::DirectoryNames,
        "directory_entry_groups" => crate::OutputSemanticKind::DirectoryEntryGroups,
        "file_paths" => crate::OutputSemanticKind::FilePaths,
        "directory_purpose_summary" => crate::OutputSemanticKind::DirectoryPurposeSummary,
        "content_excerpt_summary" => crate::OutputSemanticKind::ContentExcerptSummary,
        "content_excerpt_with_summary" => crate::OutputSemanticKind::ContentExcerptWithSummary,
        "content_presence_check" => crate::OutputSemanticKind::ContentPresenceCheck,
        "excerpt_kind_judgment" => crate::OutputSemanticKind::ExcerptKindJudgment,
        "recent_artifacts_judgment" => crate::OutputSemanticKind::RecentArtifactsJudgment,
        "workspace_project_summary" => crate::OutputSemanticKind::WorkspaceProjectSummary,
        "scalar_count" => crate::OutputSemanticKind::ScalarCount,
        "quantity_comparison" => crate::OutputSemanticKind::QuantityComparison,
        "execution_failed_step" => crate::OutputSemanticKind::ExecutionFailedStep,
        "generated_file_delivery" => crate::OutputSemanticKind::GeneratedFileDelivery,
        "generated_file_path_report" => crate::OutputSemanticKind::GeneratedFilePathReport,
        "filesystem_mutation_result" => crate::OutputSemanticKind::FilesystemMutationResult,
        "scalar_path_only" => crate::OutputSemanticKind::ScalarPathOnly,
        "file_basename" => crate::OutputSemanticKind::FileBasename,
        "existence_with_path" => crate::OutputSemanticKind::ExistenceWithPath,
        "existence_with_path_summary" => crate::OutputSemanticKind::ExistenceWithPathSummary,
        "recent_scalar_equality_check" => crate::OutputSemanticKind::RecentScalarEqualityCheck,
        "git_commit_subject" => crate::OutputSemanticKind::GitCommitSubject,
        "git_repository_state"
        | "git_workspace_state"
        | "git_state"
        | "git_status"
        | "git_branch"
        | "git_current_branch"
        | "git_remote"
        | "git_changed_files" => crate::OutputSemanticKind::GitRepositoryState,
        "structured_keys" => crate::OutputSemanticKind::StructuredKeys,
        "config_validation" | "structured_config_validation" => {
            crate::OutputSemanticKind::ConfigValidation
        }
        "config_mutation" | "config_write" | "config_set" | "structured_config_mutation" => {
            crate::OutputSemanticKind::ConfigMutation
        }
        "config_risk_assessment" | "config_risk" | "structured_config_risk" | "config_guard" => {
            crate::OutputSemanticKind::ConfigRiskAssessment
        }
        "sqlite_table_listing" => crate::OutputSemanticKind::SqliteTableListing,
        "sqlite_table_names_only" => crate::OutputSemanticKind::SqliteTableNamesOnly,
        "sqlite_database_kind_judgment" => crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
        "sqlite_schema_version" => crate::OutputSemanticKind::SqliteSchemaVersion,
        "rss_news_fetch" | "rss_latest_news" | "rss_feed_fetch" | "external_news_fetch" => {
            crate::OutputSemanticKind::RssNewsFetch
        }
        "web_page_summary"
        | "webpage_summary"
        | "web_content_summary"
        | "url_content_summary"
        | "browser_page_summary" => crate::OutputSemanticKind::WebPageSummary,
        "web_search_summary" | "web_search_results" | "search_results_summary" => {
            crate::OutputSemanticKind::WebSearchSummary
        }
        "weather_query" | "weather_current" | "weather_forecast" | "weather_report" => {
            crate::OutputSemanticKind::WeatherQuery
        }
        "market_quote" | "stock_quote" | "crypto_quote" | "asset_quote" | "market_price" => {
            crate::OutputSemanticKind::MarketQuote
        }
        "image_understanding"
        | "image_description"
        | "image_describe"
        | "image_vision"
        | "image_extract"
        | "image_compare"
        | "screenshot_summary" => crate::OutputSemanticKind::ImageUnderstanding,
        "publishing_preview" | "social_post_preview" | "channel_draft_preview" => {
            crate::OutputSemanticKind::PublishingPreview
        }
        "package_manager_detection" | "package_manager_detect" | "package_detect_manager" => {
            crate::OutputSemanticKind::PackageManagerDetection
        }
        "archive_list" => crate::OutputSemanticKind::ArchiveList,
        "archive_read" => crate::OutputSemanticKind::ArchiveRead,
        "archive_pack" => crate::OutputSemanticKind::ArchivePack,
        "archive_unpack" => crate::OutputSemanticKind::ArchiveUnpack,
        "docker_ps" => crate::OutputSemanticKind::DockerPs,
        "docker_images" => crate::OutputSemanticKind::DockerImages,
        "docker_logs" => crate::OutputSemanticKind::DockerLogs,
        "docker_container_lifecycle" => crate::OutputSemanticKind::DockerContainerLifecycle,
        _ => crate::OutputSemanticKind::None,
    }
}

pub(super) fn parse_gate_self_extension(
    raw: DirectAnswerGateSelfExtensionOut,
) -> crate::SelfExtensionContract {
    let mode = match raw.mode.trim().to_ascii_lowercase().as_str() {
        "temporary_fix" => crate::SelfExtensionMode::TemporaryFix,
        "permanent_extension" => crate::SelfExtensionMode::PermanentExtension,
        _ => crate::SelfExtensionMode::None,
    };
    let trigger = match raw.trigger.trim().to_ascii_lowercase().as_str() {
        "explicit_user_request" => crate::SelfExtensionTrigger::ExplicitUserRequest,
        "capability_gap" => crate::SelfExtensionTrigger::CapabilityGap,
        _ => crate::SelfExtensionTrigger::None,
    };
    crate::SelfExtensionContract {
        mode,
        trigger,
        execute_now: raw.execute_now,
        scalar_count_filter: Default::default(),
        list_selector: Default::default(),
        structured_field_selector: None,
    }
}

pub(super) fn output_contract_from_direct_answer_gate(
    raw: DirectAnswerGateContractOut,
    fallback: &crate::IntentOutputContract,
) -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        response_shape: parse_gate_response_shape(&raw.response_shape),
        exact_sentence_count: raw.exact_sentence_count,
        requires_content_evidence: raw.requires_content_evidence,
        delivery_required: raw.delivery_required,
        locator_kind: parse_gate_locator_kind(&raw.locator_kind),
        delivery_intent: parse_gate_delivery_intent(&raw.delivery_intent),
        semantic_kind: parse_gate_semantic_kind(&raw.semantic_kind),
        locator_hint: raw.locator_hint.trim().to_string(),
        self_extension: parse_gate_self_extension(raw.self_extension),
    }
    .with_fallback_shape(fallback)
}

pub(super) fn ordered_entry_looks_like_workspace_artifact(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    let path = Path::new(trimmed);
    let has_filename_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.chars().any(|ch| ch.is_ascii_alphabetic()));
    has_filename_extension
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('.')
}

pub(super) fn direct_answer_candidate_looks_like_artifact_listing(resolved_prompt: &str) -> bool {
    let Some(candidate) = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt) else {
        return false;
    };
    let entries = crate::followup_frame::extract_ordered_entries_from_text(&candidate);
    entries.len() >= 2
        && entries
            .iter()
            .all(|entry| ordered_entry_looks_like_workspace_artifact(entry))
}

pub(super) fn trim_artifact_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    })
}

pub(super) fn text_mentions_artifact_locator(text: &str) -> bool {
    crate::delivery_utils::extract_filename_candidates(text)
        .iter()
        .any(|candidate| ordered_entry_looks_like_workspace_artifact(candidate))
        || text
            .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '，' | ';' | '；'))
            .map(trim_artifact_token)
            .any(ordered_entry_looks_like_workspace_artifact)
}

pub(super) fn resolve_existing_recent_file_token(state: &AppState, token: &str) -> Option<String> {
    let token = trim_artifact_token(token);
    if token.is_empty() {
        return None;
    }
    let raw_path = Path::new(token);
    let mut candidates = Vec::new();
    if raw_path.is_absolute() {
        candidates.push(raw_path.to_path_buf());
    } else {
        candidates.push(state.skill_rt.workspace_root.join(raw_path));
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(raw_path));
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Some(
                candidate
                    .canonicalize()
                    .unwrap_or(candidate)
                    .display()
                    .to_string(),
            );
        }
    }
    None
}

pub(super) fn collect_recent_execution_request_file_targets(
    state: &AppState,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Vec<String> {
    let Some(context) = agent_run_context
        .and_then(|ctx| ctx.cross_turn_recent_execution_context.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "<none>")
    else {
        return Vec::new();
    };
    let section = context
        .split_once("### RECENT_EXECUTION_EVENTS")
        .map(|(_, tail)| tail)
        .unwrap_or(context);
    let mut targets = Vec::new();
    for line in section.lines() {
        let Some((_, request_tail)) = line.split_once(" request=") else {
            continue;
        };
        let request = request_tail
            .split(" result=")
            .next()
            .unwrap_or(request_tail)
            .trim();
        for token in request.split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\''
                        | '`'
                        | ','
                        | '，'
                        | '。'
                        | ';'
                        | '；'
                        | '('
                        | ')'
                        | '（'
                        | '）'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '《'
                        | '》'
                )
        }) {
            let Some(path) = resolve_existing_recent_file_token(state, token) else {
                continue;
            };
            if !targets.iter().any(|existing| existing == &path) {
                targets.push(path);
            }
        }
    }
    targets
}

pub(super) fn direct_answer_gate_should_force_recent_file_context_execution(
    current_user_request: &str,
    resolved_prompt: &str,
    contract: &crate::IntentOutputContract,
    recent_request_file_target_count: usize,
) -> bool {
    if output_contract_requires_planner_execution(contract) {
        return false;
    }
    if recent_request_file_target_count < 2 {
        return false;
    }
    let Some(candidate) = normalizer_answer_candidate_from_resolved_prompt(resolved_prompt) else {
        return false;
    };
    if !text_mentions_artifact_locator(&candidate) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_concrete_locator_hint()
        || surface.has_structured_target_refinement()
        || surface.has_filename_candidates()
    {
        return false;
    }
    true
}

pub(super) fn promote_artifact_listing_candidate_contract(
    resolved_prompt: &str,
    contract: &mut crate::IntentOutputContract,
) -> bool {
    if output_contract_requires_planner_execution(contract)
        || !direct_answer_candidate_looks_like_artifact_listing(resolved_prompt)
    {
        return false;
    }
    contract.requires_content_evidence = true;
    contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    if matches!(
        contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        contract.response_shape = crate::OutputResponseShape::Strict;
    }
    true
}
