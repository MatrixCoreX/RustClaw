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

use crate::{IntentOutputContract, OutputResponseShape};

/// §7.1 verifier 判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutputContractVerdict {
    /// candidate 已满足 contract，直接放行。
    Pass,
    /// candidate **基本** 满足 contract，但有可程序化修复的 deviation；
    /// `reshaped` 是修复后的文本，调用方应直接用它替换原 candidate。
    Reshape {
        reason_code: &'static str,
        reason: String,
        reshaped: String,
    },
    /// candidate **明显** 违反 contract，且无法程序化修复：
    /// scalar path/count 缺少对应结构值。
    /// 调用方应丢弃 candidate，走 §7.2 `VerifyRejected` fallback。
    Reject {
        reason_code: &'static str,
        reason: String,
    },
}

impl OutputContractVerdict {
    pub(crate) const OWNER_LAYER: &'static str = "output_contract_verifier";

    pub(crate) fn reshape(
        reason_code: &'static str,
        reason: impl Into<String>,
        reshaped: impl Into<String>,
    ) -> Self {
        Self::Reshape {
            reason_code,
            reason: reason.into(),
            reshaped: reshaped.into(),
        }
    }

    pub(crate) fn reject(reason_code: &'static str, reason: impl Into<String>) -> Self {
        Self::Reject {
            reason_code,
            reason: reason.into(),
        }
    }

    pub(crate) fn owner_layer(&self) -> &'static str {
        Self::OWNER_LAYER
    }

    #[cfg(test)]
    pub(crate) fn reason_code(&self) -> Option<&'static str> {
        match self {
            Self::Pass => None,
            Self::Reshape { reason_code, .. } | Self::Reject { reason_code, .. } => {
                Some(*reason_code)
            }
        }
    }
}

/// scalar_count：回答里至少要有一个整数字面（或纯数字 candidate）。
/// 候选文本中只有一个唯一整数值 → Reshape 取该整数；完全无整数 → Reject。
fn verify_scalar_count(contract: &IntentOutputContract, text: &str) -> OutputContractVerdict {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return OutputContractVerdict::reject(
            "scalar_count_empty_candidate",
            "scalar_count: empty candidate",
        );
    }
    if scalar_count_candidate_is_structural_unavailable_result(trimmed, &contract.locator_hint) {
        return OutputContractVerdict::Pass;
    }
    let integers = trimmed
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let Some(first_int) = integers.first().copied() else {
        return OutputContractVerdict::reject(
            "scalar_count_missing_integer_literal",
            "scalar_count: candidate contains no integer literal",
        );
    };
    if trimmed == first_int {
        return OutputContractVerdict::Pass;
    }
    if integers.iter().all(|candidate| *candidate == first_int) {
        return OutputContractVerdict::reshape(
            "scalar_count_extracted_unique_integer",
            "scalar_count: extracted only unique integer from candidate",
            first_int.to_string(),
        );
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

/// §7.1 verifier 主入口：保守路线，只拦最严重结构 anti-pattern。
pub(crate) fn verify_output_contract(
    contract: &IntentOutputContract,
    candidate: &str,
    _user_request: &str,
) -> OutputContractVerdict {
    let trimmed_candidate = candidate.trim();
    if trimmed_candidate.is_empty() {
        return OutputContractVerdict::reject("candidate_empty", "candidate is empty");
    }

    // 默认契约（response_shape=Free + contract_marker=None）不强制任何形状，直接 Pass。
    if matches!(contract.response_shape, OutputResponseShape::Free)
        && contract.does_not_request_exact_command_output()
    {
        return OutputContractVerdict::Pass;
    }

    if contract.requests_exact_count() {
        return verify_scalar_count(contract, trimmed_candidate);
    }
    OutputContractVerdict::Pass
}

#[cfg(test)]
#[path = "output_contract_verifier_tests.rs"]
mod tests;
