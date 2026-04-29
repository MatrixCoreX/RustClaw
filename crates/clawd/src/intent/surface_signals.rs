#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceScopePromptShape {
    ExplicitScope,
    ReferenceScope,
    ExplicitAndReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeicticPromptShape {
    ObjectTarget,
    FreshReference,
    GeneralReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineJsonShape {
    WholeValue,
    EmbeddedPayload,
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
    pub(crate) deictic_prompt_shape: Option<DeicticPromptShape>,
    pub(crate) workspace_scope_prompt_shape: Option<WorkspaceScopePromptShape>,
    pub(crate) requested_read_range: Option<crate::read_range_request::RequestedReadRange>,
    pub(crate) requested_listing_limit: Option<usize>,
    pub(crate) workspace_child_directory_hint: Option<String>,
    pub(crate) compare_target_pair: Option<(String, String)>,
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
    let requested_sentence_count = requested_sentence_count_shape(trimmed);
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
    let mentions_current_workspace_scope_shape = prompt_mentions_current_workspace_scope(trimmed);
    let mentions_current_workspace_scope_reference_shape =
        prompt_mentions_current_workspace_scope_reference_shape(trimmed);
    let workspace_scope_prompt_shape = classify_workspace_scope_prompt_shape(
        mentions_current_workspace_scope_shape,
        mentions_current_workspace_scope_reference_shape,
    );
    let requested_listing_limit =
        crate::listing_limit_request::requested_listing_limit_from_prompt(trimmed);
    let workspace_child_directory_hint = extract_workspace_child_directory_hint_shape(trimmed);
    let compare_target_pair = detect_compare_targets_shape(trimmed);
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
        deictic_prompt_shape,
        workspace_scope_prompt_shape,
        requested_read_range,
        requested_listing_limit,
        workspace_child_directory_hint,
        compare_target_pair,
    }
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

fn has_explicit_path_or_url_shape(prompt: &str) -> bool {
    crate::worker::has_explicit_path_or_url_locator_hint(prompt)
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
    None
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
        prompt_references_deictic_object, prompt_requests_compare_shape,
        prompt_requests_quantity_comparison_shape, requested_sentence_count_shape,
        DeicticPromptShape, FileReferencePromptShape, InlineJsonShape, LocatorHintPromptShape,
        LocatorReplyPromptShape, WorkspaceScopePromptShape,
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
        assert!(signals.file_reference_prompt_shape.is_none());
        assert!(signals.deictic_prompt_shape.is_none());
        assert!(signals.workspace_scope_prompt_shape.is_none());
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
            signals.file_reference_prompt_shape,
            Some(FileReferencePromptShape::GenericObject)
        );
        assert_eq!(
            signals.deictic_prompt_shape,
            Some(DeicticPromptShape::ObjectTarget)
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
