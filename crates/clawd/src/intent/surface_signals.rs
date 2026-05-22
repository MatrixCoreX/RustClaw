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

pub(crate) fn inline_json_transform_request(prompt: &str) -> bool {
    let Some(raw) = crate::extract_first_json_value_any(prompt) else {
        return prompt_has_inline_csv_records(prompt);
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .is_some_and(|value| {
            value_has_structured_transform_request(&value)
                || (value_has_transformable_inline_records(&value)
                    && prompt_has_embedded_structured_payload_with_instruction(prompt, &raw))
        })
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

fn value_has_transformable_inline_records(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(items) => {
            !items.is_empty() && items.iter().any(serde_json::Value::is_object)
        }
        serde_json::Value::Object(obj) => {
            if value_has_inline_transform_input(obj) {
                return true;
            }
            !obj.is_empty()
                && !obj.contains_key("action")
                && !obj.contains_key("skill")
                && !obj.contains_key("operation")
        }
        _ => false,
    }
}

fn prompt_has_embedded_structured_payload_with_instruction(prompt: &str, raw: &str) -> bool {
    let trimmed = prompt.trim();
    let raw_trimmed = raw.trim();
    if trimmed.is_empty() || raw_trimmed.is_empty() || trimmed == raw_trimmed {
        return false;
    }
    let Some(start) = trimmed.find(raw_trimmed) else {
        return false;
    };
    let end = start.saturating_add(raw_trimmed.len());
    let before = trimmed[..start].trim();
    let after = trimmed[end..].trim();
    !before.is_empty() || !after.is_empty()
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
    for token in trimmed.split_whitespace().map(trim_pair_candidate_token) {
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
mod tests {
    use super::{
        analyze_prompt_surface, extract_dotted_field_selector, extract_field_selector_mentions,
        inline_json_transform_request, prompt_contains_delivery_token_reference, InlineJsonShape,
        LocatorHintPromptShape, LocatorReplyPromptShape,
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
        assert!(!signals.is_structural_locator_only_reply());
        assert_eq!(signals.field_selector_count, 0);
        assert!(signals.filename_candidates.is_empty());
        assert!(!signals.has_delivery_token_reference());
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
        assert_eq!(signals.field_selector_count, 0);
        assert!(!signals.filename_candidates.is_empty());
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
        assert!(signals.is_structural_locator_only_reply());
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
    fn inline_json_transform_requires_structured_payload() {
        assert!(inline_json_transform_request(
            r#"{"action":"transform_data","data":[{"name":"alpha","score":7}],"ops":[{"op":"sort","by":"score"}]}"#
        ));
        assert!(inline_json_transform_request(
            r#"{"skill":"transform","args":{"action":"transform_data","records":[{"name":"alpha","score":7}],"ops":["sort"]}}"#
        ));
        assert!(inline_json_transform_request(
            r#"sort this JSON array by score descending: [{"name":"alpha","score":7}]"#
        ));
        assert!(inline_json_transform_request(
            r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2}]"#
        ));
        assert!(!inline_json_transform_request(
            r#"{"action":"read_field","path":"package.json","field_path":"name"}"#
        ));
        assert!(inline_json_transform_request(
            "render this CSV as a markdown table:\nname,score\nalpha,7\nbeta,9"
        ));
        assert!(inline_json_transform_request(
            "render this CSV as a markdown table:name,score\\nalpha,7\\nbeta,9"
        ));
        assert!(inline_json_transform_request(
            "这个 CSV 按 score 降序输出 markdown 表格：name,score\\nli,3\\nwang,8\\nzhao,5"
        ));
    }

    #[test]
    fn deictic_reference_comes_from_structured_state_patch_only() {
        let signals = analyze_prompt_surface(
            r#"{"state_patch":{"deictic_reference":{"target":"unresolved_prior_object"}}}"#,
        );
        assert!(signals.has_deictic_reference());

        let natural = analyze_prompt_surface("read that file");
        assert!(!natural.has_deictic_reference());
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
    fn does_not_lift_filename_like_selector_from_language_context_only() {
        assert!(extract_dotted_field_selector("package.name 字段").is_none());
        assert!(extract_dotted_field_selector("package.name field").is_none());
    }

    #[test]
    fn leaves_bare_field_selector_semantics_to_planner() {
        let out = extract_field_selector_mentions(
            "读 scripts/nl_tests/fixtures/device_local/package.json，告诉我 scripts 字段下都有哪些子键",
        );
        assert!(out.is_empty());
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
    fn leaves_single_segment_field_after_locator_to_planner() {
        let out = extract_field_selector_mentions("去 package.json 里找 name，只把值给我");
        assert!(out.is_empty());
    }

    #[test]
    fn leaves_single_segment_value_phrase_to_planner() {
        let out =
            extract_field_selector_mentions("go into package.json and return only the name value");
        assert!(out.is_empty());
    }

    #[test]
    fn detects_delivery_token_reference_shape() {
        assert!(prompt_contains_delivery_token_reference(
            "再发一次 FILE:/tmp/example.txt"
        ));
        let signals = analyze_prompt_surface("再发一次 FILE:/tmp/example.txt");
        assert!(signals.has_delivery_token_reference());
    }

    #[test]
    fn lifts_locator_target_pair_into_surface_signals() {
        let signals = analyze_prompt_surface("比较 Cargo.toml 和 Cargo.lock 哪个更大");
        assert_eq!(
            signals.locator_target_pair,
            Some(("Cargo.toml".to_string(), "Cargo.lock".to_string()))
        );
    }

    #[test]
    fn locator_target_pair_ignores_dotted_version_numbers() {
        let signals =
            analyze_prompt_surface("Correction: not Python 3.10, use Python 3.11 instead");
        assert!(signals.locator_target_pair.is_none());
    }

    #[test]
    fn dotted_version_numbers_are_not_field_or_filename_signals() {
        let signals = analyze_prompt_surface("Correction: mention Python 3.11, not Python 3.10.");
        assert_eq!(signals.field_selector_count, 0);
        assert!(signals.dotted_field_selector.is_none());
        assert!(signals.filename_candidates.is_empty());
    }
}
