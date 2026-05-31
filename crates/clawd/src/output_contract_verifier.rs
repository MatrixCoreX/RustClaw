//! §7.1 Output-contract verifier：finalize 阶段把 candidate 文本拿来对照
//! [`crate::IntentOutputContract`] 做最小结构合规性判定，是
//! [`crate::delivery_utils::output_contract::enforce_output_contract`] "shape 整形"之上的 guard。
//!
//! 设计原则（保守版 v2）：
//! - 只拦"明显违反 contract"的结构性 anti-pattern，宁可漏拦也不要误拦合规答案。
//! - 不用 Rust 词表判断 yes/no、same/different、意图类别或回复语气；这些属于 LLM
//!   normalizer/planner/composer 的语义职责。
//! - 代码层只处理空输出、路径 token、整数 token 等跨模型稳定的结构事实。
//! - **优先程序化 reshape**：比如 scalar path/count 已经在回复里给出了唯一结构值，
//!   可以抽取成严格输出；只有缺少结构值时才 Reject。
//! - Reject 由调用方接 §7.2 [`crate::fallback::ClarifyFallbackSource::VerifyRejected`]
//!   兜底，外加 tracing 事件保留判定原因，便于 inspect_task.sh 关联。

use crate::{IntentOutputContract, OutputResponseShape, OutputSemanticKind};
use std::collections::BTreeSet;
use std::path::Path;

/// §7.1 verifier 判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutputContractVerdict {
    /// candidate 已满足 contract，直接放行。
    Pass,
    /// candidate **基本** 满足 contract，但有可程序化修复的 deviation；
    /// `reshaped` 是修复后的文本，调用方应直接用它替换原 candidate。
    Reshape { reason: String, reshaped: String },
    /// candidate **明显** 违反 contract，且无法程序化修复：
    /// scalar path/count 缺少对应结构值。
    /// 调用方应丢弃 candidate，走 §7.2 `VerifyRejected` fallback。
    Reject { reason: String },
}

impl OutputContractVerdict {
    /// 给未来 metrics counter `clawd_finalize_verify_total{verdict="..."}` 用。
    /// 当前 finalize hook 直接用 match 分支 + tracing field，未走此函数。
    #[allow(dead_code)]
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Reshape { .. } => "reshape",
            Self::Reject { .. } => "reject",
        }
    }
}

/// "回答里含路径/locator 结构"的 guard：
/// - 含 `/`（绝对/相对路径或 URL 风），或
/// - 含 `\`（Windows 路径），或
/// - locator_hint 非空且回答里出现该 hint 字面。
fn contains_path_or_locator(text: &str, locator_hint: &str) -> bool {
    let t = text.trim();
    if t.contains('/') || t.contains('\\') {
        return true;
    }
    let hint = locator_hint.trim();
    !hint.is_empty() && t.contains(hint)
}

fn first_path_like_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|s| {
            s.trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '`' | '(' | ')' | '。' | '，' | ',' | '.' | ';' | ':' | '：'
                )
            })
        })
        .find(|tok| !tok.is_empty() && (tok.contains('/') || tok.contains('\\')))
        .map(str::to_string)
}

fn output_list_items(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut item = line.trim();
            if item.is_empty() || item.starts_with("```") {
                return None;
            }
            item = item
                .trim_start_matches(|ch: char| ch == '-' || ch == '*' || ch.is_whitespace())
                .trim();
            if let Some((prefix, rest)) = item.split_once('.') {
                if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
                    item = rest.trim();
                }
            } else if let Some((prefix, rest)) = item.split_once(')') {
                if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
                    item = rest.trim();
                }
            }
            let item = item
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '。' | '，' | ',' | ';' | '：'
                    )
                })
                .trim();
            (!item.is_empty()).then(|| item.to_string())
        })
        .collect()
}

fn extension_hints_from_text(text: &str) -> BTreeSet<String> {
    let mut hints = BTreeSet::new();
    let mut chars = text.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '.' {
            continue;
        }
        let mut ext = String::new();
        while let Some((_, next)) = chars.peek().copied() {
            if next.is_ascii_alphanumeric() || next == '_' || next == '-' {
                ext.push(next.to_ascii_lowercase());
                chars.next();
            } else {
                break;
            }
        }
        if (1..=16).contains(&ext.len()) {
            hints.insert(ext);
        }
    }
    hints
}

fn final_component_extension(item: &str) -> Option<String> {
    let item = item.trim().trim_end_matches('/');
    let component = item
        .rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|component| !component.is_empty())?;
    if component.starts_with('.') && component.matches('.').count() == 1 {
        return None;
    }
    Path::new(component)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty() && ext.len() <= 16)
        .map(|ext| ext.to_ascii_lowercase())
}

fn verify_directory_names(
    contract: &IntentOutputContract,
    text: &str,
    user_request: &str,
) -> OutputContractVerdict {
    let items = output_list_items(text);
    if items.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "directory_names: empty list candidate".to_string(),
        };
    }
    let ext_hints = extension_hints_from_text(user_request);
    let file_like = items
        .iter()
        .filter_map(|item| final_component_extension(item).map(|ext| (item, ext)))
        .collect::<Vec<_>>();
    let requested_ext_file_like = file_like
        .iter()
        .filter(|(_, ext)| ext_hints.contains(ext))
        .count();
    if requested_ext_file_like >= 2
        || file_like.iter().any(|(item, ext)| {
            ext_hints.contains(ext) && (item.contains('/') || item.contains('\\'))
        })
    {
        return OutputContractVerdict::Reject {
            reason: "directory_names: candidate contains file entries matching requested extension"
                .to_string(),
        };
    }
    if items.len() >= 3 && file_like.len().saturating_mul(2) > items.len() {
        return OutputContractVerdict::Reject {
            reason: "directory_names: candidate mostly contains file-like entries".to_string(),
        };
    }
    let locator_hint = contract.locator_hint.trim();
    if !locator_hint.is_empty()
        && items.len() == 1
        && items[0].contains(locator_hint)
        && file_like.len() == 1
    {
        return OutputContractVerdict::Reject {
            reason: "directory_names: locator candidate is file-like".to_string(),
        };
    }
    OutputContractVerdict::Pass
}

