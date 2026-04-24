#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptSemanticRequestShape {
    ServiceStatusQuestion,
    ScalarCount,
    DirectoryLookup,
    Existence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceRootRequestShape {
    ProjectSummary,
    TomlPathListing,
    HiddenEntriesCount,
    HiddenEntriesCheck,
    DirsOnlyListing,
    CurrentPathScalar,
    PackageManagerDetection,
    GitDirtySummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceChildRequestShape {
    Listing,
    RecentArtifactsJudgment,
    DirectoryPurposeSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortJokePromptShape {
    Eligible,
    WeatherLookupBlocked,
    FileToolingBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceScopePromptShape {
    ExplicitScope,
    ReferenceScope,
    ExplicitAndReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputRequestShape {
    ListOrTable,
    Compare,
    StructuredKeys,
    ExcerptKindJudgment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputCompressionShape {
    Brief,
    ScalarOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathOutputPromptShape {
    ScalarOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableRequestShape {
    SqliteTableListing,
    SqliteSchemaVersion,
    MarkdownRender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryPromptShape {
    PhraseWithoutTarget,
    PhraseWithTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeicticPromptShape {
    ObjectTarget,
    FreshReference,
    GeneralReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineTransformPromptShape {
    ActionWithoutTarget,
    ActionWithTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineJsonShape {
    WholeValue,
    EmbeddedPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldReadPromptShape {
    SimpleExplicitScalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocatorHintPromptShape {
    ExplicitPathOrUrl,
    ConcreteImplicit,
    WorkspaceSingleToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocatorReplyPromptShape {
    LocatorOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileReferencePromptShape {
    DeliveryToken,
    GenericObject,
    FileishReference,
    DeliveryTokenAndGenericObject,
    DeliveryTokenAndFileishReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PromptSurfaceSignals {
    pub(crate) token_count: usize,
    pub(crate) inline_json_shape: Option<InlineJsonShape>,
    pub(crate) locator_hint_prompt_shape: Option<LocatorHintPromptShape>,
    pub(crate) locator_reply_prompt_shape: Option<LocatorReplyPromptShape>,
    pub(crate) field_selector_mentions: Vec<String>,
    pub(crate) field_selector_count: usize,
    pub(crate) dotted_field_selector: Option<String>,
    pub(crate) filename_candidates: Vec<String>,
    pub(crate) filename_candidate_count: usize,
    pub(crate) bare_filename_stem_candidates: Vec<String>,
    pub(crate) bare_filename_stem_candidate_count: usize,
    pub(crate) single_filename_candidate: Option<String>,
    pub(crate) single_bare_filename_stem_candidate: Option<String>,
    pub(crate) directory_file_pair: Option<(String, String)>,
    pub(crate) workspace_single_token_hint: Option<String>,
    pub(crate) file_reference_prompt_shape: Option<FileReferencePromptShape>,
    pub(crate) requested_sentence_count: Option<usize>,
    pub(crate) output_request_shape: Option<OutputRequestShape>,
    pub(crate) output_compression_shape: Option<OutputCompressionShape>,
    pub(crate) path_output_prompt_shape: Option<PathOutputPromptShape>,
    pub(crate) delivery_prompt_shape: Option<DeliveryPromptShape>,
    pub(crate) inline_transform_prompt_shape: Option<InlineTransformPromptShape>,
    pub(crate) field_read_prompt_shape: Option<FieldReadPromptShape>,
    pub(crate) deictic_prompt_shape: Option<DeicticPromptShape>,
    pub(crate) workspace_scope_prompt_shape: Option<WorkspaceScopePromptShape>,
    pub(crate) requested_read_range: Option<crate::read_range_request::RequestedReadRange>,
    pub(crate) requested_listing_limit: Option<usize>,
    pub(crate) workspace_child_directory_hint: Option<String>,
    pub(crate) table_request_shape: Option<TableRequestShape>,
    pub(crate) compare_target_pair: Option<(String, String)>,
    pub(crate) service_status_target: Option<String>,
    pub(crate) semantic_request_shape: Option<PromptSemanticRequestShape>,
    pub(crate) workspace_root_request_shape: Option<WorkspaceRootRequestShape>,
    pub(crate) workspace_child_request_shape: Option<WorkspaceChildRequestShape>,
    pub(crate) short_joke_prompt_shape: Option<ShortJokePromptShape>,
}

impl PromptSurfaceSignals {
    pub(crate) fn has_explicit_path_or_url(&self) -> bool {
        matches!(
            self.locator_hint_prompt_shape,
            Some(LocatorHintPromptShape::ExplicitPathOrUrl)
        )
    }

    pub(crate) fn has_concrete_locator_hint(&self) -> bool {
        matches!(
            self.locator_hint_prompt_shape,
            Some(
                LocatorHintPromptShape::ExplicitPathOrUrl
                    | LocatorHintPromptShape::ConcreteImplicit
            )
        )
    }

    pub(crate) fn looks_like_locator_only_reply(&self) -> bool {
        matches!(
            self.locator_reply_prompt_shape,
            Some(LocatorReplyPromptShape::LocatorOnly)
        )
    }

    pub(crate) fn has_any_locator_reference(&self) -> bool {
        self.has_concrete_locator_hint() || self.has_workspace_single_token_hint()
    }

    pub(crate) fn has_workspace_single_token_hint(&self) -> bool {
        self.workspace_single_token_hint.is_some()
    }

    pub(crate) fn has_single_filename_candidate(&self) -> bool {
        self.single_filename_candidate.is_some()
    }

    pub(crate) fn single_filename_candidate(&self) -> Option<&str> {
        self.single_filename_candidate.as_deref()
    }

    pub(crate) fn has_structured_target_refinement(&self) -> bool {
        self.field_selector_count > 0
            || self.requested_read_range.is_some()
            || self.requested_listing_limit.is_some()
    }

    pub(crate) fn has_generic_or_fileish_reference(&self) -> bool {
        matches!(
            self.file_reference_prompt_shape,
            Some(
                FileReferencePromptShape::GenericObject
                    | FileReferencePromptShape::FileishReference
                    | FileReferencePromptShape::DeliveryTokenAndGenericObject
                    | FileReferencePromptShape::DeliveryTokenAndFileishReference
            )
        )
    }

    pub(crate) fn has_deictic_reference(&self) -> bool {
        self.deictic_prompt_shape.is_some()
    }

    pub(crate) fn has_fresh_or_object_deictic_reference(&self) -> bool {
        matches!(
            self.deictic_prompt_shape,
            Some(DeicticPromptShape::ObjectTarget | DeicticPromptShape::FreshReference)
        )
    }

    pub(crate) fn filename_candidates_excluding_field_selectors(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for candidate in &self.filename_candidates {
            if self
                .dotted_field_selector
                .as_ref()
                .is_some_and(|selector| selector.eq_ignore_ascii_case(candidate))
            {
                continue;
            }
            if self
                .field_selector_mentions
                .iter()
                .any(|selector| selector.eq_ignore_ascii_case(candidate))
            {
                continue;
            }
            let normalized = candidate.to_ascii_lowercase();
            if seen.insert(normalized) {
                out.push(candidate.clone());
            }
        }
        out
    }
}

fn workspace_root_for_surface_signals() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("clawd crate should live under workspace_root/crates/clawd")
            .to_path_buf()
    })
}

pub(crate) fn analyze_prompt_surface(prompt: &str) -> PromptSurfaceSignals {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return PromptSurfaceSignals::default();
    }
    let token_count = trimmed.split_whitespace().count();
    let field_selector_mentions = extract_field_selector_mentions(trimmed);
    let field_selector_count = field_selector_mentions.len();
    let dotted_field_selector = extract_dotted_field_selector(trimmed);
    let inline_json_shape = classify_inline_json_shape(trimmed);
    let filename_candidates = crate::delivery_utils::extract_filename_candidates(trimmed);
    let filename_candidate_count = filename_candidates.len();
    let single_filename_candidate = {
        let mut unique = filename_candidates.clone();
        unique.dedup();
        (unique.len() == 1).then(|| unique.remove(0))
    };
    let bare_filename_stem_candidates =
        crate::delivery_utils::extract_bare_filename_stem_candidates(trimmed);
    let bare_filename_stem_candidate_count = bare_filename_stem_candidates.len();
    let single_bare_filename_stem_candidate = {
        let mut unique = bare_filename_stem_candidates.clone();
        unique.dedup();
        (unique.len() == 1).then(|| unique.remove(0))
    };
    let workspace_single_token_hint = extract_workspace_existing_single_token_hint(trimmed);
    let directory_file_pair = crate::delivery_utils::extract_directory_and_file_pair(trimmed);
    let has_explicit_path_or_url = has_explicit_path_or_url_shape(trimmed);
    let has_concrete_locator_hint = crate::worker::has_concrete_locator_hint(trimmed);
    let looks_like_locator_only_reply =
        crate::clarify_followup::prompt_looks_like_locator_only(trimmed);
    let locator_hint_prompt_shape = classify_locator_hint_prompt_shape(
        has_explicit_path_or_url,
        has_concrete_locator_hint,
        workspace_single_token_hint.is_some(),
    );
    let locator_reply_prompt_shape =
        looks_like_locator_only_reply.then_some(LocatorReplyPromptShape::LocatorOnly);
    let output_request_shape = detect_output_request_shape(trimmed);
    let output_compression_shape = detect_output_compression_shape(trimmed);
    let path_output_prompt_shape = detect_path_output_prompt_shape(trimmed);
    let requested_sentence_count = requested_sentence_count_shape(trimmed);
    let requests_inline_transform_action_shape =
        prompt_requests_inline_transform_action_shape(trimmed);
    let requests_delivery_phrase = prompt_requests_delivery_phrase(trimmed);
    let references_deictic_object = prompt_references_deictic_object(trimmed);
    let has_delivery_token_reference = prompt_contains_delivery_token_reference(trimmed);
    let mentions_generic_file_object = prompt_mentions_generic_file_object(trimmed);
    let mentions_fileish_reference_shape = prompt_mentions_fileish_reference_shape(trimmed);
    let file_reference_prompt_shape = classify_file_reference_prompt_shape(
        has_delivery_token_reference,
        mentions_generic_file_object,
        mentions_fileish_reference_shape,
    );
    let contains_deictic_reference_shape = prompt_contains_deictic_reference_shape(trimmed);
    let deictic_prompt_shape = classify_deictic_prompt_shape(
        references_deictic_object,
        contains_deictic_reference_shape,
        has_explicit_path_or_url,
    );
    let requested_read_range =
        crate::read_range_request::extract_explicit_read_range_request(trimmed);
    let has_delivery_target_shape = has_concrete_locator_hint
        || filename_candidate_count > 0
        || bare_filename_stem_candidate_count > 0
        || workspace_single_token_hint.is_some()
        || has_delivery_token_reference
        || matches!(deictic_prompt_shape, Some(DeicticPromptShape::ObjectTarget));
    let delivery_prompt_shape = classify_delivery_prompt_shape(
        requests_delivery_phrase,
        has_delivery_target_shape,
        matches!(
            file_reference_prompt_shape,
            Some(
                FileReferencePromptShape::GenericObject
                    | FileReferencePromptShape::DeliveryTokenAndGenericObject
            )
        ),
    );
    let has_inline_transform_target_shape = contains_inline_transform_target_shape(
        inline_json_shape.is_some(),
        looks_like_locator_only_reply,
        requested_read_range.is_some(),
        field_selector_count,
        has_delivery_token_reference,
    );
    let inline_transform_prompt_shape = classify_inline_transform_prompt_shape(
        requests_inline_transform_action_shape,
        has_inline_transform_target_shape,
    );
    let looks_like_simple_explicit_field_read_shape =
        prompt_looks_like_simple_explicit_field_read_shape(
            inline_json_shape.is_some(),
            output_compression_shape,
            output_request_shape,
            has_explicit_path_or_url,
            has_concrete_locator_hint,
            requested_read_range.is_some(),
            count_non_filename_field_mentions_shape(&field_selector_mentions, &filename_candidates),
            token_count,
        );
    let field_read_prompt_shape = looks_like_simple_explicit_field_read_shape
        .then_some(FieldReadPromptShape::SimpleExplicitScalar);
    let mentions_current_workspace_scope_shape = prompt_mentions_current_workspace_scope(trimmed);
    let mentions_current_workspace_scope_reference_shape =
        prompt_mentions_current_workspace_scope_reference_shape(trimmed);
    let workspace_scope_prompt_shape = classify_workspace_scope_prompt_shape(
        mentions_current_workspace_scope_shape,
        mentions_current_workspace_scope_reference_shape,
    );
    let mentions_current_workspace_or_this_directory =
        prompt_mentions_current_workspace_or_this_directory(trimmed);
    let requests_hidden_entries = prompt_requests_hidden_entries(trimmed);
    let requests_workspace_child_listing = prompt_requests_workspace_child_listing(trimmed);
    let requests_directory_only_listing = prompt_requests_directory_only_listing(trimmed);
    let semantic_request_shape = detect_prompt_semantic_request_shape(trimmed);
    let requests_scalar_count = matches!(
        semantic_request_shape,
        Some(PromptSemanticRequestShape::ScalarCount)
    );
    let lower = trimmed.to_ascii_lowercase();
    let mentions_project_structure = trimmed.contains("怎么组织")
        || trimmed.contains("大概怎么组织")
        || trimmed.contains("怎么分区")
        || trimmed.contains("大概怎么分区")
        || trimmed.contains("像是什么项目")
        || trimmed.contains("像什么项目")
        || trimmed.contains("适合新手")
        || lower.contains("how this project is organized")
        || lower.contains("how the project is organized")
        || lower.contains("what this project is")
        || lower.contains("what kind of project")
        || lower.contains("how this repo is organized")
        || lower.contains("plain sentence");
    let mentions_overview = trimmed.contains("扫一眼")
        || trimmed.contains("整体")
        || trimmed.contains("先看")
        || trimmed.contains("先看看")
        || lower.contains("inspect")
        || lower.contains("overview")
        || lower.contains("glance")
        || lower.contains("top-level");
    let compact_summary_shape = matches!(
        output_compression_shape,
        Some(OutputCompressionShape::Brief)
    ) || requested_sentence_count == Some(1);
    let requests_workspace_project_summary = mentions_current_workspace_scope_shape
        && mentions_project_structure
        && (mentions_overview || requests_workspace_child_listing || compact_summary_shape);
    let requests_toml_path_listing = prompt_requests_toml_path_listing(trimmed);
    let requests_directory_purpose_summary = requests_workspace_child_listing
        && (trimmed.contains("更像说明文档还是运行产物")
            || trimmed.contains("更像文档还是运行产物")
            || trimmed.contains("这些是干什么的")
            || lower.contains("more like docs or runtime")
            || lower.contains("more like documentation or runtime")
            || lower.contains("what these files are for"));
    let requests_package_manager_detection = prompt_requests_package_manager_detection(trimmed);
    let requests_current_workspace_path_scalar =
        prompt_requests_current_workspace_path_scalar(trimmed);
    let requested_listing_limit =
        crate::listing_limit_request::requested_listing_limit_from_prompt(trimmed);
    let requests_workspace_hidden_entries_count = mentions_current_workspace_or_this_directory
        && requests_hidden_entries
        && requests_scalar_count;
    let requests_workspace_hidden_entries_check = mentions_current_workspace_or_this_directory
        && requests_hidden_entries
        && !requests_workspace_hidden_entries_count;
    let requests_workspace_dirs_only_listing = requests_workspace_child_listing
        && mentions_current_workspace_scope_shape
        && requests_directory_only_listing
        && requested_listing_limit.is_none();
    let workspace_child_directory_hint = extract_workspace_child_directory_hint_shape(trimmed);
    let table_request_shape = detect_table_request_shape(trimmed);
    let requests_git_dirty_summary = prompt_requests_git_dirty_summary(trimmed);
    let requests_recent_artifacts_judgment = prompt_requests_recent_artifacts_judgment(trimmed);
    let short_joke_prompt_shape = detect_short_joke_prompt_shape(trimmed);
    let compare_target_pair = detect_compare_targets_shape(trimmed);
    let service_status_target = extract_service_status_target_shape(trimmed);
    let workspace_root_request_shape = classify_workspace_root_request_shape(
        requests_workspace_project_summary,
        requests_toml_path_listing,
        requests_package_manager_detection,
        requests_current_workspace_path_scalar,
        requests_workspace_hidden_entries_count,
        requests_workspace_hidden_entries_check,
        requests_workspace_dirs_only_listing,
        requests_git_dirty_summary,
        workspace_scope_prompt_shape,
    );
    let workspace_child_request_shape = classify_workspace_child_request_shape(
        requests_directory_purpose_summary,
        requests_recent_artifacts_judgment,
        requests_workspace_child_listing,
    );
    PromptSurfaceSignals {
        token_count,
        inline_json_shape,
        locator_hint_prompt_shape,
        locator_reply_prompt_shape,
        field_selector_mentions,
        field_selector_count,
        dotted_field_selector,
        filename_candidates,
        filename_candidate_count,
        bare_filename_stem_candidates,
        bare_filename_stem_candidate_count,
        single_filename_candidate,
        single_bare_filename_stem_candidate,
        directory_file_pair,
        workspace_single_token_hint,
        file_reference_prompt_shape,
        requested_sentence_count,
        output_request_shape,
        output_compression_shape,
        path_output_prompt_shape,
        delivery_prompt_shape,
        inline_transform_prompt_shape,
        field_read_prompt_shape,
        deictic_prompt_shape,
        workspace_scope_prompt_shape,
        requested_read_range,
        requested_listing_limit,
        workspace_child_directory_hint,
        table_request_shape,
        compare_target_pair,
        service_status_target,
        semantic_request_shape,
        workspace_root_request_shape,
        workspace_child_request_shape,
        short_joke_prompt_shape,
    }
}

fn normalize_surface_prompt<'a>(prompt: Option<&'a str>) -> Option<&'a str> {
    prompt.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn requested_read_range_from_prompt_pair(
    primary_prompt: Option<&str>,
    fallback_prompt: &str,
) -> Option<crate::read_range_request::RequestedReadRange> {
    let primary_prompt = normalize_surface_prompt(primary_prompt);
    let fallback_prompt = normalize_surface_prompt(Some(fallback_prompt));
    if let Some(range) =
        primary_prompt.and_then(|prompt| analyze_prompt_surface(prompt).requested_read_range)
    {
        return Some(range);
    }
    let fallback_prompt = fallback_prompt?;
    if primary_prompt.is_some_and(|prompt| prompt == fallback_prompt) {
        return None;
    }
    analyze_prompt_surface(fallback_prompt).requested_read_range
}

pub(crate) fn requested_listing_limit_from_prompt_pair(
    primary_prompt: Option<&str>,
    fallback_prompt: &str,
) -> Option<usize> {
    let primary_prompt = normalize_surface_prompt(primary_prompt);
    let fallback_prompt = normalize_surface_prompt(Some(fallback_prompt));
    if let Some(limit) =
        primary_prompt.and_then(|prompt| analyze_prompt_surface(prompt).requested_listing_limit)
    {
        return Some(limit);
    }
    let fallback_prompt = fallback_prompt?;
    if primary_prompt.is_some_and(|prompt| prompt == fallback_prompt) {
        return None;
    }
    analyze_prompt_surface(fallback_prompt).requested_listing_limit
}

fn classify_locator_hint_prompt_shape(
    has_explicit_path_or_url: bool,
    has_concrete_locator_hint: bool,
    has_workspace_single_token_hint: bool,
) -> Option<LocatorHintPromptShape> {
    if has_explicit_path_or_url {
        Some(LocatorHintPromptShape::ExplicitPathOrUrl)
    } else if has_concrete_locator_hint {
        Some(LocatorHintPromptShape::ConcreteImplicit)
    } else if has_workspace_single_token_hint {
        Some(LocatorHintPromptShape::WorkspaceSingleToken)
    } else {
        None
    }
}

fn detect_output_request_shape(prompt: &str) -> Option<OutputRequestShape> {
    if prompt_requests_excerpt_kind_judgment(prompt) {
        Some(OutputRequestShape::ExcerptKindJudgment)
    } else if prompt_requests_compare_shape(prompt) {
        Some(OutputRequestShape::Compare)
    } else if prompt_requests_structured_keys_shape(prompt) {
        Some(OutputRequestShape::StructuredKeys)
    } else if prompt_requests_list_or_table_shape(prompt) {
        Some(OutputRequestShape::ListOrTable)
    } else {
        None
    }
}

fn detect_output_compression_shape(prompt: &str) -> Option<OutputCompressionShape> {
    if prompt_requests_scalar_only_shape(prompt) {
        Some(OutputCompressionShape::ScalarOnly)
    } else if prompt_requests_brief_shape(prompt) {
        Some(OutputCompressionShape::Brief)
    } else {
        None
    }
}

fn detect_path_output_prompt_shape(prompt: &str) -> Option<PathOutputPromptShape> {
    let lower = prompt.to_ascii_lowercase();
    let asks_path = prompt.contains("路径")
        || lower.contains(" path")
        || lower.contains("path ")
        || lower.ends_with("path");
    let asks_location = prompt.contains("在哪")
        || prompt.contains("在哪里")
        || lower.contains("where is")
        || lower.contains("where's");
    let output_only = prompt.contains("只输出")
        || prompt.contains("只给")
        || prompt.contains("只回")
        || prompt.contains("只要")
        || prompt.contains("就行")
        || prompt.contains("就好")
        || lower.contains("path only")
        || lower.contains("only path")
        || lower.contains("only the path")
        || lower.contains("output only the path")
        || lower.contains("return only the path")
        || lower.contains("just output the path")
        || lower.contains("just give the path")
        || lower.contains("just the path")
        || lower.contains("just path");
    let conflicting = prompt.contains("路径列表")
        || lower.contains("path list")
        || lower.contains("paths only")
        || prompt.contains("发给我")
        || prompt.contains("发我")
        || lower.contains("send me");
    ((asks_path && output_only) || (asks_location && asks_path && output_only))
        .then_some(PathOutputPromptShape::ScalarOnly)
        .filter(|_| !conflicting)
}

fn detect_table_request_shape(prompt: &str) -> Option<TableRequestShape> {
    if prompt_requests_sqlite_table_listing(prompt) {
        Some(TableRequestShape::SqliteTableListing)
    } else if prompt_requests_sqlite_schema_version(prompt) {
        Some(TableRequestShape::SqliteSchemaVersion)
    } else if prompt_requests_markdown_table_render(prompt) {
        Some(TableRequestShape::MarkdownRender)
    } else {
        None
    }
}

fn classify_delivery_prompt_shape(
    requests_delivery_phrase: bool,
    has_delivery_target_shape: bool,
    mentions_generic_file_object: bool,
) -> Option<DeliveryPromptShape> {
    if !requests_delivery_phrase {
        None
    } else if has_delivery_target_shape || mentions_generic_file_object {
        Some(DeliveryPromptShape::PhraseWithTarget)
    } else {
        Some(DeliveryPromptShape::PhraseWithoutTarget)
    }
}

fn classify_deictic_prompt_shape(
    references_deictic_object: bool,
    contains_deictic_reference_shape: bool,
    has_explicit_path_or_url: bool,
) -> Option<DeicticPromptShape> {
    if references_deictic_object {
        Some(DeicticPromptShape::ObjectTarget)
    } else if !has_explicit_path_or_url && contains_deictic_reference_shape {
        Some(DeicticPromptShape::FreshReference)
    } else if contains_deictic_reference_shape {
        Some(DeicticPromptShape::GeneralReference)
    } else {
        None
    }
}

fn classify_inline_transform_prompt_shape(
    requests_inline_transform_action_shape: bool,
    has_inline_transform_target_shape: bool,
) -> Option<InlineTransformPromptShape> {
    if !requests_inline_transform_action_shape {
        None
    } else if has_inline_transform_target_shape {
        Some(InlineTransformPromptShape::ActionWithTarget)
    } else {
        Some(InlineTransformPromptShape::ActionWithoutTarget)
    }
}

fn classify_inline_json_shape(prompt: &str) -> Option<InlineJsonShape> {
    crate::extract_first_json_value_any(prompt).map(|value| {
        if value.trim() == prompt {
            InlineJsonShape::WholeValue
        } else {
            InlineJsonShape::EmbeddedPayload
        }
    })
}

fn classify_file_reference_prompt_shape(
    has_delivery_token_reference: bool,
    mentions_generic_file_object: bool,
    mentions_fileish_reference_shape: bool,
) -> Option<FileReferencePromptShape> {
    match (
        has_delivery_token_reference,
        mentions_generic_file_object,
        mentions_fileish_reference_shape,
    ) {
        (true, _, true) => Some(FileReferencePromptShape::DeliveryTokenAndFileishReference),
        (true, true, false) => Some(FileReferencePromptShape::DeliveryTokenAndGenericObject),
        (true, false, false) => Some(FileReferencePromptShape::DeliveryToken),
        (false, true, _) => Some(FileReferencePromptShape::GenericObject),
        (false, false, true) => Some(FileReferencePromptShape::FileishReference),
        (false, false, false) => None,
    }
}

fn detect_prompt_semantic_request_shape(prompt: &str) -> Option<PromptSemanticRequestShape> {
    if looks_like_service_status_question(prompt) {
        Some(PromptSemanticRequestShape::ServiceStatusQuestion)
    } else if prompt_requests_scalar_count(prompt) {
        Some(PromptSemanticRequestShape::ScalarCount)
    } else if prompt_requests_directory_lookup(prompt) {
        Some(PromptSemanticRequestShape::DirectoryLookup)
    } else if prompt_requests_existence(prompt) {
        Some(PromptSemanticRequestShape::Existence)
    } else {
        None
    }
}

fn classify_workspace_root_request_shape(
    requests_workspace_project_summary: bool,
    requests_toml_path_listing: bool,
    requests_package_manager_detection: bool,
    requests_current_workspace_path_scalar: bool,
    requests_workspace_hidden_entries_count: bool,
    requests_workspace_hidden_entries_check: bool,
    requests_workspace_dirs_only_listing: bool,
    requests_git_dirty_summary: bool,
    workspace_scope_prompt_shape: Option<WorkspaceScopePromptShape>,
) -> Option<WorkspaceRootRequestShape> {
    if requests_current_workspace_path_scalar {
        Some(WorkspaceRootRequestShape::CurrentPathScalar)
    } else if requests_workspace_hidden_entries_count {
        Some(WorkspaceRootRequestShape::HiddenEntriesCount)
    } else if requests_workspace_hidden_entries_check {
        Some(WorkspaceRootRequestShape::HiddenEntriesCheck)
    } else if requests_workspace_dirs_only_listing {
        Some(WorkspaceRootRequestShape::DirsOnlyListing)
    } else if requests_package_manager_detection {
        Some(WorkspaceRootRequestShape::PackageManagerDetection)
    } else if requests_git_dirty_summary
        && workspace_scope_shape_has_explicit_scope(workspace_scope_prompt_shape)
    {
        Some(WorkspaceRootRequestShape::GitDirtySummary)
    } else if requests_workspace_project_summary {
        Some(WorkspaceRootRequestShape::ProjectSummary)
    } else if requests_toml_path_listing {
        Some(WorkspaceRootRequestShape::TomlPathListing)
    } else {
        None
    }
}

fn classify_workspace_scope_prompt_shape(
    mentions_current_workspace_scope_shape: bool,
    mentions_current_workspace_scope_reference_shape: bool,
) -> Option<WorkspaceScopePromptShape> {
    match (
        mentions_current_workspace_scope_shape,
        mentions_current_workspace_scope_reference_shape,
    ) {
        (true, true) => Some(WorkspaceScopePromptShape::ExplicitAndReference),
        (true, false) => Some(WorkspaceScopePromptShape::ExplicitScope),
        (false, true) => Some(WorkspaceScopePromptShape::ReferenceScope),
        (false, false) => None,
    }
}

pub(crate) fn workspace_scope_shape_has_explicit_scope(
    shape: Option<WorkspaceScopePromptShape>,
) -> bool {
    matches!(
        shape,
        Some(
            WorkspaceScopePromptShape::ExplicitScope
                | WorkspaceScopePromptShape::ExplicitAndReference
        )
    )
}

pub(crate) fn workspace_scope_shape_has_reference_scope(
    shape: Option<WorkspaceScopePromptShape>,
) -> bool {
    matches!(
        shape,
        Some(
            WorkspaceScopePromptShape::ReferenceScope
                | WorkspaceScopePromptShape::ExplicitAndReference
        )
    )
}

fn classify_workspace_child_request_shape(
    requests_directory_purpose_summary: bool,
    requests_recent_artifacts_judgment: bool,
    requests_workspace_child_listing: bool,
) -> Option<WorkspaceChildRequestShape> {
    if requests_directory_purpose_summary {
        Some(WorkspaceChildRequestShape::DirectoryPurposeSummary)
    } else if requests_recent_artifacts_judgment {
        Some(WorkspaceChildRequestShape::RecentArtifactsJudgment)
    } else if requests_workspace_child_listing {
        Some(WorkspaceChildRequestShape::Listing)
    } else {
        None
    }
}

fn detect_short_joke_prompt_shape(prompt: &str) -> Option<ShortJokePromptShape> {
    if !prompt_requests_short_joke_shape(prompt) {
        None
    } else if prompt_mentions_file_write_tooling_shape(prompt) {
        Some(ShortJokePromptShape::FileToolingBlocked)
    } else if prompt_mentions_weather_lookup_shape(prompt)
        && !prompt_requests_weather_lookup_suppression_shape(prompt)
    {
        Some(ShortJokePromptShape::WeatherLookupBlocked)
    } else {
        Some(ShortJokePromptShape::Eligible)
    }
}

fn has_explicit_path_or_url_shape(prompt: &str) -> bool {
    crate::worker::has_explicit_path_or_url_locator_hint(prompt)
}

fn contains_inline_transform_target_shape(
    has_inline_json_shape: bool,
    looks_like_locator_only_reply: bool,
    has_requested_read_range: bool,
    field_selector_count: usize,
    has_delivery_token_reference: bool,
) -> bool {
    has_inline_json_shape
        && !looks_like_locator_only_reply
        && !has_requested_read_range
        && field_selector_count == 0
        && !has_delivery_token_reference
}

fn prompt_looks_like_simple_explicit_field_read_shape(
    has_inline_json_shape: bool,
    output_compression_shape: Option<OutputCompressionShape>,
    output_request_shape: Option<OutputRequestShape>,
    has_explicit_path_or_url: bool,
    has_concrete_locator_hint: bool,
    has_requested_read_range: bool,
    non_filename_field_mention_count: usize,
    token_count: usize,
) -> bool {
    if !(has_explicit_path_or_url || has_concrete_locator_hint)
        || has_inline_json_shape
        || matches!(
            output_compression_shape,
            Some(OutputCompressionShape::Brief)
        )
        || output_request_shape.is_some()
        || has_requested_read_range
    {
        return false;
    }
    non_filename_field_mention_count == 1 && token_count <= 24
}

fn count_non_filename_field_mentions_shape(
    field_selector_mentions: &[String],
    filename_candidates: &[String],
) -> usize {
    let filename_candidates = filename_candidates
        .iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    field_selector_mentions
        .iter()
        .filter(|selector| {
            !filename_candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(selector))
        })
        .count()
}

pub(crate) fn looks_like_service_status_question(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let asks_status = prompt.contains("在运行")
        || prompt.contains("活着")
        || prompt.contains("状态")
        || lower.contains("is running")
        || lower.contains("running right now")
        || lower.contains("status");
    let mutating = prompt.contains("重启")
        || prompt.contains("停止")
        || prompt.contains("启动")
        || lower.contains("restart")
        || lower.contains("stop ")
        || lower.contains("start ");
    asks_status && !mutating
}

pub(crate) fn prompt_requests_scalar_count(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("数一下")
        || prompt.contains("多少个")
        || prompt.contains("几个")
        || lower.contains("count ")
        || lower.contains("count how many")
        || lower.contains("how many")
}

pub(crate) fn prompt_requests_directory_lookup(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let asks_listing = prompt.contains("有什么")
        || prompt.contains("都有啥")
        || prompt.contains("都有什么")
        || prompt.contains("下面有什么")
        || prompt.contains("下面都有什么")
        || prompt.contains("里面有什么")
        || lower.contains("what is in")
        || lower.contains("what's in")
        || lower.contains("what is inside")
        || lower.contains("what's inside")
        || lower.contains("list what is in")
        || lower.contains("list what's in")
        || lower.contains("show what is in")
        || lower.contains("show what's in");
    let mutating = prompt.contains("删除")
        || prompt.contains("移动")
        || prompt.contains("重命名")
        || lower.contains("delete ")
        || lower.contains("move ")
        || lower.contains("rename ");
    asks_listing && !mutating
}

pub(crate) fn prompt_requests_existence(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("在不在")
        || prompt.contains("存不存在")
        || prompt.contains("存在吗")
        || prompt.contains("有没有")
        || lower.contains("is there")
        || lower.contains("does it exist")
        || lower.contains("does that exist")
        || lower.contains("exists")
}

pub(crate) fn prompt_mentions_current_workspace_scope(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("当前目录")
        || prompt.contains("当前工作区")
        || prompt.contains("当前仓库")
        || prompt.contains("这个仓库")
        || prompt.contains("最外层")
        || lower.contains("current directory")
        || lower.contains("current workspace")
        || lower.contains("current repo")
        || lower.contains("top-level")
}

pub(crate) fn prompt_mentions_current_workspace_or_this_directory(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt_mentions_current_workspace_scope(prompt)
        || prompt.contains("这个目录")
        || lower.contains("this directory")
}

pub(crate) fn prompt_requests_hidden_entries(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("隐藏文件")
        || prompt.contains("点开头")
        || prompt.contains("点前缀")
        || lower.contains("hidden file")
        || lower.contains("hidden files")
        || lower.contains("hidden entry")
        || lower.contains("hidden entries")
        || lower.contains("dot-prefixed")
        || lower.contains("starting with a dot")
}

pub(crate) fn prompt_requests_workspace_child_listing(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("列出")
        || prompt.contains("有哪些文件")
        || prompt.contains("文件名列表")
        || prompt.contains("哪些文件")
        || lower.contains("list ")
        || lower.contains("show ")
        || lower.contains("which files")
        || lower.contains("file names")
}

pub(crate) fn prompt_requests_directory_only_listing(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("文件夹")
        || prompt.contains("目录名")
        || lower.contains("directories")
        || lower.contains("directory names")
        || lower.contains("folders")
}

pub(crate) fn prompt_requests_recent_artifacts_judgment(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let mentions_recent = prompt.contains("最近修改")
        || lower.contains("recently modified")
        || lower.contains("most recently modified")
        || lower.contains("recent ");
    let mentions_judgment =
        prompt.contains("更像") || lower.contains("more like") || lower.contains("formal deliver");
    mentions_recent && mentions_judgment
}

#[allow(dead_code)]
pub(crate) fn prompt_requests_workspace_project_summary(prompt: &str) -> bool {
    if !prompt_mentions_current_workspace_scope(prompt) {
        return false;
    }
    let lower = prompt.to_ascii_lowercase();
    let mentions_structure = prompt.contains("怎么组织")
        || prompt.contains("大概怎么组织")
        || prompt.contains("怎么分区")
        || prompt.contains("大概怎么分区")
        || prompt.contains("像是什么项目")
        || prompt.contains("像什么项目")
        || prompt.contains("适合新手")
        || lower.contains("how this project is organized")
        || lower.contains("how the project is organized")
        || lower.contains("what this project is")
        || lower.contains("what kind of project")
        || lower.contains("how this repo is organized")
        || lower.contains("plain sentence");
    let mentions_overview = prompt.contains("扫一眼")
        || prompt.contains("整体")
        || prompt.contains("先看")
        || prompt.contains("先看看")
        || lower.contains("inspect")
        || lower.contains("overview")
        || lower.contains("glance")
        || lower.contains("top-level");
    let compact_summary_shape =
        prompt_requests_brief_shape(prompt) || requested_sentence_count_shape(prompt) == Some(1);
    mentions_structure
        && (mentions_overview
            || prompt_requests_workspace_child_listing(prompt)
            || compact_summary_shape)
}

pub(crate) fn prompt_requests_toml_path_listing(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let asks_toml =
        prompt.contains("toml 文件") || lower.contains("toml file") || lower.contains("toml files");
    let asks_paths = prompt.contains("路径列表")
        || prompt.contains("只输出路径")
        || lower.contains("path list")
        || lower.contains("paths only")
        || lower.contains("output only paths");
    asks_toml && asks_paths
}

#[allow(dead_code)]
pub(crate) fn prompt_requests_directory_purpose_summary(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt_requests_workspace_child_listing(prompt)
        && (prompt.contains("更像说明文档还是运行产物")
            || prompt.contains("更像文档还是运行产物")
            || prompt.contains("这些是干什么的")
            || lower.contains("more like docs or runtime")
            || lower.contains("more like documentation or runtime")
            || lower.contains("what these files are for"))
}

pub(crate) fn prompt_requests_package_manager_detection(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("包管理器") || lower.contains("package manager")
}

pub(crate) fn prompt_requests_git_dirty_summary(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("未提交改动")
        || prompt.contains("有没有改动")
        || lower.contains("uncommitted changes")
        || lower.contains("git dirty")
        || lower.contains("dirty working tree")
}

pub(crate) fn prompt_requests_current_workspace_path_scalar(prompt: &str) -> bool {
    if prompt.is_empty() {
        return false;
    }
    let lower = prompt.to_ascii_lowercase();
    let mentions_current_dir = prompt.contains("当前工作目录")
        || prompt.contains("当前目录")
        || prompt.contains("当前路径")
        || prompt.contains("现在在哪个目录")
        || lower.contains("current working directory")
        || lower.contains("current directory")
        || lower.contains("pwd");
    let asks_for_path = prompt.contains("绝对路径")
        || prompt.contains("完整路径")
        || prompt.contains("路径")
        || lower.contains("absolute path")
        || lower.contains("full path")
        || lower.contains("path")
        || lower.contains("pwd");
    let output_only = prompt.contains("只输出")
        || prompt.contains("只回")
        || prompt.contains("直接把")
        || prompt.contains("发我就行")
        || prompt.contains("不要解释")
        || prompt.contains("不用解释")
        || lower.contains("only")
        || lower.contains("just")
        || lower.contains("do not explain")
        || lower.contains("don't explain")
        || lower.contains("without explanation");
    let conflicting = prompt.contains("比较")
        || prompt.contains("对比")
        || prompt.contains("列出")
        || prompt.contains("笑话")
        || lower.contains("compare")
        || lower.contains("list ")
        || lower.contains("joke");
    mentions_current_dir && asks_for_path && output_only && !conflicting
}

fn normalize_nl_phrase(prompt: &str) -> String {
    let mut normalized = String::with_capacity(prompt.len());
    let mut last_was_space = false;
    for ch in prompt.trim().chars() {
        let is_space = ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '.'
                    | '。'
                    | '!'
                    | '！'
                    | '?'
                    | '？'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '"'
                    | '\''
                    | '`'
            );
        if is_space {
            if !last_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            last_was_space = true;
            continue;
        }
        for lower in ch.to_lowercase() {
            normalized.push(lower);
        }
        last_was_space = false;
    }
    normalized.trim().to_string()
}

fn normalized_contains_phrase(normalized_prompt: &str, phrase: &str) -> bool {
    if normalized_prompt == phrase {
        return true;
    }
    let phrase_len = phrase.split_whitespace().count();
    normalized_prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(phrase_len)
        .any(|window| window.join(" ") == phrase)
}

pub(crate) fn prompt_contains_deictic_reference_shape(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = normalize_nl_phrase(trimmed);
    normalized_contains_phrase(&normalized, "this")
        || normalized_contains_phrase(&normalized, "that")
        || ["那个", "这个", "那份", "这份"]
            .iter()
            .any(|needle| trimmed.contains(needle))
        || [
            "该文件",
            "该日志",
            "该配置",
            "该脚本",
            "该目录",
            "该文档",
            "该服务",
        ]
        .iter()
        .any(|needle| trimmed.contains(needle))
}

pub(crate) fn prompt_mentions_current_workspace_scope_reference_shape(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = normalize_nl_phrase(trimmed);
    [
        "当前目录",
        "当前工作区",
        "当前仓库",
        "这个目录",
        "这个工作区",
        "这个仓库",
        "current directory",
        "current workspace",
        "current repo",
        "current repository",
        "this directory",
        "this workspace",
        "this repo",
        "this repository",
    ]
    .iter()
    .any(|needle| trimmed.contains(needle) || normalized_contains_phrase(&normalized, needle))
}

pub(crate) fn prompt_requests_list_or_table_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    ["markdown table", "table", "list", "列表", "表格"]
        .iter()
        .any(|needle| lower.contains(needle) || prompt.contains(needle))
}

pub(crate) fn prompt_requests_excerpt_kind_judgment(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("更像") || lower.contains("more like")
}

pub(crate) fn prompt_requests_compare_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    ["比较", "对比", "哪个", "compare", "which one"]
        .iter()
        .any(|needle| lower.contains(needle) || prompt.contains(needle))
}

pub(crate) fn prompt_requests_quantity_comparison_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        "更大", "更小", "更长", "更短", "大小", "size", "bigger", "smaller", "larger", "shorter",
        "longer",
    ]
    .iter()
    .any(|needle| lower.contains(needle) || prompt.contains(needle))
}

pub(crate) fn detect_compare_targets_shape(prompt: &str) -> Option<(String, String)> {
    let trimmed = prompt.trim();
    if trimmed.is_empty()
        || !prompt_requests_compare_shape(trimmed)
        || !prompt_requests_quantity_comparison_shape(trimmed)
    {
        return None;
    }
    let mut explicit_paths = trimmed
        .split_whitespace()
        .map(|token| {
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
        })
        .filter(|token| !token.is_empty())
        .filter(|token| crate::worker::has_explicit_path_or_url_locator_hint(token))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    explicit_paths.sort();
    explicit_paths.dedup();
    if explicit_paths.len() == 2 {
        return Some((explicit_paths.remove(0), explicit_paths.remove(0)));
    }
    let mut filenames = crate::delivery_utils::extract_filename_candidates(trimmed);
    filenames.sort();
    filenames.dedup();
    (filenames.len() == 2).then(|| (filenames.remove(0), filenames.remove(0)))
}

#[allow(dead_code)]
pub(crate) fn prompt_requests_workspace_hidden_entries_count_shape(prompt: &str) -> bool {
    prompt_mentions_current_workspace_or_this_directory(prompt)
        && prompt_requests_hidden_entries(prompt)
        && prompt_requests_scalar_count(prompt)
}

#[allow(dead_code)]
pub(crate) fn prompt_requests_workspace_hidden_entries_check_shape(prompt: &str) -> bool {
    prompt_mentions_current_workspace_or_this_directory(prompt)
        && prompt_requests_hidden_entries(prompt)
        && !prompt_requests_workspace_hidden_entries_count_shape(prompt)
}

#[allow(dead_code)]
pub(crate) fn prompt_requests_workspace_dirs_only_listing_shape(
    prompt: &str,
    requested_listing_limit: Option<usize>,
) -> bool {
    prompt_requests_workspace_child_listing(prompt)
        && prompt_mentions_current_workspace_scope(prompt)
        && prompt_requests_directory_only_listing(prompt)
        && requested_listing_limit.is_none()
}

pub(crate) fn extract_workspace_child_directory_hint_shape(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    for marker in ["目录", "folder", "dir"] {
        let Some(idx) = trimmed.find(marker) else {
            continue;
        };
        let mut end = idx;
        while let Some(ch) = trimmed[..end].chars().next_back() {
            if ch.is_whitespace() {
                end -= ch.len_utf8();
            } else {
                break;
            }
        }
        let mut start = end;
        while let Some(ch) = trimmed[..start].chars().next_back() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                start -= ch.len_utf8();
            } else {
                break;
            }
        }
        let token = trimmed[start..end].trim().trim_matches('.');
        if !token.is_empty()
            && token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            return Some(token.to_string());
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    let listing_like = prompt_requests_workspace_child_listing(trimmed)
        || prompt_requests_recent_artifacts_judgment(trimmed);
    if listing_like {
        for marker in ["under ", "inside ", "within ", "in "] {
            let Some(idx) = lower.find(marker) else {
                continue;
            };
            let mut rest = &trimmed[idx + marker.len()..];
            for article in ["the ", "this ", "that ", "current "] {
                if rest.to_ascii_lowercase().starts_with(article) {
                    rest = &rest[article.len()..];
                    break;
                }
            }
            let token: String = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
                .collect();
            if !token.is_empty()
                && token != "current"
                && token != "workspace"
                && token != "directory"
                && token != "folder"
                && token != "dir"
            {
                return Some(token);
            }
        }
    }
    None
}

pub(crate) fn prompt_requests_sqlite_table_listing(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    (lower.contains(".sqlite") || lower.contains(".db"))
        && (prompt.contains("有哪些表")
            || prompt.contains("所有表")
            || prompt.contains("全部表")
            || prompt.contains("什么表")
            || prompt.contains("哪些表")
            || prompt.contains("表名")
            || lower.contains("all tables")
            || lower.contains("list the tables")
            || lower.contains("table names")
            || lower.contains("what tables")
            || lower.contains("which tables"))
}

pub(crate) fn prompt_requests_sqlite_schema_version(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    (lower.contains(".sqlite") || lower.contains(".db")) && lower.contains("schema version")
}

pub(crate) fn prompt_requests_markdown_table_render(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("markdown table")
        || (lower.contains("markdown") && prompt.contains("表格"))
        || prompt.contains("markdown 表格")
        || (prompt.contains("表格") && prompt.contains("markdown"))
}

pub(crate) fn prompt_requests_structured_keys_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        "子键",
        "子字段",
        "顶层键",
        "顶层键名",
        "顶层 key",
        "顶层字段",
        "有哪些键",
        "键名列表",
        "哪些 key",
        "哪些字段",
        "subkey",
        "subkeys",
        "child key",
        "child keys",
        "key names",
        "top-level key",
        "top-level keys",
        "top-level key names",
        "top level key",
        "which keys",
        "what keys",
    ]
    .iter()
    .any(|needle| lower.contains(needle) || prompt.contains(needle))
}

pub(crate) fn prompt_requests_brief_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    [
        "keep it brief",
        "briefly",
        "brief ",
        "short ",
        "shortly",
        "short note",
        "short setup note",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || ["简短", "简要", "简洁", "一段", "brief"]
            .iter()
            .any(|needle| prompt.contains(needle))
}

pub(crate) fn extract_service_status_target_shape(prompt: &str) -> Option<String> {
    const SERVICE_TOKENS: &[&str] = &[
        "telegramd",
        "clawd",
        "whatsappd",
        "whatsapp_webd",
        "feishud",
        "larkd",
        "sshd",
        "ssh",
    ];
    let lower = prompt.to_ascii_lowercase();
    for token in
        lower.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
    {
        if SERVICE_TOKENS.iter().any(|candidate| token == *candidate) {
            return Some(token.to_string());
        }
    }
    None
}

pub(crate) fn prompt_requests_inline_transform_action_shape(prompt: &str) -> bool {
    let lower = prompt.trim().to_ascii_lowercase();
    ["sort", "markdown", "table", "render", "convert"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub(crate) fn prompt_requests_delivery_phrase(prompt: &str) -> bool {
    let lower = prompt.trim().to_ascii_lowercase();
    [
        "send me",
        "send it",
        "deliver",
        "attach",
        "upload",
        "as a file",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || ["发给我", "发我", "直接发文件", "别贴正文", "作为文件"]
            .iter()
            .any(|needle| prompt.contains(needle))
}

pub(crate) fn prompt_references_deictic_object(prompt: &str) -> bool {
    let lower = prompt.trim().to_ascii_lowercase();
    let has_en_deictic = lower
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .any(|token| matches!(token, "this" | "it"));
    has_en_deictic
        || ["这个", "那个", "它", "该文件"]
            .iter()
            .any(|needle| prompt.contains(needle))
}

pub(crate) fn prompt_mentions_generic_file_object(prompt: &str) -> bool {
    let scrubbed = strip_delivery_tokens_for_phrase_match(prompt);
    let lower = scrubbed.trim().to_ascii_lowercase();
    lower.contains(" file")
        || lower.starts_with("file ")
        || lower.contains(" document")
        || lower.starts_with("document ")
        || ["文件", "文档", "配置", "配置文件", "说明文档"]
            .iter()
            .any(|needle| scrubbed.contains(needle))
}

pub(crate) fn prompt_mentions_fileish_reference_shape(prompt: &str) -> bool {
    let scrubbed = strip_delivery_tokens_for_phrase_match(prompt);
    let lower = scrubbed.trim().to_ascii_lowercase();
    [
        "日志", "脚本", "目录", "服务", "readme", "log", "config", "script", "report", "service",
        "folder", "dir",
    ]
    .iter()
    .any(|needle| lower.contains(needle) || scrubbed.contains(needle))
}

fn strip_delivery_tokens_for_phrase_match(prompt: &str) -> String {
    prompt
        .split_whitespace()
        .filter(|token| {
            let trimmed = token.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | '，' | ';' | '；' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            crate::finalize::parse_delivery_token(trimmed).is_none()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn prompt_requests_short_joke_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("笑话") || lower.contains("joke")
}

pub(crate) fn prompt_requests_weather_lookup_suppression_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("别查")
        || prompt.contains("不要查")
        || prompt.contains("不用查")
        || lower.contains("don't check")
        || lower.contains("do not check")
        || lower.contains("no need to check")
}

pub(crate) fn prompt_mentions_weather_lookup_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("天气") || lower.contains("weather")
}

pub(crate) fn prompt_mentions_file_write_tooling_shape(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    prompt.contains("文件")
        || prompt.contains("保存")
        || prompt.contains("写入")
        || lower.contains("save")
        || lower.contains("write to")
        || lower.contains("file")
}

pub(crate) fn prompt_requests_scalar_only_shape(prompt: &str) -> bool {
    if prompt_requests_list_or_table_shape(prompt) {
        return false;
    }
    let lower = prompt.to_ascii_lowercase();
    [
        "output only",
        "return only",
        "just the value",
        "nothing else",
        "don't add anything else",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || [
            "只输出",
            "只返回",
            "只给结果",
            "只回答",
            "只回",
            "只输出值",
            "别补别的",
            "别加别的",
        ]
        .iter()
        .any(|needle| prompt.contains(needle))
        || (prompt.contains("只") && prompt.contains("值") && prompt.contains("给我"))
}

fn trim_sentence_count_token(token: &str) -> &str {
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

fn parse_small_sentence_count_token(token: &str) -> Option<usize> {
    let trimmed = trim_sentence_count_token(token);
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<usize>().ok().or_else(|| match trimmed {
        "one" | "a" | "an" | "一" => Some(1),
        "two" | "二" | "两" => Some(2),
        "three" | "三" => Some(3),
        _ => None,
    })
}

fn parse_count_before_sentence_suffix(token: &str) -> Option<usize> {
    let trimmed = trim_sentence_count_token(token);
    for suffix in ["sentences", "sentence", "句话", "句"] {
        let Some(prefix) = trimmed.strip_suffix(suffix) else {
            continue;
        };
        let prefix = prefix.trim();
        if prefix.is_empty() {
            continue;
        }
        if let Some(value) = parse_small_sentence_count_token(prefix) {
            return Some(value);
        }
    }
    None
}

pub(crate) fn requested_sentence_count_shape(prompt: &str) -> Option<usize> {
    let lower = prompt.to_ascii_lowercase();
    let words = lower
        .split_whitespace()
        .map(trim_sentence_count_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for window in words.windows(2) {
        let [count_token, unit_token] = window else {
            continue;
        };
        if *unit_token != "sentence" && *unit_token != "sentences" {
            continue;
        }
        if let Some(value) = parse_small_sentence_count_token(count_token) {
            return Some(value);
        }
    }
    for token in prompt.split_whitespace() {
        if let Some(value) = parse_count_before_sentence_suffix(token) {
            return Some(value);
        }
    }
    let compact = prompt
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if let Some(value) = parse_count_before_sentence_suffix(&compact) {
        return Some(value);
    }
    let compact_lower = lower
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    for (needle, value) in [
        ("1sentence", 1),
        ("onesentence", 1),
        ("singlesentence", 1),
        ("2sentences", 2),
        ("twosentences", 2),
        ("3sentences", 3),
        ("threesentences", 3),
    ] {
        if compact_lower.contains(needle) {
            return Some(value);
        }
    }
    for (needle, value) in [
        ("一句话", 1),
        ("一句大白话", 1),
        ("一大白话", 1),
        ("两句话", 2),
        ("二句话", 2),
        ("2句话", 2),
        ("三句话", 3),
        ("3句话", 3),
    ] {
        if compact.contains(needle) {
            return Some(value);
        }
    }
    None
}

pub(crate) fn prompt_contains_delivery_token_reference(prompt: &str) -> bool {
    if !crate::extract_delivery_file_tokens(prompt).is_empty() {
        return true;
    }
    prompt.split_whitespace().any(|token| {
        let trimmed = token.trim_matches(|c: char| {
            matches!(
                c,
                ',' | '，' | ';' | '；' | ':' | '：' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        });
        crate::finalize::parse_delivery_token(trimmed).is_some()
    })
}

fn normalize_field_selector_token(token: &str, allow_single_segment: bool) -> Option<String> {
    let trimmed = token.trim_matches(|c: char| {
        !c.is_alphanumeric() && c != '.' && c != '_' && c != '-' && c != '$'
    });
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    let mut parts = trimmed.split('.');
    let first = parts.next()?;
    if first.is_empty()
        || !first
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
    {
        return None;
    }
    let mut saw_dot_segment = false;
    for part in parts {
        if part.is_empty()
            || !part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '$'))
        {
            return None;
        }
        saw_dot_segment = true;
    }
    if !allow_single_segment && !saw_dot_segment {
        return None;
    }
    Some(trimmed.to_string())
}

fn push_unique_selector(selectors: &mut Vec<String>, selector: String) {
    if !selectors
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&selector))
    {
        selectors.push(selector);
    }
}

fn split_selector_candidate_tokens<'a>(prompt: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    prompt.split_whitespace().flat_map(|token| {
        token.split(|ch: char| {
            matches!(
                ch,
                ',' | '，'
                    | '。'
                    | ';'
                    | '；'
                    | ':'
                    | '：'
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
    })
}

fn extract_embedded_path_basename_candidates(prompt: &str) -> Vec<String> {
    prompt
        .split_whitespace()
        .filter_map(|token| {
            let trimmed = token.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | '，'
                        | '。'
                        | ';'
                        | '；'
                        | ':'
                        | '：'
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
                        | '"'
                        | '\''
                        | '`'
                )
            });
            if !(trimmed.contains('/') || trimmed.contains('\\')) {
                return None;
            }
            std::path::Path::new(trimmed)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_ascii_lowercase())
        })
        .collect()
}

fn selector_before_marker(prompt: &str, marker: &str) -> Option<String> {
    let idx = prompt.find(marker)?;
    let mut end = idx;
    while let Some(ch) = prompt[..end].chars().next_back() {
        if ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’') {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    let mut start = end;
    while let Some(ch) = prompt[..start].chars().next_back() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '$') {
            start -= ch.len_utf8();
        } else {
            break;
        }
    }
    (start < end)
        .then(|| normalize_field_selector_token(&prompt[start..end], true))
        .flatten()
}

