use std::path::{Path, PathBuf};

use crate::{AppState, IntentOutputContract, OutputResponseShape};

use super::file_delivery::resolve_file_delivery_target_with_hint;
use super::{
    extract_delivery_file_tokens, extract_file_path_from_delivery_token, localize_delivery_message,
    FileDeliveryTargetResolution,
};

pub(super) fn enforce_output_contract(
    state: &AppState,
    user_request: &str,
    output_contract: &IntentOutputContract,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    match output_contract.response_shape {
        OutputResponseShape::OneSentence => {
            *normalized_text = take_first_sentence(normalized_text);
        }
        OutputResponseShape::Scalar => {
            if let Some(scalar) = extract_scalar_literal(normalized_text) {
                *normalized_text = scalar;
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
        match resolve_file_delivery_target_with_hint(
            user_request,
            Path::new("/"),
            &state.default_locator_search_dir,
            state.locator_scan_max_depth,
            state.locator_scan_max_files,
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
                *normalized_text = localize_delivery_message(state, msg);
                normalized_messages
                    .retain(|msg| crate::finalizer::parse_delivery_file_token(msg).is_none());
            }
            None => {}
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
            | OutputResponseShape::Scalar
            | OutputResponseShape::FileToken
    ) || response_has_any_delivery_token(text, messages)
}

pub(crate) fn sync_output_payload(
    output_contract: &IntentOutputContract,
    normalized_text: &mut String,
    normalized_messages: &mut Vec<String>,
) {
    let canonical = canonical_output_text(normalized_text, normalized_messages);
    *normalized_text = canonical.clone();
    normalized_messages.retain(|message| !message.trim().is_empty());
    if canonical.is_empty() {
        normalized_messages.clear();
        return;
    }
    if should_collapse_to_single_output(output_contract, normalized_text, normalized_messages) {
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

fn extract_scalar_literal(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if is_scalar_literal(trimmed) {
        return Some(trimmed.to_string());
    }
    for token in trimmed.split_whitespace() {
        if is_scalar_literal(token) {
            return Some(token.to_string());
        }
    }
    None
}

fn is_scalar_literal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.trim();
    if s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    s.parse::<f64>().is_ok()
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
