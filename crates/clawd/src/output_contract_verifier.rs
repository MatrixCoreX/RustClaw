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

fn nonempty_line_count(text: &str) -> usize {
    text.lines().filter(|line| !line.trim().is_empty()).count()
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
/// 多行文本且只有一个整数 → Reshape 取该整数；完全无整数 → Reject。
fn verify_scalar_count(text: &str) -> OutputContractVerdict {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "scalar_count: empty candidate".to_string(),
        };
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
    if nonempty_line_count(trimmed) >= 2 && integers.len() == 1 {
        return OutputContractVerdict::Reshape {
            reason: "scalar_count: extracted sole integer from multi-line candidate".to_string(),
            reshaped: first_int.to_string(),
        };
    }
    OutputContractVerdict::Pass
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
    _user_request: &str,
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
        OutputSemanticKind::ScalarPathOnly => verify_scalar_path_only(contract, trimmed_candidate),
        OutputSemanticKind::HiddenEntriesCheck => {
            verify_hidden_entries_check(contract, trimmed_candidate)
        }
        OutputSemanticKind::ScalarCount => verify_scalar_count(trimmed_candidate),
        OutputSemanticKind::RecentScalarEqualityCheck => OutputContractVerdict::Pass,
        _ => OutputContractVerdict::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_existence(hint: &str) -> IntentOutputContract {
        IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: hint.to_string(),
            ..IntentOutputContract::default()
        }
    }

    #[test]
    fn pass_for_default_contract() {
        let v = verify_output_contract(&IntentOutputContract::default(), "anything goes", "what?");
        assert_eq!(v, OutputContractVerdict::Pass);
    }

    #[test]
    fn reject_for_empty_candidate() {
        let v = verify_output_contract(&contract_existence("rustclaw.service"), "  ", "?");
        assert!(matches!(v, OutputContractVerdict::Reject { .. }));
    }

    #[test]
    fn existence_with_path_no_longer_autoprepends_or_hard_rejects() {
        assert_eq!(
            verify_output_contract(
                &contract_existence("rustclaw.service"),
                "/home/guagua/rustclaw/rustclaw.service",
                "?",
            ),
            OutputContractVerdict::Pass
        );
        assert_eq!(
            verify_output_contract(
                &contract_existence("rustclaw.service"),
                "这是一个 systemd 服务单元文件，用于在系统启动时拉起 rustclaw 守护进程。",
                "检查仓库里有没有 rustclaw.service",
            ),
            OutputContractVerdict::Pass
        );
    }

    #[test]
    fn recent_scalar_equality_is_not_verified_by_local_text_tokens() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            semantic_kind: OutputSemanticKind::RecentScalarEqualityCheck,
            ..IntentOutputContract::default()
        };
        assert_eq!(
            verify_output_contract(&contract, "react-example、clawd、不一样", ""),
            OutputContractVerdict::Pass
        );
        assert_eq!(
            verify_output_contract(&contract, "needs composer judgment", ""),
            OutputContractVerdict::Pass
        );
    }

    #[test]
    fn pass_scalar_path_only_for_pure_path() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            ..IntentOutputContract::default()
        };
        let v = verify_output_contract(&contract, "/etc/passwd", "?");
        assert_eq!(v, OutputContractVerdict::Pass);
    }

    #[test]
    fn reshape_scalar_path_only_extracts_path_structurally() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            ..IntentOutputContract::default()
        };
        let candidate = "路径是 /etc/passwd。";
        let v = verify_output_contract(&contract, candidate, "?");
        match v {
            OutputContractVerdict::Reshape { reshaped, .. } => {
                assert_eq!(reshaped, "/etc/passwd");
            }
            other => panic!("expected Reshape extracting path, got: {other:?}"),
        }
    }

    #[test]
    fn reject_scalar_path_only_when_no_path_or_locator() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            ..IntentOutputContract::default()
        };
        let v = verify_output_contract(&contract, "我不知道在哪", "?");
        assert!(matches!(v, OutputContractVerdict::Reject { .. }));
    }

    #[test]
    fn pass_scalar_count_for_pure_integer() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarCount,
            ..IntentOutputContract::default()
        };
        let v = verify_output_contract(&contract, "3", "?");
        assert_eq!(v, OutputContractVerdict::Pass);
    }

    #[test]
    fn reshape_scalar_count_extracts_sole_int_from_multiline_candidate() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarCount,
            ..IntentOutputContract::default()
        };
        let candidate = "目录检查完成。\n一共是 5 个。";
        let v = verify_output_contract(&contract, candidate, "?");
        match v {
            OutputContractVerdict::Reshape { reshaped, .. } => assert_eq!(reshaped, "5"),
            other => panic!("expected Reshape extracting int, got: {other:?}"),
        }
    }

    #[test]
    fn reject_scalar_count_when_no_integer_at_all() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarCount,
            ..IntentOutputContract::default()
        };
        let v = verify_output_contract(&contract, "数不清", "?");
        assert!(matches!(v, OutputContractVerdict::Reject { .. }));
    }

    #[test]
    fn hidden_entries_check_no_longer_uses_yes_no_or_hidden_word_dictionary() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            ..IntentOutputContract::default()
        };
        assert_eq!(
            verify_output_contract(&contract, "看起来一切正常", "?"),
            OutputContractVerdict::Pass
        );
    }
}