fn extract_single_segment_field_after_locator_segment(
    prompt: &str,
    filename_candidates: &[String],
) -> Option<String> {
    let lower = prompt.to_ascii_lowercase();
    let locator = filename_candidates
        .iter()
        .find(|candidate| lower.contains(candidate.as_str()))
        .cloned()?;
    let locator_start = lower.find(&locator)?;
    let locator_end = locator_start + locator.len();
    let after_locator = prompt.get(locator_end..)?.trim_start();
    if after_locator.is_empty() {
        return None;
    }
    let segment_end = after_locator
        .find(|ch: char| {
            matches!(
                ch,
                ',' | '，' | ';' | '；' | '?' | '？' | '!' | '！' | '\n' | '\r'
            )
        })
        .unwrap_or(after_locator.len());
    let segment = after_locator[..segment_end].trim();
    if segment.is_empty() {
        return None;
    }
    let mut identifiers = segment
        .split_whitespace()
        .filter_map(|token| normalize_field_selector_token(token, true))
        .filter(|identifier| {
            !filename_candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(identifier))
        })
        .collect::<Vec<_>>();
    identifiers.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if identifiers.len() == 1 {
        return Some(identifiers.remove(0));
    }
    field_before_value_marker(segment, filename_candidates)
}

fn field_before_value_marker(segment: &str, filename_candidates: &[String]) -> Option<String> {
    let raw_tokens = segment
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '$' && c != '.'
            })
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for window in raw_tokens.windows(2) {
        let marker = window[1].to_ascii_lowercase();
        if marker != "value" && marker != "values" {
            continue;
        }
        let candidate = normalize_field_selector_token(window[0], true)?;
        if filename_candidates
            .iter()
            .any(|filename| filename.eq_ignore_ascii_case(&candidate))
        {
            continue;
        }
        if matches!(
            candidate.to_ascii_lowercase().as_str(),
            "and" | "or" | "the" | "a" | "an" | "only" | "just" | "return" | "output"
        ) {
            continue;
        }
        return Some(candidate);
    }
    None
}

