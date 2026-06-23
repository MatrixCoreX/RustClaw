use super::{
    ascii_token_present, execution_finalize_style_for_contract, ActFinalizeStyle,
    FirstLayerDecision, IntentExecutionRecipeOut, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputScalarCountTargetKind, OutputSemanticKind,
    ScheduleKind, SelfExtensionMode, SelfExtensionTrigger,
};

pub(super) fn downgrade_executionless_route_to_direct_answer(
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    _execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> Option<&'static str> {
    if needs_clarify
        || !matches!(
            legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
    {
        return None;
    }
    if !matches!(execution_finalize_style, ActFinalizeStyle::ChatWrapped) {
        return None;
    }
    if route_has_structured_execution_signal(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        None,
    ) {
        return None;
    }
    *legacy_normalizer_decision = FirstLayerDecision::DirectAnswer;
    *execution_finalize_style = ActFinalizeStyle::Plain;
    Some("executionless_route_downgraded_to_direct_answer")
}

pub(super) fn apply_explicit_command_execution_contract_repair(
    command_runtime: &crate::CommandIntentRuntime,
    current_user_request: &str,
    route_reason: &str,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    output_contract: &mut IntentOutputContract,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    let Some(explicit_command_segment) =
        crate::agent_engine::explicit_execution_command_segment_for_policy(
            command_runtime,
            current_user_request,
        )
    else {
        return None;
    };
    if matches!(
        *legacy_normalizer_decision,
        FirstLayerDecision::DirectAnswer
    ) && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.semantic_kind, OutputSemanticKind::None)
    {
        return None;
    }
    if output_contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery
        && output_contract.delivery_required
        && output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && output_contract.response_shape == OutputResponseShape::FileToken
    {
        *needs_clarify = false;
        clarify_question.clear();
        output_contract.requires_content_evidence = true;
        *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
        *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
        return Some("explicit_command_preserves_generated_file_delivery_execution");
    }
    if output_contract.semantic_kind == OutputSemanticKind::GeneratedFilePathReport
        && !output_contract.delivery_required
        && output_contract.delivery_intent == OutputDeliveryIntent::None
        && output_contract.response_shape == OutputResponseShape::Scalar
    {
        *needs_clarify = false;
        clarify_question.clear();
        output_contract.requires_content_evidence = true;
        *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
        *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
        return Some("explicit_command_preserves_generated_file_path_report_execution");
    }
    if explicit_command_structured_observation_contract_should_be_preserved(output_contract) {
        *needs_clarify = false;
        clarify_question.clear();
        output_contract.requires_content_evidence = true;
        *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
        *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
        return Some("explicit_command_preserves_structured_observation_contract");
    }
    if repair_explicit_directory_listing_selector_contract(
        &explicit_command_segment,
        route_reason,
        output_contract,
    ) {
        *needs_clarify = false;
        clarify_question.clear();
        *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
        *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
        return Some("explicit_command_directory_listing_selector_contract_repair");
    }
    let preserve_command_summary_contract = command_output_summary_contract_from_structured_fields(
        output_contract,
        *legacy_normalizer_decision,
        *needs_clarify,
        ascii_token_present(route_reason, "command_result_synthesis"),
    );
    *needs_clarify = false;
    clarify_question.clear();
    output_contract.requires_content_evidence = true;
    output_contract.semantic_kind =
        if output_contract.semantic_kind == OutputSemanticKind::ExecutionFailedStep {
            output_contract.response_shape = OutputResponseShape::Strict;
            OutputSemanticKind::ExecutionFailedStep
        } else if preserve_command_summary_contract {
            OutputSemanticKind::CommandOutputSummary
        } else {
            OutputSemanticKind::RawCommandOutput
        };
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    Some(if preserve_command_summary_contract {
        "explicit_command_requires_command_output_summary_execution"
    } else {
        "explicit_command_requires_fresh_execution"
    })
}

