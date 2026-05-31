use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineJsonShape {
    WholeValue,
    EmbeddedPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocatorHintPromptShape {
    ExplicitPathOrUrl,
    ConcreteImplicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocatorReplyPromptShape {
    LocatorOnly,
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
    pub(crate) single_filename_candidate: Option<String>,
    pub(crate) delivery_token_reference: bool,
    pub(crate) locator_target_pair: Option<(String, String)>,
    pub(crate) deictic_reference: bool,
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

    pub(crate) fn is_structural_locator_only_reply(&self) -> bool {
        matches!(
            self.locator_reply_prompt_shape,
            Some(LocatorReplyPromptShape::LocatorOnly)
        )
    }

    pub(crate) fn has_any_locator_reference(&self) -> bool {
        self.has_concrete_locator_hint()
    }

    pub(crate) fn has_single_filename_candidate(&self) -> bool {
        self.single_filename_candidate.is_some()
    }

    pub(crate) fn has_filename_candidates(&self) -> bool {
        !self
            .filename_candidates_excluding_field_selectors()
            .is_empty()
    }

    pub(crate) fn single_filename_candidate(&self) -> Option<&str> {
        self.single_filename_candidate.as_deref()
    }

    pub(crate) fn has_structured_target_refinement(&self) -> bool {
        self.field_selector_count > 0 || self.dotted_field_selector.is_some()
    }

    pub(crate) fn has_delivery_token_reference(&self) -> bool {
        self.delivery_token_reference
    }

    pub(crate) fn has_deictic_reference(&self) -> bool {
        self.deictic_reference
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

pub(crate) fn analyze_prompt_surface(prompt: &str) -> PromptSurfaceSignals {
    let prompt = prompt_without_contract_test_hint_blocks(prompt);
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
    let single_filename_candidate = {
        let mut unique = filename_candidates.clone();
        unique.dedup();
        (unique.len() == 1).then(|| unique.remove(0))
    };
    let has_explicit_path_or_url = has_explicit_path_or_url_shape(trimmed);
    let has_concrete_locator_hint = crate::worker::has_concrete_locator_hint(trimmed);
    let structural_locator_only_reply =
        crate::clarify_followup::prompt_is_structural_locator_only(trimmed);
    let locator_hint_prompt_shape =
        classify_locator_hint_prompt_shape(has_explicit_path_or_url, has_concrete_locator_hint);
    let locator_reply_prompt_shape =
        structural_locator_only_reply.then_some(LocatorReplyPromptShape::LocatorOnly);
    let delivery_token_reference = prompt_contains_delivery_token_reference(trimmed);
    let locator_target_pair = detect_locator_target_pair_shape(trimmed);
    let deictic_reference = prompt_has_structured_deictic_reference(trimmed);
    PromptSurfaceSignals {
        token_count,
        inline_json_shape,
        locator_hint_prompt_shape,
        locator_reply_prompt_shape,
        field_selector_mentions,
        field_selector_count,
        dotted_field_selector,
        filename_candidates,
        single_filename_candidate,
        delivery_token_reference,
        locator_target_pair,
        deictic_reference,
    }
}

fn prompt_without_contract_test_hint_blocks(prompt: &str) -> Cow<'_, str> {
    const START: &str = "[CONTRACT_TEST_HINT]";
    const END: &str = "[/CONTRACT_TEST_HINT]";

    let Some(first_start) = prompt.find(START) else {
        return Cow::Borrowed(prompt);
    };

    let mut out = String::with_capacity(prompt.len());
    out.push_str(&prompt[..first_start]);
    let mut rest = &prompt[first_start + START.len()..];
    loop {
        let Some(end_idx) = rest.find(END) else {
            return Cow::Owned(out);
        };
        rest = &rest[end_idx + END.len()..];
        let Some(start_idx) = rest.find(START) else {
            out.push_str(rest);
            return Cow::Owned(out);
        };
        out.push_str(&rest[..start_idx]);
        rest = &rest[start_idx + START.len()..];
    }
}

pub(crate) fn inline_json_transform_request(prompt: &str) -> bool {
    let Some(raw) = crate::extract_first_json_value_any(prompt) else {
        return prompt_has_inline_csv_records(prompt);
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .is_some_and(|value| value_has_structured_transform_request(&value))
}

fn prompt_has_inline_csv_records(prompt: &str) -> bool {
    inline_csv_record_block(prompt).is_some()
}

pub(crate) fn inline_csv_record_block(prompt: &str) -> Option<Vec<String>> {
    let lines = split_inline_record_lines(prompt);
    for idx in 0..lines.len().saturating_sub(1) {
        let header = parse_csv_surface_line(&lines[idx]);
        if header.len() < 2 || !header.iter().all(|cell| is_plain_record_field_name(cell)) {
            continue;
        }
        let mut block = vec![lines[idx].clone()];
        let mut row_count = 0usize;
        for row in lines.iter().skip(idx + 1) {
            let cells = parse_csv_surface_line(row);
            if cells.len() != header.len() {
                break;
            }
            block.push(row.clone());
            row_count += 1;
        }
        if row_count > 0 {
            return Some(block);
        }
    }
    None
}

fn split_inline_record_lines(prompt: &str) -> Vec<String> {
    let normalized = prompt
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n");
    let mut lines = Vec::new();
    for raw_line in normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        lines.push(raw_line.to_string());
        if let Some((idx, len)) = raw_line
            .char_indices()
            .filter(|(_, ch)| matches!(ch, ':' | '：'))
            .map(|(idx, ch)| (idx, ch.len_utf8()))
            .last()
        {
            let suffix = raw_line[idx + len..].trim();
            if suffix.contains(',') && suffix != raw_line {
                lines.push(suffix.to_string());
            }
        }
    }
    lines
}

fn parse_csv_surface_line(line: &str) -> Vec<&str> {
    line.split(',')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .collect()
}

fn is_plain_record_field_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '-' || ch.is_ascii_alphanumeric())
}

fn value_has_structured_transform_request(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.get("skill").and_then(|item| item.as_str()) == Some("transform") {
        return obj
            .get("args")
            .is_some_and(value_has_structured_transform_request);
    }
    let action = obj
        .get("action")
        .or_else(|| obj.get("operation"))
        .and_then(|item| item.as_str());
    let action_requests_transform = matches!(action, Some("transform_data" | "transform"));
    let has_structural_ops = obj
        .get("ops")
        .and_then(|item| item.as_array())
        .is_some_and(|ops| !ops.is_empty() && ops.iter().all(value_is_structured_transform_op));
    action_requests_transform && has_structural_ops && value_has_inline_transform_input(obj)
}

fn value_has_inline_transform_input(obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    obj.get("data")
        .or_else(|| obj.get("records"))
        .or_else(|| obj.get("input"))
        .and_then(|item| item.as_array())
        .is_some_and(|items| !items.is_empty() && items.iter().any(serde_json::Value::is_object))
}

fn value_is_structured_transform_op(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(op) => matches!(
            op.as_str(),
            "sort" | "filter" | "dedup" | "project" | "group" | "aggregate" | "format"
        ),
        serde_json::Value::Object(obj) => obj
            .get("op")
            .or_else(|| obj.get("action"))
            .and_then(|item| item.as_str())
            .is_some_and(|op| {
                matches!(
                    op,
                    "sort" | "filter" | "dedup" | "project" | "group" | "aggregate" | "format"
                )
            }),
        _ => false,
    }
}