/// existence_with_path 的正/否、路径是否必须出现，都是语义判断。
/// 不再用本地 yes/no 词表或描述词硬裁决；prompt/composer 负责按 contract 输出。
fn verify_existence_with_path(
    _contract: &IntentOutputContract,
    _text: &str,
) -> OutputContractVerdict {
    OutputContractVerdict::Pass
}

/// scalar_path_only：回答应该就是一个路径/locator 字面。
/// - 不含路径/locator marker → Reject；
/// - 含 path-like token + 无关 prose → Reshape 抽出第一个 path token。
fn verify_scalar_path_only(contract: &IntentOutputContract, text: &str) -> OutputContractVerdict {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "scalar_path_only: empty candidate".to_string(),
        };
    }
    if let Some(token) = first_path_like_token(trimmed) {
        if token != trimmed {
            return OutputContractVerdict::Reshape {
                reason: "scalar_path_only: extracted first path token".to_string(),
                reshaped: token,
            };
        }
        return OutputContractVerdict::Pass;
    }
    if !contains_path_or_locator(trimmed, &contract.locator_hint) {
        return OutputContractVerdict::Reject {
            reason: "scalar_path_only: candidate does not contain a path or locator token"
                .to_string(),
        };
    }
    OutputContractVerdict::Pass
}

/// scalar_count：回答里至少要有一个整数字面（或纯数字 candidate）。
/// 候选文本中只有一个唯一整数值 → Reshape 取该整数；完全无整数 → Reject。
fn verify_scalar_count(contract: &IntentOutputContract, text: &str) -> OutputContractVerdict {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "scalar_count: empty candidate".to_string(),
        };
    }
    if scalar_count_candidate_is_structural_unavailable_result(trimmed, &contract.locator_hint) {
        return OutputContractVerdict::Pass;
    }
    let integers = trimmed
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let Some(first_int) = integers.first().copied() else {
        return OutputContractVerdict::Reject {
            reason: "scalar_count: candidate contains no integer literal".to_string(),
        };
    };
    if trimmed == first_int {
        return OutputContractVerdict::Pass;
    }
    if integers.iter().all(|candidate| *candidate == first_int) {
        return OutputContractVerdict::Reshape {
            reason: "scalar_count: extracted only unique integer from candidate".to_string(),
            reshaped: first_int.to_string(),
        };
    }
    OutputContractVerdict::Pass
}

fn scalar_count_candidate_is_structural_unavailable_result(text: &str, locator_hint: &str) -> bool {
    let trimmed = text.trim();
    if trimmed == "<missing>" || trimmed.ends_with(": <missing>") {
        return true;
    }
    let hint = locator_hint.trim();
    if hint.is_empty() || !trimmed.contains(hint) {
        return false;
    }
    let outside_hint = trimmed.replace(hint, "");
    !outside_hint.chars().any(|ch| ch.is_ascii_digit())
}

fn verify_hidden_entries_check(
    _contract: &IntentOutputContract,
    text: &str,
) -> OutputContractVerdict {
    if text.trim().is_empty() {
        return OutputContractVerdict::Reject {
            reason: "hidden_entries_check: empty candidate".to_string(),
        };
    }
    // 正/否和示例是否充分属于语义输出质量，交给 composer/prompt。
    OutputContractVerdict::Pass
}

/// §7.1 verifier 主入口：保守路线，只拦最严重结构 anti-pattern。
pub(crate) fn verify_output_contract(
    contract: &IntentOutputContract,
    candidate: &str,
    user_request: &str,
) -> OutputContractVerdict {
    let trimmed_candidate = candidate.trim();
    if trimmed_candidate.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "candidate is empty".to_string(),
        };
    }

    // 默认契约（response_shape=Free + semantic_kind=None）不强制任何形状，直接 Pass。
    if matches!(contract.response_shape, OutputResponseShape::Free)
        && matches!(contract.semantic_kind, OutputSemanticKind::None)
    {
        return OutputContractVerdict::Pass;
    }

    match contract.semantic_kind {
        OutputSemanticKind::ExistenceWithPath => {
            verify_existence_with_path(contract, trimmed_candidate)
        }
        OutputSemanticKind::ScalarPathOnly
            if matches!(contract.response_shape, OutputResponseShape::Scalar) =>
        {
            verify_scalar_path_only(contract, trimmed_candidate)
        }
        OutputSemanticKind::ScalarPathOnly => {
            if contains_path_or_locator(trimmed_candidate, &contract.locator_hint) {
                OutputContractVerdict::Pass
            } else {
                OutputContractVerdict::Reject {
                    reason: "scalar_path_only: non-scalar candidate does not contain a path or locator token"
                        .to_string(),
                }
            }
        }
        OutputSemanticKind::HiddenEntriesCheck => {
            verify_hidden_entries_check(contract, trimmed_candidate)
        }
        OutputSemanticKind::DirectoryNames => {
            verify_directory_names(contract, trimmed_candidate, user_request)
        }
        OutputSemanticKind::ScalarCount => verify_scalar_count(contract, trimmed_candidate),
        OutputSemanticKind::RecentScalarEqualityCheck => OutputContractVerdict::Pass,
        _ => OutputContractVerdict::Pass,
    }
}

#[cfg(test)]
#[path = "output_contract_verifier_tests.rs"]
mod tests;