pub(super) fn apply_command_payload_contract_repair(
    command_payload_declared: bool,
    output_contract: &mut IntentOutputContract,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !command_payload_declared || output_contract.delivery_required {
        return None;
    }
    let preserve_command_summary_contract = command_output_summary_contract_from_structured_fields(
        output_contract,
        *legacy_normalizer_decision,
        *needs_clarify,
        false,
    );
    if matches!(output_contract.semantic_kind, OutputSemanticKind::None) {
        output_contract.semantic_kind = if preserve_command_summary_contract {
            OutputSemanticKind::CommandOutputSummary
        } else {
            OutputSemanticKind::RawCommandOutput
        };
    }
    if !matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::RawCommandOutput
            | OutputSemanticKind::ExecutionFailedStep
            | OutputSemanticKind::CommandOutputSummary
    ) {
        return None;
    }
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    *needs_clarify = false;
    clarify_question.clear();
    *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    Some(if preserve_command_summary_contract {
        "command_payload_requires_command_output_summary_execution"
    } else {
        "command_payload_requires_raw_output_execution"
    })
}

pub(super) fn apply_file_delivery_contract_repair(
    wants_file_delivery: bool,
    output_contract: &mut IntentOutputContract,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if !wants_file_delivery
        || output_contract.locator_hint.trim().is_empty()
        || !matches!(
            output_contract.locator_kind,
            OutputLocatorKind::Filename | OutputLocatorKind::Path
        )
        || matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::GeneratedFileDelivery
                | OutputSemanticKind::ContentExcerptSummary
                | OutputSemanticKind::ContentExcerptWithSummary
                | OutputSemanticKind::ArchivePack
                | OutputSemanticKind::ArchiveUnpack
        )
    {
        return None;
    }
    if output_contract.delivery_required
        && output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && output_contract.response_shape == OutputResponseShape::FileToken
    {
        return None;
    }

    *needs_clarify = false;
    clarify_question.clear();
    output_contract.requires_content_evidence = true;
    output_contract.delivery_required = true;
    output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    output_contract.response_shape = OutputResponseShape::FileToken;
    output_contract.semantic_kind = OutputSemanticKind::None;
    *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    Some("file_delivery_request_preserves_delivery_contract")
}

pub(super) fn restore_declared_publishing_preview_contract(
    declared_semantic_kind: OutputSemanticKind,
    active_text_followup_route_repair: Option<&'static str>,
    structural_contract_repair: Option<&'static str>,
    schedule_kind: ScheduleKind,
    output_contract: &mut IntentOutputContract,
    needs_clarify: &mut bool,
    clarify_question: &mut String,
    wants_file_delivery: &mut bool,
    legacy_normalizer_decision: &mut FirstLayerDecision,
    execution_finalize_style: &mut ActFinalizeStyle,
) -> Option<&'static str> {
    if declared_semantic_kind != OutputSemanticKind::PublishingPreview
        || active_text_followup_route_repair.is_some()
        || matches!(
            structural_contract_repair,
            Some("media_generation_path_report_contract_repair")
        )
        || !matches!(schedule_kind, ScheduleKind::None)
    {
        return None;
    }
    if output_contract.semantic_kind == OutputSemanticKind::PublishingPreview
        && output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && matches!(
            *legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
    {
        return None;
    }

    *needs_clarify = false;
    clarify_question.clear();
    *wants_file_delivery = false;
    output_contract.semantic_kind = OutputSemanticKind::PublishingPreview;
    output_contract.requires_content_evidence = true;
    output_contract.delivery_required = false;
    output_contract.delivery_intent = OutputDeliveryIntent::None;
    output_contract.locator_kind = OutputLocatorKind::None;
    output_contract.locator_hint.clear();
    if matches!(
        output_contract.response_shape,
        OutputResponseShape::FileToken
    ) {
        output_contract.response_shape = OutputResponseShape::Free;
    }
    *legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
    *execution_finalize_style = execution_finalize_style_for_contract(output_contract);
    Some("declared_publishing_preview_contract_preserved")
}

fn command_output_summary_contract_from_structured_fields(
    output_contract: &IntentOutputContract,
    legacy_normalizer_decision: FirstLayerDecision,
    needs_clarify: bool,
    command_result_synthesis_marker: bool,
) -> bool {
    let raw_command_output_with_summary_signal = matches!(
        output_contract.semantic_kind,
        OutputSemanticKind::RawCommandOutput
    ) && (matches!(
        output_contract.response_shape,
        OutputResponseShape::OneSentence
    ) || command_result_synthesis_marker);
    !needs_clarify
        && matches!(
            legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
        && output_contract.requires_content_evidence
        && !output_contract.delivery_required
        && matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        && matches!(output_contract.locator_kind, OutputLocatorKind::None)
        && output_contract.locator_hint.trim().is_empty()
        && (matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::None | OutputSemanticKind::CommandOutputSummary
        ) || raw_command_output_with_summary_signal)
        && matches!(
            output_contract.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::OneSentence
                | OutputResponseShape::Strict
        )
}

