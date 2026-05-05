use std::path::{Path, PathBuf};

use crate::{AppState, IntentOutputContract, OutputResponseShape};

use super::file_delivery::resolve_file_delivery_target_with_hint;
use super::types::localize_delivery_message_for_request;
use super::{
    extract_delivery_file_tokens, extract_file_path_from_delivery_token, trim_path_token,
    FileDeliveryTargetResolution,
};

fn existing_file_path_literal(text: &str) -> Option<PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    let path = Path::new(trimmed);
    if !path.is_file() {
        return None;
    }
    Some(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

pub(super) fn looks_like_delivery_locator_literal(text: &str, locator_hint: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || crate::finalize::looks_like_planner_artifact(trimmed)
        || crate::finalize::parse_delivery_file_token(trimmed).is_some()
    {
        return false;
    }

    let normalized = trim_path_token(trimmed);
    if normalized.is_empty() {
        return false;
    }

    let hint = trim_path_token(locator_hint);
    if !hint.is_empty() {
        if normalized == hint {
            return true;
        }
        if Path::new(&hint)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| normalized == name)
        {
            return true;
        }
    }

    if normalized.starts_with('/')
        || normalized.starts_with("./")
        || normalized.starts_with("../")
        || normalized.contains('/')
        || normalized.contains('\\')
    {
        return true;
    }

    if normalized.chars().any(char::is_whitespace) {
        return false;
    }

    normalized
        .rsplit('/')
        .next()
        .unwrap_or(&normalized)
        .contains('.')
}

fn looks_like_markdown_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 3
}

fn looks_like_markdown_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !looks_like_markdown_table_row(trimmed) {
        return false;
    }
    trimmed
        .trim_matches('|')
        .split('|')
        .all(|cell| cell.trim().chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

fn strip_preamble_before_markdown_table(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let Some(table_start) = lines
        .iter()
        .position(|line| looks_like_markdown_table_row(line))
    else {
        return text.to_string();
    };
    let Some(separator) = lines.get(table_start + 1) else {
        return text.to_string();
    };
    if !looks_like_markdown_table_separator(separator) {
        return text.to_string();
    }
    lines[table_start..].join("\n").trim().to_string()
}

pub(super) fn enforce_output_contract(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    *normalized_text = strip_preamble_before_markdown_table(normalized_text);
    match output_contract.response_shape {
        OutputResponseShape::OneSentence => {
            if output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary {
                *normalized_text = take_first_sentence(normalized_text);
            }
        }
        OutputResponseShape::Scalar => {
            // QuantityComparison 的回答天然由"较大方 + 双方数值"组成（如 "docs 更多：docs 有 3 个，logs 有 2 个"），
            // 强行 extract_scalar_literal 会把整句压成首个 ASCII 数字 "3"，把已经合规的对比答案破坏成
            // 单孤立数字——典型"假成功"。Comparison 类保留 LLM 的完整短句即可，下游 chat 渲染器
            // 已经按 chat_response_prompt 的输出契约保证了简洁度。
            if !matches!(
                output_contract.semantic_kind,
                crate::OutputSemanticKind::QuantityComparison
            ) && !contains_missing_scalar_sentinel(normalized_text)
            {
                if let Some(scalar) =
                    extract_scalar_literal_for_contract(normalized_text, output_contract)
                {
                    *normalized_text = scalar;
                }
            }
        }
        _ => {}
    }

    let file_contract = output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        );
    if file_contract && !response_has_any_delivery_token(normalized_text, normalized_messages) {
        let current_output = canonical_output_text(normalized_text, normalized_messages);
        if let Some(path) = existing_file_path_literal(normalized_text).or_else(|| {
            normalized_messages
                .iter()
                .rev()
                .find_map(|message| existing_file_path_literal(message))
        }) {
            let token = format!("FILE:{}", path.display());
            *normalized_text = token.clone();
            normalized_messages.clear();
            normalized_messages.push(token);
        } else if current_output.trim().is_empty()
            || looks_like_delivery_locator_literal(&current_output, &output_contract.locator_hint)
        {
            match resolve_file_delivery_target_with_hint(
                user_request,
                Path::new("/"),
                &state.skill_rt.default_locator_search_dir,
                state.skill_rt.locator_scan_max_depth,
                state.skill_rt.locator_scan_max_files,
                Some(output_contract.locator_hint.as_str()),
            ) {
                Some(FileDeliveryTargetResolution::Resolved(path)) => {
                    let token = format!("FILE:{}", path.display());
                    *normalized_text = token.clone();
                    if !normalized_messages.iter().any(|m| m == &token) {
                        normalized_messages.push(token);
                    }
                }
                Some(FileDeliveryTargetResolution::UserMessage(msg)) => {
                    *normalized_text =
                        localize_delivery_message_for_request(state, msg, user_request);
                    normalized_messages
                        .retain(|msg| crate::finalize::parse_delivery_file_token(msg).is_none());
                }
                Some(FileDeliveryTargetResolution::Candidates(paths)) => {
                    let mut lines = Vec::with_capacity(paths.len() + 1);
                    lines.push(localize_delivery_message_for_request(
                        state,
                        super::DeliveryMessageKind::FilenameNotUnique,
                        user_request,
                    ));
                    lines.extend(paths.into_iter().map(|path| path.display().to_string()));
                    let text = lines.join("\n");
                    *normalized_text = text.clone();
                    normalized_messages
                        .retain(|msg| crate::finalize::parse_delivery_file_token(msg).is_none());
                    normalized_messages.clear();
                    normalized_messages.push(text);
                }
                None => {}
            }
        }
    }
    sync_output_payload(output_contract, normalized_text, normalized_messages);
}