pub(crate) fn extract_dotted_field_selector(prompt: &str) -> Option<String> {
    let filename_candidates = crate::delivery_utils::extract_filename_candidates(prompt)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .chain(extract_embedded_path_basename_candidates(prompt))
        .collect::<Vec<_>>();
    split_selector_candidate_tokens(prompt).find_map(|token| {
        let selector = normalize_field_selector_token(token, false)?;
        let looks_like_filename_candidate = filename_candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&selector));
        if looks_like_filename_candidate
            && !filename_like_dotted_selector_has_context(prompt, &selector, &filename_candidates)
        {
            return None;
        }
        Some(selector)
    })
}

fn filename_like_dotted_selector_has_context(
    prompt: &str,
    selector: &str,
    filename_candidates: &[String],
) -> bool {
    let lower_prompt = prompt.to_ascii_lowercase();
    let selector_lower = selector.to_ascii_lowercase();
    let Some(selector_idx) = lower_prompt.find(&selector_lower) else {
        return false;
    };

    if filename_candidates.iter().any(|candidate| {
        !candidate.eq_ignore_ascii_case(&selector_lower)
            && lower_prompt
                .find(candidate)
                .is_some_and(|candidate_idx| candidate_idx < selector_idx)
    }) {
        return true;
    }

    let prefix = &prompt[..selector_idx];
    let suffix = &prompt[selector_idx + selector.len()..];
    let trimmed_suffix = suffix.trim_start_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，' | '。' | ';' | '；' | ':' | '：' | '(' | ')' | '（' | '）'
            )
    });
    prefix.trim_end().ends_with('的')
        || prefix.to_ascii_lowercase().ends_with(" of ")
        || trimmed_suffix.starts_with("字段")
        || trimmed_suffix.starts_with("值")
        || trimmed_suffix.to_ascii_lowercase().starts_with("field")
        || trimmed_suffix.to_ascii_lowercase().starts_with("value")
}