pub(super) fn route_has_structured_execution_signal(
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
) -> bool {
    wants_file_delivery
        || !matches!(schedule_kind, ScheduleKind::None)
        || output_contract.requires_content_evidence
        || output_contract.delivery_required
        || !matches!(output_contract.locator_kind, OutputLocatorKind::None)
        || !output_contract.locator_hint.trim().is_empty()
        || !matches!(output_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(output_contract.semantic_kind, OutputSemanticKind::None)
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        )
        || !matches!(output_contract.self_extension.mode, SelfExtensionMode::None)
        || !matches!(
            output_contract.self_extension.trigger,
            SelfExtensionTrigger::None
        )
        || output_contract.self_extension.execute_now
        || execution_recipe_hint.is_some_and(|spec| {
            !matches!(
                spec.kind,
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
}

fn explicit_command_structured_observation_contract_should_be_preserved(
    output_contract: &IntentOutputContract,
) -> bool {
    if output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::None
        || output_contract.locator_kind == OutputLocatorKind::None
    {
        return false;
    }
    match output_contract.semantic_kind {
        OutputSemanticKind::DirectoryEntryGroups
        | OutputSemanticKind::FileNames
        | OutputSemanticKind::DirectoryNames
        | OutputSemanticKind::FilePaths
        | OutputSemanticKind::DirectoryPurposeSummary => true,
        OutputSemanticKind::ScalarPathOnly => {
            output_contract.locator_kind == OutputLocatorKind::CurrentWorkspace
        }
        _ => false,
    }
}

fn repair_explicit_directory_listing_selector_contract(
    explicit_command_segment: &str,
    route_reason: &str,
    output_contract: &mut IntentOutputContract,
) -> bool {
    if output_contract.delivery_required
        || output_contract.delivery_intent != OutputDeliveryIntent::None
        || !matches!(
            output_contract.semantic_kind,
            OutputSemanticKind::RawCommandOutput
                | OutputSemanticKind::CommandOutputSummary
                | OutputSemanticKind::None
        )
    {
        return false;
    }
    let Some(path) = safe_ls_directory_path_from_command_segment(explicit_command_segment) else {
        return false;
    };
    let selector_limit = selector_limit_machine_token(route_reason);
    let selector_sort_by = selector_sort_by_machine_token(route_reason);
    if selector_limit.is_none() && selector_sort_by.is_none() {
        return false;
    }
    let target_kind = selector_target_kind_machine_token(route_reason).unwrap_or_default();
    output_contract.semantic_kind = match target_kind {
        OutputScalarCountTargetKind::File => OutputSemanticKind::FileNames,
        OutputScalarCountTargetKind::Dir => OutputSemanticKind::DirectoryNames,
        OutputScalarCountTargetKind::Any => OutputSemanticKind::DirectoryEntryGroups,
    };
    output_contract.response_shape = OutputResponseShape::Strict;
    output_contract.requires_content_evidence = true;
    output_contract.locator_kind = OutputLocatorKind::Path;
    output_contract.locator_hint = path;
    output_contract.self_extension.list_selector.target_kind = target_kind;
    output_contract
        .self_extension
        .list_selector
        .target_kind_specified = selector_target_kind_machine_token(route_reason).is_some();
    output_contract.self_extension.list_selector.limit = selector_limit;
    output_contract.self_extension.list_selector.sort_by = selector_sort_by.clone();
    output_contract.self_extension.list_selector.include_hidden =
        selector_bool_machine_token(route_reason, "selector_include_hidden");
    output_contract
        .self_extension
        .list_selector
        .include_metadata = selector_bool_machine_token(route_reason, "selector_include_metadata")
        .or_else(|| {
            selector_sort_by
                .as_deref()
                .is_some_and(|sort_by| {
                    matches!(
                        sort_by,
                        "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
                    )
                })
                .then_some(true)
        });
    true
}

fn safe_ls_directory_path_from_command_segment(command: &str) -> Option<String> {
    if command.chars().any(|ch| {
        matches!(
            ch,
            '\n' | '\r' | '\0' | '$' | '`' | '|' | ';' | '<' | '>' | '&'
        )
    }) {
        return None;
    }
    let words = command.split_whitespace().collect::<Vec<_>>();
    let first = words
        .first()?
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
    let executable = first
        .rsplit_once('/')
        .map(|(_, basename)| basename)
        .unwrap_or(first);
    if !executable.eq_ignore_ascii_case("ls") {
        return None;
    }
    let mut path: Option<&str> = None;
    let mut after_double_dash = false;
    for word in words.iter().skip(1).copied() {
        let token = word.trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
        if token.is_empty() {
            continue;
        }
        if !after_double_dash && token == "--" {
            after_double_dash = true;
            continue;
        }
        if !after_double_dash && token.starts_with('-') {
            if matches!(
                token,
                "-1" | "-a" | "-A" | "--all" | "--almost-all" | "--color=never"
            ) {
                continue;
            }
            return None;
        }
        if path.replace(token).is_some() {
            return None;
        }
    }
    let path = path.unwrap_or(".");
    safe_shell_path_token(path).then(|| path.to_string())
}

fn safe_shell_path_token(path: &str) -> bool {
    !path.trim().is_empty()
        && path != "-"
        && !path.starts_with('~')
        && !path.contains('$')
        && !path
            .chars()
            .any(|ch| matches!(ch, '\0' | '*' | '?' | '[' | ']' | '{' | '}'))
}

fn selector_value_machine_token(text: &str, key: &str) -> Option<String> {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | '，' | '；'))
        .filter_map(|part| part.trim().strip_prefix(key))
        .filter_map(|value| value.strip_prefix('='))
        .map(|value| {
            value
                .trim_matches(|ch: char| matches!(ch, '.' | '。' | ',' | ';' | '，' | '；'))
                .trim()
                .to_string()
        })
        .find(|value| !value.is_empty())
}

fn selector_limit_machine_token(text: &str) -> Option<u64> {
    selector_value_machine_token(text, "selector_limit")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(1, 1000))
}