fn response_has_any_delivery_token(text: &str, messages: &[String]) -> bool {
    !extract_delivery_file_tokens(text).is_empty()
        || messages
            .iter()
            .any(|m| !extract_delivery_file_tokens(m).is_empty())
}

fn canonical_output_text(text: &str, messages: &[String]) -> String {
    let text = text.trim();
    if !extract_delivery_file_tokens(text).is_empty() {
        return text.to_string();
    }
    if let Some(message) = messages
        .iter()
        .rev()
        .find(|msg| !extract_delivery_file_tokens(msg).is_empty())
    {
        return message.trim().to_string();
    }
    if !text.is_empty() {
        return text.to_string();
    }
    messages
        .iter()
        .rev()
        .find_map(|message| {
            let trimmed = message.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        })
        .unwrap_or_default()
}

fn should_collapse_to_single_output(
    output_contract: &IntentOutputContract,
    text: &str,
    messages: &[String],
) -> bool {
    matches!(
        output_contract.response_shape,
        OutputResponseShape::OneSentence
            | OutputResponseShape::Strict
            | OutputResponseShape::Scalar
            | OutputResponseShape::FileToken
    ) || response_has_any_delivery_token(text, messages)
}

fn should_preserve_execution_summary_messages(output_contract: &IntentOutputContract) -> bool {
    let _ = output_contract;
    true
}

pub(crate) fn sync_output_payload(
    output_contract: &IntentOutputContract,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    let mut canonical = canonical_output_text(normalized_text, normalized_messages);
    canonical = strip_preamble_before_markdown_table(&canonical);
    let file_contract = output_contract.delivery_required
        || matches!(
            output_contract.response_shape,
            OutputResponseShape::FileToken
        );
    if file_contract {
        if let Some(path) = existing_file_path_literal(&canonical) {
            canonical = format!("FILE:{}", path.display());
        }
    }
    *normalized_text = canonical.clone();
    normalized_messages.retain(|message| !message.trim().is_empty());
    if canonical.is_empty() {
        normalized_messages.clear();
        return;
    }
    if should_collapse_to_single_output(output_contract, normalized_text, normalized_messages) {
        if should_preserve_execution_summary_messages(output_contract) {
            let execution_summaries = normalized_messages
                .iter()
                .filter(|message| crate::finalize::is_execution_summary_message(message))
                .cloned()
                .collect::<Vec<_>>();
            if !execution_summaries.is_empty() {
                normalized_messages.clear();
                normalized_messages.extend(execution_summaries);
                normalized_messages.push(canonical);
                return;
            }
        }
        normalized_messages.clear();
        normalized_messages.push(canonical);
        return;
    }
    match normalized_messages.last_mut() {
        Some(last) => *last = canonical,
        None => normalized_messages.push(canonical),
    }
}