pub(crate) fn extract_field_selector_mentions(prompt: &str) -> Vec<String> {
    let mut selectors = Vec::new();
    let filename_candidates = crate::delivery_utils::extract_filename_candidates(prompt)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .chain(extract_embedded_path_basename_candidates(prompt))
        .collect::<Vec<_>>();
    for token in split_selector_candidate_tokens(prompt) {
        if let Some(selector) = normalize_field_selector_token(token, false) {
            if !filename_candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&selector))
            {
                push_unique_selector(&mut selectors, selector);
            }
        }
    }
    for marker in ["字段", "field"] {
        if let Some(selector) = selector_before_marker(prompt, marker) {
            if !filename_candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&selector))
            {
                push_unique_selector(&mut selectors, selector);
            }
        }
    }
    if selectors.is_empty() {
        if let Some(selector) =
            extract_single_segment_field_after_locator_segment(prompt, &filename_candidates)
        {
            push_unique_selector(&mut selectors, selector);
        }
    }
    selectors
}

pub(crate) fn extract_workspace_existing_single_token_hint(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty()
        || trimmed.split_whitespace().count() != 1
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('.')
    {
        return None;
    }
    workspace_root_for_surface_signals()
        .join(trimmed)
        .try_exists()
        .ok()
        .filter(|exists| *exists)
        .map(|_| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        analyze_prompt_surface, extract_dotted_field_selector, extract_field_selector_mentions,
        extract_workspace_child_directory_hint_shape, extract_workspace_existing_single_token_hint,
        prompt_contains_deictic_reference_shape, prompt_contains_delivery_token_reference,
        prompt_mentions_current_workspace_scope_reference_shape,
        prompt_mentions_fileish_reference_shape, prompt_mentions_generic_file_object,
        prompt_mentions_weather_lookup_shape, prompt_references_deictic_object,
        prompt_requests_brief_shape, prompt_requests_compare_shape,
        prompt_requests_current_workspace_path_scalar, prompt_requests_delivery_phrase,
        prompt_requests_directory_purpose_summary, prompt_requests_excerpt_kind_judgment,
        prompt_requests_git_dirty_summary, prompt_requests_inline_transform_action_shape,
        prompt_requests_list_or_table_shape, prompt_requests_markdown_table_render,
        prompt_requests_package_manager_detection, prompt_requests_quantity_comparison_shape,
        prompt_requests_scalar_only_shape, prompt_requests_sqlite_schema_version,
        prompt_requests_sqlite_table_listing, prompt_requests_structured_keys_shape,
        prompt_requests_toml_path_listing, prompt_requests_workspace_project_summary,
        requested_listing_limit_from_prompt_pair, requested_read_range_from_prompt_pair,
        requested_sentence_count_shape, DeicticPromptShape, DeliveryPromptShape,
        FieldReadPromptShape, FileReferencePromptShape, InlineJsonShape,
        InlineTransformPromptShape, LocatorHintPromptShape, LocatorReplyPromptShape,
        OutputCompressionShape, OutputRequestShape, PathOutputPromptShape,
        PromptSemanticRequestShape, ShortJokePromptShape, TableRequestShape,
        WorkspaceChildRequestShape, WorkspaceRootRequestShape, WorkspaceScopePromptShape,
    };

    #[test]
    fn detects_empty_prompt_as_default_signals() {
        let signals = analyze_prompt_surface("   ");
        assert_eq!(signals.token_count, 0);
        assert!(signals.inline_json_shape.is_none());
        assert!(signals.locator_hint_prompt_shape.is_none());
        assert!(signals.locator_reply_prompt_shape.is_none());
        assert!(!signals.has_explicit_path_or_url());
        assert!(!signals.has_concrete_locator_hint());
        assert!(!signals.looks_like_locator_only_reply());
        assert_eq!(signals.field_selector_count, 0);
        assert_eq!(signals.filename_candidate_count, 0);
        assert_eq!(signals.bare_filename_stem_candidate_count, 0);
        assert!(signals.workspace_single_token_hint.is_none());
        assert!(signals.output_request_shape.is_none());
        assert!(signals.output_compression_shape.is_none());
        assert!(signals.inline_transform_prompt_shape.is_none());
        assert!(signals.delivery_prompt_shape.is_none());
        assert!(signals.file_reference_prompt_shape.is_none());
        assert!(signals.deictic_prompt_shape.is_none());
        assert!(signals.workspace_scope_prompt_shape.is_none());
        assert!(signals.workspace_root_request_shape.is_none());
        assert!(signals.workspace_child_request_shape.is_none());
        assert!(signals.table_request_shape.is_none());
        assert!(signals.semantic_request_shape.is_none());
        assert!(signals.short_joke_prompt_shape.is_none());
    }

    #[test]
    fn detects_inline_json_and_locator_shape() {
        let signals = analyze_prompt_surface("{\"path\":\"logs/clawd.log\"}");
        assert_eq!(signals.inline_json_shape, Some(InlineJsonShape::WholeValue));
        assert!(signals.has_concrete_locator_hint());
    }

    #[test]
    fn detects_explicit_path_locator() {
        let signals = analyze_prompt_surface("读取 UI/package.json 里的 name 字段，只输出值");
        assert_eq!(
            signals.locator_hint_prompt_shape,
            Some(LocatorHintPromptShape::ExplicitPathOrUrl)
        );
        assert!(signals.has_explicit_path_or_url());
        assert!(signals.has_concrete_locator_hint());
        assert_eq!(signals.field_selector_count, 1);
        assert!(signals.filename_candidate_count >= 1);
    }

    #[test]
    fn detects_locator_only_reply_shape() {
        let signals = analyze_prompt_surface("logs/model_io.log");
        assert_eq!(
            signals.locator_hint_prompt_shape,
            Some(LocatorHintPromptShape::ExplicitPathOrUrl)
        );
        assert_eq!(
            signals.locator_reply_prompt_shape,
            Some(LocatorReplyPromptShape::LocatorOnly)
        );
        assert!(signals.has_explicit_path_or_url());
        assert!(signals.looks_like_locator_only_reply());
    }

    #[test]
    fn requested_read_range_from_prompt_pair_prefers_primary_then_fallback() {
        assert_eq!(
            requested_read_range_from_prompt_pair(
                Some("把 README.md 开头读 10 行"),
                "把 README.md 开头读 10 行"
            ),
            Some(crate::read_range_request::RequestedReadRange::Head { n: 10 })
        );
        assert_eq!(
            requested_read_range_from_prompt_pair(
                Some("继续刚才那个请求"),
                "把 README.md 开头读 10 行"
            ),
            Some(crate::read_range_request::RequestedReadRange::Head { n: 10 })
        );
    }

    #[test]
    fn requested_listing_limit_from_prompt_pair_prefers_primary_then_fallback() {
        assert_eq!(
            requested_listing_limit_from_prompt_pair(
                Some("列出 logs 目录最近修改的 3 个文件"),
                "列出 logs 目录最近修改的 3 个文件"
            ),
            Some(3)
        );
        assert_eq!(
            requested_listing_limit_from_prompt_pair(
                Some("继续刚才那个请求"),
                "列出 logs 目录最近修改的 3 个文件"
            ),
            Some(3)
        );
    }

    #[test]
    fn detects_embedded_json_payload() {
        let signals = analyze_prompt_surface(
            r#"sort this JSON array by score descending: [{"name":"alpha","score":7}]"#,
        );
        assert_eq!(
            signals.inline_json_shape,
            Some(InlineJsonShape::EmbeddedPayload)
        );
    }

    #[test]
    fn extracts_dotted_field_selector_from_mixed_prompt() {
        let out = extract_dotted_field_selector(
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
        )
        .expect("should find dotted field selector");
        assert_eq!(out, "tools.allow_sudo");
    }

    #[test]
    fn ignores_path_tokens_when_extracting_dotted_field_selector() {
        let out = extract_dotted_field_selector("读取 /tmp/config.toml 只输出值");
        assert!(out.is_none());
    }

    #[test]
    fn ignores_filename_tokens_when_extracting_dotted_field_selector() {
        let out = extract_dotted_field_selector("restart_clawd_latest.sh");
        assert!(out.is_none());
    }

    #[test]
    fn keeps_filename_like_selector_when_field_context_is_present() {
        let out = extract_dotted_field_selector("读取 Cargo.toml 的 package.name，只输出值");
        assert_eq!(out.as_deref(), Some("package.name"));
    }

    #[test]
    fn extracts_bare_field_selector_before_field_marker() {
        let out = extract_field_selector_mentions(
            "读 scripts/nl_tests/fixtures/device_local/package.json，告诉我 scripts 字段下都有哪些子键",
        );
        assert_eq!(out, vec!["scripts".to_string()]);
    }

    #[test]
    fn extracts_multiple_field_selectors_in_order() {
        let out = extract_field_selector_mentions(
            "读取 /tmp/config.toml 里的 database.sqlite_path 和 tools.allow_sudo，告诉我两个字段的值",
        );
        assert_eq!(
            out,
            vec![
                "database.sqlite_path".to_string(),
                "tools.allow_sudo".to_string()
            ]
        );
    }

    #[test]
    fn extracts_single_segment_field_after_locator_segment() {
        let out = extract_field_selector_mentions("去 package.json 里找 name，只把值给我");
        assert_eq!(out, vec!["name".to_string()]);
    }

    #[test]
    fn extracts_single_segment_field_from_value_phrase_after_locator_segment() {
        let out =
            extract_field_selector_mentions("go into package.json and return only the name value");
        assert_eq!(out, vec!["name".to_string()]);
    }

    #[test]
    fn detects_workspace_existing_single_token_hint() {
        assert_eq!(
            extract_workspace_existing_single_token_hint("logs").as_deref(),
            Some("logs")
        );
        assert!(extract_workspace_existing_single_token_hint("git").is_none());
    }

    #[test]
    fn detects_delivery_token_reference_shape() {
        assert!(prompt_contains_delivery_token_reference(
            "再发一次 FILE:/tmp/example.txt"
        ));
        let signals = analyze_prompt_surface("再发一次 FILE:/tmp/example.txt");
        assert_eq!(
            signals.file_reference_prompt_shape,
            Some(FileReferencePromptShape::DeliveryToken)
        );
    }

    #[test]
    fn lifts_phrase_fallbacks_into_surface_signal_flags() {
        let signals = analyze_prompt_surface("把这个文件发给我，只输出值，简短说明一下");
        assert_eq!(
            signals.delivery_prompt_shape,
            Some(DeliveryPromptShape::PhraseWithTarget)
        );
        assert_eq!(
            signals.file_reference_prompt_shape,
            Some(FileReferencePromptShape::GenericObject)
        );
        assert_eq!(
            signals.deictic_prompt_shape,
            Some(DeicticPromptShape::ObjectTarget)
        );
        assert_eq!(
            signals.output_compression_shape,
            Some(OutputCompressionShape::ScalarOnly)
        );
    }

    #[test]
    fn lifts_router_deictic_shape_flags_into_surface_signals() {
        let status = analyze_prompt_surface("那个服务现在在运行吗");
        assert_eq!(
            status.semantic_request_shape,
            Some(PromptSemanticRequestShape::ServiceStatusQuestion)
        );
        let count = analyze_prompt_surface("那个目录里有多少个文件");
        assert_eq!(
            count.semantic_request_shape,
            Some(PromptSemanticRequestShape::ScalarCount)
        );
        let lookup = analyze_prompt_surface("那个目录下面有什么");
        assert_eq!(
            lookup.semantic_request_shape,
            Some(PromptSemanticRequestShape::DirectoryLookup)
        );
        let existence = analyze_prompt_surface("那个文件还在不在");
        assert_eq!(
            existence.semantic_request_shape,
            Some(PromptSemanticRequestShape::Existence)
        );
    }

    #[test]
    fn lifts_workspace_root_request_shapes_into_surface_signals() {
        let project = analyze_prompt_surface("用一句话说明当前工作区像什么项目");
        assert_eq!(
            project.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::ProjectSummary)
        );
        let toml = analyze_prompt_surface("列出当前仓库里的 toml 文件路径列表，只输出路径");
        assert_eq!(
            toml.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::TomlPathListing)
        );
        let hidden_count =
            analyze_prompt_surface("数一下当前目录里以点开头的隐藏文件有几个，只输出数字");
        assert_eq!(
            hidden_count.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::HiddenEntriesCount)
        );
        let hidden_check =
            analyze_prompt_surface("这个目录里有没有那种点开头的隐藏文件？有的话随便举两个例子");
        assert_eq!(
            hidden_check.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::HiddenEntriesCheck)
        );
        let dirs = analyze_prompt_surface("列出当前目录有哪些顶层文件夹，只输出目录名列表");
        assert_eq!(
            dirs.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::DirsOnlyListing)
        );
        let path = analyze_prompt_surface("只输出当前工作目录的绝对路径，不要解释");
        assert_eq!(
            path.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::CurrentPathScalar)
        );
        let pkg = analyze_prompt_surface("用一句话说当前机器的包管理器是什么");
        assert_eq!(
            pkg.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::PackageManagerDetection)
        );
        let dirty = analyze_prompt_surface("看看当前仓库有没有改动");
        assert_eq!(
            dirty.workspace_root_request_shape,
            Some(WorkspaceRootRequestShape::GitDirtySummary)
        );
    }

    #[test]
    fn lifts_workspace_scope_prompt_shape_into_surface_signals() {
        let explicit = analyze_prompt_surface("看看当前目录");
        assert_eq!(
            explicit.workspace_scope_prompt_shape,
            Some(WorkspaceScopePromptShape::ExplicitAndReference)
        );
        let reference = analyze_prompt_surface("看看这个目录");
        assert_eq!(
            reference.workspace_scope_prompt_shape,
            Some(WorkspaceScopePromptShape::ReferenceScope)
        );
    }

    #[test]
    fn lifts_output_request_shape_into_surface_signals() {
        let list = analyze_prompt_surface("把结果输出成 markdown table");
        assert_eq!(
            list.output_request_shape,
            Some(OutputRequestShape::ListOrTable)
        );
        let compare = analyze_prompt_surface("比较 Cargo.toml 和 Cargo.lock 哪个更大");
        assert_eq!(
            compare.output_request_shape,
            Some(OutputRequestShape::Compare)
        );
        let keys = analyze_prompt_surface("读取 configs/config.toml 的顶层键名，只输出键名列表");
        assert_eq!(
            keys.output_request_shape,
            Some(OutputRequestShape::StructuredKeys)
        );
        let judgment = analyze_prompt_surface("一句话说它更像日志还是清单");
        assert_eq!(
            judgment.output_request_shape,
            Some(OutputRequestShape::ExcerptKindJudgment)
        );
    }

    #[test]
    fn lifts_table_request_shape_into_surface_signals() {
        let sqlite = analyze_prompt_surface("看看 data/db-basic-contract.sqlite 里有哪些表");
        assert_eq!(
            sqlite.table_request_shape,
            Some(TableRequestShape::SqliteTableListing)
        );
        let table_names =
            analyze_prompt_surface("看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名");
        assert_eq!(
            table_names.table_request_shape,
            Some(TableRequestShape::SqliteTableListing)
        );
        let markdown = analyze_prompt_surface("把结果输出成 markdown table");
        assert_eq!(
            markdown.table_request_shape,
            Some(TableRequestShape::MarkdownRender)
        );
    }

    #[test]
    fn lifts_workspace_child_request_shapes_into_surface_signals() {
        let list = analyze_prompt_surface("列出 document 目录下有哪些文件，只输出文件名列表");
        assert_eq!(
            list.workspace_child_request_shape,
            Some(WorkspaceChildRequestShape::Listing)
        );
        let recent = analyze_prompt_surface(
            "列出 logs 目录最近修改的 3 个文件，再告诉我这更像测试日志还是正式产物",
        );
        assert_eq!(
            recent.workspace_child_request_shape,
            Some(WorkspaceChildRequestShape::RecentArtifactsJudgment)
        );
        let purpose = analyze_prompt_surface(
            "列出 logs 目录最近修改的 3 个文件，再告诉我这更像说明文档还是运行产物",
        );
        assert_eq!(
            purpose.workspace_child_request_shape,
            Some(WorkspaceChildRequestShape::DirectoryPurposeSummary)
        );
    }

    #[test]
    fn lifts_short_joke_prompt_shape_into_surface_signals() {
        let eligible = analyze_prompt_surface("讲个笑话");
        assert_eq!(
            eligible.short_joke_prompt_shape,
            Some(ShortJokePromptShape::Eligible)
        );
        let weather_blocked = analyze_prompt_surface("讲个天气笑话");
        assert_eq!(
            weather_blocked.short_joke_prompt_shape,
            Some(ShortJokePromptShape::WeatherLookupBlocked)
        );
        let weather_suppressed = analyze_prompt_surface("讲个天气笑话，但别查天气");
        assert_eq!(
            weather_suppressed.short_joke_prompt_shape,
            Some(ShortJokePromptShape::Eligible)
        );
        let file_blocked = analyze_prompt_surface("写个保存到文件里的笑话");
        assert_eq!(
            file_blocked.short_joke_prompt_shape,
            Some(ShortJokePromptShape::FileToolingBlocked)
        );
    }

    #[test]
    fn detects_workspace_project_summary_shape() {
        assert!(prompt_requests_workspace_project_summary(
            "别细看，先整体扫一眼当前工作区，然后用一句适合新手的话告诉我这里像是什么项目"
        ));
    }

    #[test]
    fn detects_workspace_project_summary_shape_for_compact_one_sentence_variant() {
        assert!(prompt_requests_workspace_project_summary(
            "用一句话说明当前工作区像什么项目"
        ));
    }

    #[test]
    fn detects_toml_path_listing_shape() {
        assert!(prompt_requests_toml_path_listing(
            "列出当前仓库里的 toml 文件路径列表，只输出路径"
        ));
    }

    #[test]
    fn detects_directory_purpose_summary_shape() {
        assert!(prompt_requests_directory_purpose_summary(
            "列出 logs 目录最近修改的 3 个文件，再告诉我这更像说明文档还是运行产物"
        ));
    }

    #[test]
    fn detects_package_manager_detection_shape() {
        assert!(prompt_requests_package_manager_detection(
            "用一句话说当前机器的包管理器是什么"
        ));
    }

    #[test]
    fn detects_git_dirty_summary_shape() {
        assert!(prompt_requests_git_dirty_summary("看看当前仓库有没有改动"));
        assert!(prompt_requests_git_dirty_summary(
            "tell me whether this repo has uncommitted changes"
        ));
    }

    #[test]
    fn detects_current_workspace_path_scalar_shape() {
        assert!(prompt_requests_current_workspace_path_scalar(
            "只输出当前工作目录的绝对路径，不要解释"
        ));
    }

    #[test]
    fn detects_excerpt_kind_judgment_shape() {
        assert!(prompt_requests_excerpt_kind_judgment(
            "一句话说它更像日志还是清单"
        ));
    }

    #[test]
    fn detects_sqlite_table_listing_shape() {
        assert!(prompt_requests_sqlite_table_listing(
            "看看 data/db-basic-contract.sqlite 里有哪些表"
        ));
        assert!(prompt_requests_sqlite_table_listing(
            "查看 data/db-basic-contract.sqlite 中的所有表"
        ));
        assert!(prompt_requests_sqlite_table_listing(
            "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
        ));
        assert!(prompt_requests_sqlite_table_listing(
            "list all tables in data/db-basic-contract.sqlite"
        ));
    }

    #[test]
    fn lifts_directory_file_pair_into_surface_signals() {
        let explicit = analyze_prompt_surface(
            "去 scripts/nl_tests/fixtures/locator_smart/case_only 找 report.md，只输出路径",
        );
        assert_eq!(
            explicit.directory_file_pair,
            Some((
                "scripts/nl_tests/fixtures/locator_smart/case_only".to_string(),
                "report.md".to_string()
            ))
        );

        let stem = analyze_prompt_surface(
            "去 scripts/nl_tests/fixtures/locator_smart/stem_unique 找 abcd，只输出路径",
        );
        assert_eq!(
            stem.directory_file_pair,
            Some((
                "scripts/nl_tests/fixtures/locator_smart/stem_unique".to_string(),
                "abcd".to_string()
            ))
        );
    }

    #[test]
    fn detects_path_output_prompt_shape_for_where_is_variant() {
        let surface = analyze_prompt_surface(
            "在 scripts/nl_tests/fixtures/locator_smart/case_only 里查一下 report.md 在哪，路径就行",
        );
        assert_eq!(
            surface.path_output_prompt_shape,
            Some(PathOutputPromptShape::ScalarOnly)
        );
    }

    #[test]
    fn lifts_english_directory_file_pair_into_surface_signals() {
        let explicit = analyze_prompt_surface(
            "In scripts/nl_tests/fixtures/locator_smart/case_only, where is report.md? just output the path",
        );
        assert_eq!(
            explicit.directory_file_pair,
            Some((
                "scripts/nl_tests/fixtures/locator_smart/case_only".to_string(),
                "report.md".to_string()
            ))
        );

        let stem = analyze_prompt_surface(
            "In scripts/nl_tests/fixtures/locator_smart/stem_unique, where is abcd? just the path",
        );
        assert_eq!(
            stem.directory_file_pair,
            Some((
                "scripts/nl_tests/fixtures/locator_smart/stem_unique".to_string(),
                "abcd".to_string()
            ))
        );
    }

    #[test]
    fn detects_path_output_prompt_shape_for_english_where_is_variant() {
        let surface = analyze_prompt_surface(
            "In scripts/nl_tests/fixtures/locator_smart/case_only, where is report.md? just output the path",
        );
        assert_eq!(
            surface.path_output_prompt_shape,
            Some(PathOutputPromptShape::ScalarOnly)
        );
    }

    #[test]
    fn detects_path_output_prompt_shape_for_english_path_only_variants() {
        let contracted = analyze_prompt_surface(
            "In scripts/nl_tests/fixtures/locator_smart/case_only, where's report.md? only the path",
        );
        assert_eq!(
            contracted.path_output_prompt_shape,
            Some(PathOutputPromptShape::ScalarOnly)
        );

        let imperative = analyze_prompt_surface(
            "Inside scripts/nl_tests/fixtures/locator_smart/stem_unique, find abcd and return only the path",
        );
        assert_eq!(
            imperative.path_output_prompt_shape,
            Some(PathOutputPromptShape::ScalarOnly)
        );
    }

    #[test]
    fn detects_sqlite_schema_version_shape() {
        assert!(prompt_requests_sqlite_schema_version(
            "看一下 data/db-basic-contract.sqlite 的 schema version，只输出数字"
        ));
        let sqlite = analyze_prompt_surface(
            "看一下 data/db-basic-contract.sqlite 的 schema version，只输出数字",
        );
        assert_eq!(
            sqlite.table_request_shape,
            Some(TableRequestShape::SqliteSchemaVersion)
        );
    }

    #[test]
    fn detects_markdown_table_render_shape() {
        assert!(prompt_requests_markdown_table_render(
            "把这个 JSON 数组按 score 从高到低排一下，再输出成 markdown 表格"
        ));
    }

    #[test]
    fn detects_list_or_table_shape() {
        assert!(prompt_requests_list_or_table_shape(
            "把结果输出成 markdown table"
        ));
    }

    #[test]
    fn detects_compare_shape() {
        assert!(prompt_requests_compare_shape(
            "比较 Cargo.toml 和 Cargo.lock 哪个更大"
        ));
    }

    #[test]
    fn detects_quantity_comparison_shape() {
        assert!(prompt_requests_quantity_comparison_shape(
            "比较 Cargo.toml 和 Cargo.lock 哪个更大"
        ));
    }

    #[test]
    fn lifts_compare_targets_into_surface_signals() {
        let signals = analyze_prompt_surface("比较 Cargo.toml 和 Cargo.lock 哪个更大");
        assert_eq!(
            signals.compare_target_pair,
            Some(("Cargo.lock".to_string(), "Cargo.toml".to_string()))
        );
    }

    #[test]
    fn detects_structured_keys_shape() {
        assert!(prompt_requests_structured_keys_shape(
            "读取 configs/config.toml 的顶层键名，只输出键名列表"
        ));
    }

    #[test]
    fn detects_brief_shape() {
        assert!(prompt_requests_brief_shape("briefly explain this"));
        assert!(prompt_requests_brief_shape(
            "Write a short RustClaw setup note"
        ));
        assert!(prompt_requests_brief_shape("简短说明一下"));
        assert!(prompt_requests_brief_shape("帮我写一段 RustClaw 安装说明"));
    }

    #[test]
    fn detects_exact_sentence_count_shape() {
        assert_eq!(
            requested_sentence_count_shape("explain this in 1 sentence"),
            Some(1)
        );
        assert_eq!(
            requested_sentence_count_shape("用一句话说明这个项目"),
            Some(1)
        );
        assert_eq!(
            requested_sentence_count_shape("summarize in 3 sentences"),
            Some(3)
        );
        let signals = analyze_prompt_surface("用一句话说明这个项目");
        assert_eq!(signals.requested_sentence_count, Some(1));
    }

    #[test]
    fn detects_fileish_reference_shape() {
        assert!(prompt_mentions_fileish_reference_shape("把那个日志发给我"));
        assert!(prompt_mentions_fileish_reference_shape(
            "show me that script"
        ));
        assert!(prompt_mentions_fileish_reference_shape("use this folder"));
    }

    #[test]
    fn extracts_workspace_child_directory_hint_shape() {
        assert_eq!(
            extract_workspace_child_directory_hint_shape("列出 logs 目录最近修改的 3 个文件")
                .as_deref(),
            Some("logs")
        );
        assert_eq!(
            extract_workspace_child_directory_hint_shape("show me files in document folder")
                .as_deref(),
            Some("document")
        );
        assert_eq!(
            extract_workspace_child_directory_hint_shape(
                "list the 2 most recently modified files under logs and output only the file names"
            )
            .as_deref(),
            Some("logs")
        );
    }

    #[test]
    fn lifts_requested_listing_limit_into_surface_signals() {
        let signals = analyze_prompt_surface("列出 logs 目录最近修改的 3 个文件");
        assert_eq!(signals.requested_listing_limit, Some(3));
        assert_eq!(
            signals.workspace_child_directory_hint.as_deref(),
            Some("logs")
        );
    }

    #[test]
    fn detects_scalar_only_shape() {
        assert!(prompt_requests_scalar_only_shape(
            "去 package.json 里找 name，只把值给我"
        ));
        assert!(!prompt_requests_scalar_only_shape(
            "把结果输出成 markdown table"
        ));
    }

    #[test]
    fn lifts_delivery_target_shape_into_surface_signals() {
        let signals = analyze_prompt_surface("把 readme 发给我");
        assert_eq!(
            signals.delivery_prompt_shape,
            Some(DeliveryPromptShape::PhraseWithTarget)
        );
    }

    #[test]
    fn detects_inline_transform_action_shape() {
        assert!(prompt_requests_inline_transform_action_shape(
            "sort this JSON array by score descending and render it as a markdown table"
        ));
    }

    #[test]
    fn lifts_inline_transform_target_shape_into_surface_signals() {
        let signals = analyze_prompt_surface(
            r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7}]"#,
        );
        assert_eq!(
            signals.inline_transform_prompt_shape,
            Some(InlineTransformPromptShape::ActionWithTarget)
        );
    }

    #[test]
    fn lifts_simple_explicit_field_read_shape_into_surface_signals() {
        let signals = analyze_prompt_surface(
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo",
        );
        assert_eq!(
            signals.field_read_prompt_shape,
            Some(FieldReadPromptShape::SimpleExplicitScalar)
        );
    }

    #[test]
    fn detects_weather_lookup_shape() {
        assert!(prompt_mentions_weather_lookup_shape("天气怎么样"));
        assert!(prompt_mentions_weather_lookup_shape("what is the weather"));
    }

    #[test]
    fn detects_delivery_phrase_shape() {
        assert!(prompt_requests_delivery_phrase("把 README.md 发给我"));
    }

    #[test]
    fn lifts_service_status_target_into_surface_signals() {
        let signals = analyze_prompt_surface("check whether telegramd is running");
        assert_eq!(signals.service_status_target.as_deref(), Some("telegramd"));
    }

    #[test]
    fn detects_deictic_object_shape() {
        assert!(prompt_references_deictic_object("把这个文件发给我"));
    }

    #[test]
    fn detects_generic_file_object_shape() {
        assert!(prompt_mentions_generic_file_object("请直接把文件发给我"));
    }

    #[test]
    fn detects_deictic_reference_shape() {
        assert!(prompt_contains_deictic_reference_shape("Use THIS log."));
        assert!(prompt_contains_deictic_reference_shape(
            "看看那个日志最后 5 行"
        ));
        assert!(!prompt_contains_deictic_reference_shape(
            "thisness should not match"
        ));
    }

    #[test]
    fn detects_current_workspace_scope_reference_shape() {
        assert!(prompt_mentions_current_workspace_scope_reference_shape(
            "this repository"
        ));
        assert!(prompt_mentions_current_workspace_scope_reference_shape(
            "看看这个目录"
        ));
    }
}