fn selector_sort_by_machine_token(text: &str) -> Option<String> {
    selector_value_machine_token(text, "selector_sort_by")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "name" | "name_desc" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

fn selector_target_kind_machine_token(text: &str) -> Option<OutputScalarCountTargetKind> {
    selector_value_machine_token(text, "selector_target_kind")
        .map(|value| value.to_ascii_lowercase())
        .and_then(|value| match value.as_str() {
            "file" | "files" => Some(OutputScalarCountTargetKind::File),
            "dir" | "dirs" | "directory" | "directories" | "folder" | "folders" => {
                Some(OutputScalarCountTargetKind::Dir)
            }
            "any" => Some(OutputScalarCountTargetKind::Any),
            _ => None,
        })
}

fn selector_bool_machine_token(text: &str, key: &str) -> Option<bool> {
    selector_value_machine_token(text, key).and_then(|value| {
        match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        }
    })
}

pub(super) fn direct_answer_decision_should_be_overridden_by_executable_contract(
    needs_clarify: bool,
    legacy_normalizer_decision: FirstLayerDecision,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    direct_answer_contract_repair: Option<&'static str>,
) -> bool {
    !needs_clarify
        && matches!(legacy_normalizer_decision, FirstLayerDecision::DirectAnswer)
        && structured_execution_signal_for_effective_route(
            output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
            direct_answer_contract_repair,
        )
}

pub(super) fn structured_execution_signal_for_effective_route(
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    direct_answer_contract_repair: Option<&'static str>,
) -> bool {
    direct_answer_contract_repair.is_none()
        && route_has_structured_execution_signal(
            output_contract,
            wants_file_delivery,
            schedule_kind,
            execution_recipe_hint,
        )
}