fn looks_like_leading_label_line(line: &str) -> bool {
    let mut trimmed = line.trim();
    loop {
        let next = if let Some(inner) = trimmed
            .strip_prefix("**")
            .and_then(|v| v.strip_suffix("**"))
        {
            Some(inner.trim())
        } else if let Some(inner) = trimmed
            .strip_prefix("__")
            .and_then(|v| v.strip_suffix("__"))
        {
            Some(inner.trim())
        } else if let Some(inner) = trimmed.strip_prefix('*').and_then(|v| v.strip_suffix('*')) {
            Some(inner.trim())
        } else {
            trimmed
                .strip_prefix('_')
                .and_then(|v| v.strip_suffix('_'))
                .map(str::trim)
        };
        if let Some(next_trimmed) = next {
            if next_trimmed == trimmed || next_trimmed.is_empty() {
                break;
            }
            trimmed = next_trimmed;
            continue;
        }
        break;
    }
    if trimmed.is_empty() {
        return false;
    }
    let has_label_suffix = trimmed.ends_with(':') || trimmed.ends_with('：');
    if !has_label_suffix {
        return false;
    }
    let core = trimmed
        .strip_suffix(':')
        .or_else(|| trimmed.strip_suffix('：'))
        .unwrap_or(trimmed)
        .trim();
    if core.is_empty() {
        return false;
    }
    let core_chars = core.chars().count();
    core_chars <= 64
        && !core
            .chars()
            .any(|ch| matches!(ch, '.' | '。' | '!' | '?' | '！' | '？'))
}

pub(crate) fn take_first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lines = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let mut source_idx = 0usize;
    if lines[source_idx].starts_with('#') {
        if let Some((idx, _)) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| !line.starts_with('#'))
        {
            source_idx = idx;
        }
    }
    if looks_like_leading_label_line(lines[source_idx]) {
        if let Some((idx, _)) = lines
            .iter()
            .enumerate()
            .skip(source_idx + 1)
            .find(|(_, line)| !line.starts_with('#'))
        {
            source_idx = idx;
        }
    }
    let source = lines[source_idx];
    let chars: Vec<char> = source.chars().collect();
    let mut buf = String::new();
    for (idx, ch) in chars.iter().copied().enumerate() {
        buf.push(ch);
        if matches!(ch, '。' | '!' | '?' | '！' | '？') {
            break;
        }
        if ch == '.' {
            let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
            let next = chars.get(idx + 1).copied();
            let in_token = prev.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false)
                && next.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
            if in_token {
                continue;
            }
            if next.map(|c| c.is_whitespace()).unwrap_or(true) {
                break;
            }
        }
    }
    let out = buf.trim();
    if out.is_empty() {
        source.to_string()
    } else {
        out.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::{IntentOutputContract, OutputResponseShape, OutputSemanticKind};

    use super::{enforce_output_contract, looks_like_delivery_locator_literal};

    #[test]
    fn delivery_locator_literal_accepts_hint_and_path_shapes() {
        assert!(looks_like_delivery_locator_literal(
            "README.md",
            "README.md"
        ));
        assert!(looks_like_delivery_locator_literal(
            "/tmp/report.md",
            "README.md"
        ));
        assert!(looks_like_delivery_locator_literal(
            "configs/config.toml",
            "configs/config.toml"
        ));
        assert!(looks_like_delivery_locator_literal("LICENSE", "LICENSE"));
    }

    #[test]
    fn delivery_locator_literal_rejects_user_facing_sentences() {
        assert!(!looks_like_delivery_locator_literal(
            "未找到该文件。文件 `definitely_missing_named_file_rustclaw_001.txt` 在工作区中不存在。",
            "definitely_missing_named_file_rustclaw_001.txt"
        ));
        assert!(!looks_like_delivery_locator_literal(
            "LOCATOR_CLARIFY_PROMPT",
            "README.md"
        ));
    }

    #[test]
    fn scalar_contract_extracts_single_machine_token_from_sentence() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            ..Default::default()
        };
        let mut text = "测试编号是 **minimax-small-20260429_201348**。".to_string();
        let mut messages = Vec::new();

        enforce_output_contract(&state, "只回答编号", &contract, &mut text, &mut messages);

        assert_eq!(text, "minimax-small-20260429_201348");
    }

    #[test]
    fn scalar_contract_preserves_natural_language_summary_with_single_ascii_token() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::None,
            ..Default::default()
        };
        let expected = "该测试验证 RustClaw 在连续会话下能否稳定保持上下文、记忆和状态。";
        let mut text = expected.to_string();
        let mut messages = Vec::new();

        enforce_output_contract(
            &state,
            "请用一句话总结这个连续会话测试主要验证什么。",
            &contract,
            &mut text,
            &mut messages,
        );

        assert_eq!(text, expected);
        assert_eq!(messages, vec![expected.to_string()]);
    }

    #[test]
    fn scalar_count_contract_still_extracts_count_from_sentence() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarCount,
            ..Default::default()
        };
        let mut text = "共有 3 个文件。".to_string();
        let mut messages = Vec::new();

        enforce_output_contract(&state, "只回答数量", &contract, &mut text, &mut messages);

        assert_eq!(text, "3");
        assert_eq!(messages, vec!["3".to_string()]);
    }

    #[test]
    fn scalar_contract_preserves_missing_sentinel() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            ..Default::default()
        };
        let mut text = "<missing>".to_string();
        let mut messages = Vec::new();

        enforce_output_contract(&state, "只回答字段值", &contract, &mut text, &mut messages);

        assert_eq!(text, "<missing>");
    }

    #[test]
    fn scalar_contract_preserves_structured_missing_field_line() {
        let state = crate::AppState::test_default_with_fixture_provider();
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            ..Default::default()
        };
        let mut text = "package.name: <missing>".to_string();
        let mut messages = Vec::new();

        enforce_output_contract(&state, "只回答字段值", &contract, &mut text, &mut messages);

        assert_eq!(text, "package.name: <missing>");
    }
}