fn prompt_has_structured_deictic_reference(prompt: &str) -> bool {
    let Some(raw) = crate::extract_first_json_value_any(prompt) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .is_some_and(|value| value_has_structured_deictic_reference(&value))
}

fn value_has_structured_deictic_reference(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let direct = obj.get("deictic_reference");
    let nested = obj
        .get("state_patch")
        .and_then(serde_json::Value::as_object)
        .and_then(|patch| patch.get("deictic_reference"));
    [direct, nested]
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
        .filter_map(|reference| reference.get("target"))
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .any(|target| !target.is_empty() && target != "none")
}

fn classify_locator_hint_prompt_shape(
    has_explicit_path_or_url: bool,
    has_concrete_locator_hint: bool,
) -> Option<LocatorHintPromptShape> {
    if has_explicit_path_or_url {
        Some(LocatorHintPromptShape::ExplicitPathOrUrl)
    } else if has_concrete_locator_hint {
        Some(LocatorHintPromptShape::ConcreteImplicit)
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

fn has_explicit_path_or_url_shape(prompt: &str) -> bool {
    crate::worker::has_explicit_path_or_url_locator_hint(prompt)
}

pub(crate) fn detect_locator_target_pair_shape(prompt: &str) -> Option<(String, String)> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut explicit_paths = Vec::new();
    for token in split_pair_candidate_tokens(trimmed).map(trim_pair_candidate_token) {
        if !token.is_empty() && crate::worker::has_explicit_path_or_url_locator_hint(token) {
            push_unique_case_insensitive(&mut explicit_paths, token.to_string());
        }
    }
    if explicit_paths.len() == 2 {
        return Some((explicit_paths.remove(0), explicit_paths.remove(0)));
    }
    let mut filenames = Vec::new();
    for candidate in crate::delivery_utils::extract_filename_candidates(trimmed) {
        if !crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(&candidate)
        {
            push_unique_case_insensitive(&mut filenames, candidate);
        }
    }
    (filenames.len() == 2).then(|| (filenames.remove(0), filenames.remove(0)))
}

fn split_pair_candidate_tokens<'a>(prompt: &'a str) -> impl Iterator<Item = &'a str> + 'a {
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

fn trim_pair_candidate_token(token: &str) -> &str {
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

fn push_unique_case_insensitive(values: &mut Vec<String>, value: String) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
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
    if crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed) {
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
            && !filename_like_dotted_selector_has_prior_locator_context(
                prompt,
                &selector,
                &filename_candidates,
            )
        {
            return None;
        }
        Some(selector)
    })
}

fn filename_like_dotted_selector_has_prior_locator_context(
    prompt: &str,
    selector: &str,
    filename_candidates: &[String],
) -> bool {
    let lower_prompt = prompt.to_ascii_lowercase();
    let selector_lower = selector.to_ascii_lowercase();
    let Some(selector_idx) = lower_prompt.find(&selector_lower) else {
        return false;
    };

    filename_candidates.iter().any(|candidate| {
        !candidate.eq_ignore_ascii_case(&selector_lower)
            && lower_prompt
                .find(candidate)
                .is_some_and(|candidate_idx| candidate_idx < selector_idx)
    })
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
    selectors
}

#[cfg(test)]
#[path = "surface_signals_tests.rs"]
mod tests;