pub(super) fn output_semantic_kind_requires_fresh_evidence(kind: OutputSemanticKind) -> bool {
    matches!(
        kind,
        OutputSemanticKind::RawCommandOutput
            | OutputSemanticKind::ServiceStatus
            | OutputSemanticKind::HiddenEntriesCheck
            | OutputSemanticKind::FileNames
            | OutputSemanticKind::DirectoryNames
            | OutputSemanticKind::DirectoryEntryGroups
            | OutputSemanticKind::FilePaths
            | OutputSemanticKind::DirectoryPurposeSummary
            | OutputSemanticKind::ContentExcerptSummary
            | OutputSemanticKind::DocumentHeading
            | OutputSemanticKind::ContentPresenceCheck
            | OutputSemanticKind::ExcerptKindJudgment
            | OutputSemanticKind::RecentArtifactsJudgment
            | OutputSemanticKind::WorkspaceProjectSummary
            | OutputSemanticKind::ScalarCount
            | OutputSemanticKind::RecentScalarEqualityCheck
            | OutputSemanticKind::ExecutionFailedStep
            | OutputSemanticKind::GeneratedFileDelivery
            | OutputSemanticKind::GeneratedFilePathReport
            | OutputSemanticKind::FilesystemMutationResult
            | OutputSemanticKind::ExistenceWithPath
            | OutputSemanticKind::ExistenceWithPathSummary
            | OutputSemanticKind::GitCommitSubject
            | OutputSemanticKind::GitRepositoryState
            | OutputSemanticKind::StructuredKeys
            | OutputSemanticKind::ConfigValidation
            | OutputSemanticKind::ConfigMutation
            | OutputSemanticKind::ConfigRiskAssessment
            | OutputSemanticKind::SqliteTableListing
            | OutputSemanticKind::SqliteTableNamesOnly
            | OutputSemanticKind::SqliteDatabaseKindJudgment
            | OutputSemanticKind::SqliteSchemaVersion
            | OutputSemanticKind::WeatherQuery
            | OutputSemanticKind::MarketQuote
            | OutputSemanticKind::ImageUnderstanding
            | OutputSemanticKind::PublishingPreview
            | OutputSemanticKind::PackageManagerDetection
            | OutputSemanticKind::ArchiveList
            | OutputSemanticKind::ArchiveRead
            | OutputSemanticKind::ArchivePack
            | OutputSemanticKind::ArchiveUnpack
            | OutputSemanticKind::DockerPs
            | OutputSemanticKind::DockerImages
            | OutputSemanticKind::DockerLogs
            | OutputSemanticKind::DockerContainerLifecycle
    )
}

pub(super) fn parse_execution_recipe_hint(
    out: Option<IntentExecutionRecipeOut>,
) -> Option<crate::execution_recipe::ExecutionRecipeSpec> {
    // 关键语义（B1 修复）：
    //   - `out == None`           → normalizer 没在响应里给出 execution_recipe 字段，
    //                               说明 LLM 没决断；planner-first 主链不再用本地
    //                               keyword detect 代替 LLM 决策。
    //   - `out == Some` 且 kind != none → normalizer 显式给出 ops loop spec，照用。
    //   - `out == Some` 且 kind == none → normalizer 显式说"这不是 ops loop"，
    //                               同样应被信任。返回 Some(default spec)（kind=None,
    //                               runtime.is_active()=false），让下游知道 normalizer
    //                               已分类过，不要再被 legacy local detector 误升级。
    //
    // 这块逻辑是为了修复 act 类只读任务（如 `pwd`）被长期记忆里残留的
    // "configs/" "verify" 等关键字误升级为 OpsClosedLoop config_change，
    // 导致 plan 校验拒绝纯只读 plan、走完 max_repairs 后失败的问题。
    let raw = out?;
    let kind = crate::execution_recipe::parse_execution_recipe_kind_text(&raw.kind);
    let profile = crate::execution_recipe::parse_execution_recipe_profile_text(&raw.profile);
    let target_scope =
        crate::execution_recipe::parse_execution_recipe_target_scope_text(&raw.target_scope);
    if matches!(kind, crate::execution_recipe::ExecutionRecipeKind::None) {
        return Some(crate::execution_recipe::ExecutionRecipeSpec::default());
    }
    crate::execution_recipe::explicit_execution_recipe_spec(kind, profile, target_scope)
        .or_else(|| Some(crate::execution_recipe::ExecutionRecipeSpec::default()))
}