fn extract_scalar_literal_for_contract(
    text: &str,
    output_contract: &IntentOutputContract,
) -> Option<String> {
    if allows_loose_scalar_token_extraction(output_contract.semantic_kind) {
        extract_scalar_literal_loose(text)
    } else {
        extract_scalar_literal_explicit(text)
    }
}

fn allows_loose_scalar_token_extraction(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::ScalarCount
            | crate::OutputSemanticKind::ScalarPathOnly
            | crate::OutputSemanticKind::SqliteTableNamesOnly
            | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
    )
}

fn extract_scalar_literal_explicit(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed == "<missing>" {
        return Some(trimmed.to_string());
    }
    if is_scalar_literal(trimmed) {
        return Some(trimmed.to_string());
    }

    if let Some(scalar) = extract_single_delimited_scalar(trimmed, "`") {
        return Some(scalar);
    }
    if let Some(scalar) = extract_single_delimited_scalar(trimmed, "**") {
        return Some(scalar);
    }
    None
}

fn extract_scalar_literal_loose(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if let Some(scalar) = extract_scalar_literal_explicit(trimmed) {
        return Some(scalar);
    }

    let mut candidates = Vec::new();
    for token in trimmed.split_whitespace() {
        let token = trim_scalar_token_punctuation(token);
        if is_scalar_literal(&token) && !candidates.iter().any(|existing| existing == &token) {
            candidates.push(token);
        }
    }
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn contains_missing_scalar_sentinel(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == "<missing>" || trimmed.ends_with(": <missing>")
}

fn extract_single_delimited_scalar(text: &str, delimiter: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(delimiter) {
        let after_start = &rest[start + delimiter.len()..];
        let Some(end) = after_start.find(delimiter) else {
            break;
        };
        let candidate = trim_scalar_token_punctuation(&after_start[..end]);
        if is_scalar_literal(&candidate)
            && !candidates.iter().any(|existing| existing == &candidate)
        {
            candidates.push(candidate);
        }
        rest = &after_start[end + delimiter.len()..];
    }
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn trim_scalar_token_punctuation(token: &str) -> String {
    let mut current = token.trim();
    loop {
        let next = current
            .trim_matches(|ch: char| {
                ch.is_ascii_punctuation()
                    && !matches!(
                        ch,
                        '-' | '_' | '.' | ':' | '/' | '\\' | '@' | '=' | '+' | '#'
                    )
            })
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '。' | '，'
                        | '、'
                        | '；'
                        | '：'
                        | '！'
                        | '？'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '《'
                        | '》'
                )
            });
        if next == current {
            return current.to_string();
        }
        current = next;
    }
}

fn is_scalar_literal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.trim();
    if s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if s.parse::<f64>().is_ok() {
        return true;
    }
    let char_count = s.chars().count();
    char_count <= 200
        && s.chars().any(|c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '-' | '_' | '.' | ':' | '/' | '\\' | '@' | '=' | '+' | '#'
                )
        })
}

pub(crate) fn response_has_same_file_token(
    text: &str,
    messages: &[String],
    expected: &Path,
) -> bool {
    let expected_str = expected.to_string_lossy().to_string();
    let mut candidates = Vec::with_capacity(messages.len() + 1);
    candidates.push(text.to_string());
    candidates.extend_from_slice(messages);
    candidates.iter().any(|msg| {
        extract_delivery_file_tokens(msg).iter().any(|token| {
            extract_file_path_from_delivery_token(token)
                .map(|path| {
                    let p = if Path::new(&path).is_absolute() {
                        PathBuf::from(&path)
                    } else {
                        expected
                            .parent()
                            .map(|parent| parent.join(&path))
                            .unwrap_or_else(|| PathBuf::from(&path))
                    };
                    p.canonicalize()
                        .ok()
                        .map(|cp| cp == expected)
                        .unwrap_or_else(|| path == expected_str)
                })
                .unwrap_or(false)
        })
    })
}
