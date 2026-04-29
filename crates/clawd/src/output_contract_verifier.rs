//! §7.1 Output-contract verifier：finalize 阶段把 candidate 文本拿来对照
//! [`crate::IntentOutputContract`] 做"语义合规性"判定，是 [`crate::delivery_utils::output_contract::enforce_output_contract`] "shape 整形" 之上的语义层。
//!
//! 设计原则（保守版 v1）：
//! - 只拦"明显违反 contract"的最严重 anti-pattern，宁可漏拦也不要误拦合规答案
//!   （由 [`OutputContractVerdict`] 的语义保证：Pass / Reshape / Reject 三态，没有
//!   "Almost-Pass" 这种灰色态）。
//! - **优先程序化 reshape** —— 比如 runtime synthesis 已经在回复里给出了路径、只是缺
//!   yes/no 前缀，这种能直接补；只有完全缺关键证据时才 Reject。
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
    /// candidate **明显** 违反 contract，且无法程序化修复（典型：
    /// existence_with_path 答成了纯描述句，没有 yes/no 也没有路径）。
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

/// 语义级 yes/no 词典（中英 + ascii 大小写）。命中即认为有 existence verdict。
fn contains_existence_yes_token(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    // ASCII：用单词边界感避免 "found" 误匹配 "background"，但保守起见用 contains，
    // 因为短答场景里此类碰撞极罕见，且 verifier 是"放行偏多"的保守路线。
    const ASCII: &[&str] = &["yes", "exists", "found", "present"];
    if ASCII.iter().any(|tok| lower.contains(tok)) {
        return true;
    }
    // 中文：用字面 contains，避免词性切分。
    const ZH: &[&str] = &["有", "存在", "找到了", "找到", "已找到"];
    ZH.iter().any(|tok| t.contains(tok))
}

fn contains_existence_no_token(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    const ASCII: &[&str] = &[
        "no\n",
        "no.",
        "no,",
        "no ",
        "missing",
        "not found",
        "absent",
    ];
    // 单独处理"裸 no"边界：结尾 / 全字符串。
    if lower == "no"
        || lower.starts_with("no ")
        || lower.starts_with("no.")
        || lower.starts_with("no,")
    {
        return true;
    }
    if ASCII.iter().any(|tok| lower.contains(tok)) {
        return true;
    }
    const ZH: &[&str] = &["没有", "不存在", "未找到", "找不到", "无此", "查无"];
    ZH.iter().any(|tok| t.contains(tok))
}

fn contains_equality_same_token(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    const ASCII: &[&str] = &["same", "equal", "identical", "matches"];
    if ASCII.iter().any(|tok| lower.contains(tok)) {
        return true;
    }
    const ZH: &[&str] = &["一样", "相同", "一致", "相等"];
    ZH.iter().any(|tok| t.contains(tok))
}

fn contains_equality_different_token(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    const ASCII: &[&str] = &["different", "not the same", "unequal", "does not match"];
    if ASCII.iter().any(|tok| lower.contains(tok)) {
        return true;
    }
    const ZH: &[&str] = &["不一样", "不同", "不相同", "不一致", "不相等"];
    ZH.iter().any(|tok| t.contains(tok))
}

/// "回答里看起来含路径子串"的 heuristic：
/// - 含 `/`（绝对/相对路径或 URL 风），或
/// - 含 `\`（Windows 路径），或
/// - locator_hint 非空且回答里出现该 hint 字面。
fn looks_like_contains_path(text: &str, locator_hint: &str) -> bool {
    let t = text.trim();
    if t.contains('/') || t.contains('\\') {
        return true;
    }
    let hint = locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    t.contains(hint)
}

/// 看起来是"段落式描述句"：
/// - ≥ 2 个非空行，或
/// - 单行 ≥ 60 chars 且含至少一个明显 description marker（"是…文件"/"看起来像"/"是一个"/"this is a"/"appears to be"/"systemd"/"用于"/"主要"）。
///
/// 这类候选若在 ExistenceWithPath / ScalarPathOnly contract 下出现且缺 yes/no token，
/// 就是 act_find_service_file 那类失败模式 —— 强 reject。
fn looks_like_description_paragraph(text: &str) -> bool {
    let trimmed = text.trim();
    let line_count = trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    if line_count >= 2 {
        return true;
    }
    if trimmed.chars().count() < 60 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    const ASCII_MARKERS: &[&str] = &[
        "this is a",
        "appears to be",
        "looks like",
        "is a systemd",
        "is the ",
    ];
    if ASCII_MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    const ZH_MARKERS: &[&str] = &[
        "是一个",
        "看起来像",
        "应该是",
        "这是",
        "是 systemd",
        "用于",
        "主要",
        "通常",
    ];
    ZH_MARKERS.iter().any(|m| trimmed.contains(m))
}

/// existence_with_path 类（典型 act_find_service_file）的 verify。
/// - 必须含 yes/no token；
/// - yes 分支额外要求含路径子串；
/// - 段落式描述但缺 yes/no → Reject（不可修，证据不足）；
/// - 含路径但缺 yes/no 前缀 → Reshape，自动加"有，"前缀。
fn verify_existence_with_path(
    contract: &IntentOutputContract,
    text: &str,
) -> OutputContractVerdict {
    let has_yes = contains_existence_yes_token(text);
    let has_no = contains_existence_no_token(text);
    let has_path = looks_like_contains_path(text, &contract.locator_hint);

    if has_yes && has_path {
        return OutputContractVerdict::Pass;
    }
    if has_no {
        // "没有 / 未找到 …" 这种否定回答允许不带路径（路径不存在自然没法给）。
        return OutputContractVerdict::Pass;
    }

    // 没有 yes/no token，但确实有路径 → 程序化 Reshape 加 "有，" 前缀。
    if !has_yes && !has_no && has_path && !looks_like_description_paragraph(text) {
        let reshaped = format!("有，{}", text.trim());
        return OutputContractVerdict::Reshape {
            reason:
                "existence_with_path: candidate has path but missing yes/no prefix; auto-prepended"
                    .to_string(),
            reshaped,
        };
    }

    // 段落式描述但既没 yes/no 又没路径——典型"chat 把'有没有'答成'这是 systemd 文件'"
    // 模式。无可挽救，Reject。
    if looks_like_description_paragraph(text) || (!has_path) {
        return OutputContractVerdict::Reject {
            reason: format!(
                "existence_with_path: candidate violates contract (yes={has_yes}, no={has_no}, has_path={has_path}, looks_paragraph={})",
                looks_like_description_paragraph(text)
            ),
        };
    }

    // 含路径不像段落，但既无 yes 也无 no —— Reshape 加 "有，"。
    OutputContractVerdict::Reshape {
        reason: "existence_with_path: candidate missing yes/no token; auto-prepended".to_string(),
        reshaped: format!("有，{}", text.trim()),
    }
}

/// scalar_path_only：回答应该就是一个路径字面，最多含一两个分隔符／引号；
/// - 多行段落 → Reject；
/// - 不含路径 marker → Reject；
/// - 含路径 + 无关 prose → Reshape 抽出第一个 path-like token。
fn verify_scalar_path_only(contract: &IntentOutputContract, text: &str) -> OutputContractVerdict {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "scalar_path_only: empty candidate".to_string(),
        };
    }
    if !looks_like_contains_path(trimmed, &contract.locator_hint) {
        return OutputContractVerdict::Reject {
            reason: "scalar_path_only: candidate does not contain a path token".to_string(),
        };
    }
    if looks_like_description_paragraph(trimmed) {
        // 提取第一个路径 token 作 reshape 候选。
        if let Some(token) = first_path_like_token(trimmed) {
            return OutputContractVerdict::Reshape {
                reason: "scalar_path_only: candidate is a description paragraph; extracted first path token"
                    .to_string(),
                reshaped: token,
            };
        }
        return OutputContractVerdict::Reject {
            reason: "scalar_path_only: candidate is a description paragraph and no path token extractable".to_string(),
        };
    }
    OutputContractVerdict::Pass
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

/// scalar_count：回答里至少要有一个整数字面（或纯数字 candidate）。
/// 多句段落但确实含整数 → Reshape 取第一个整数；完全无整数 → Reject。
fn verify_scalar_count(text: &str) -> OutputContractVerdict {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return OutputContractVerdict::Reject {
            reason: "scalar_count: empty candidate".to_string(),
        };
    }
    let first_int = trimmed
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty());
    let Some(int_lit) = first_int else {
        return OutputContractVerdict::Reject {
            reason: "scalar_count: candidate contains no integer literal".to_string(),
        };
    };
    if trimmed == int_lit {
        return OutputContractVerdict::Pass;
    }
    if looks_like_description_paragraph(trimmed) {
        return OutputContractVerdict::Reshape {
            reason: "scalar_count: candidate is a paragraph; extracted first integer".to_string(),
            reshaped: int_lit.to_string(),
        };
    }
    OutputContractVerdict::Pass
}

/// hidden_entries_check / recent_scalar_equality_check：回答必须含 yes/no token。
fn looks_like_hidden_entries_evidence(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.contains("隐藏") || trimmed.to_ascii_lowercase().contains("hidden") {
        return true;
    }
    trimmed
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '，' | ';' | '；' | '、'))
        .map(|token| {
            token.trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '。'
                )
            })
        })
        .any(|token| {
            token.len() > 1
                && token.starts_with('.')
                && token
                    .chars()
                    .nth(1)
                    .map(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    .unwrap_or(false)
        })
}

fn verify_hidden_entries_check(
    _contract: &IntentOutputContract,
    text: &str,
) -> OutputContractVerdict {
    if contains_existence_yes_token(text)
        || contains_existence_no_token(text)
        || looks_like_hidden_entries_evidence(text)
    {
        return OutputContractVerdict::Pass;
    }
    OutputContractVerdict::Reject {
        reason: "hidden_entries_check: candidate lacks hidden-entry evidence".to_string(),
    }
}

fn verify_same_or_different_only(text: &str, kind_label: &str) -> OutputContractVerdict {
    let has_same = contains_equality_same_token(text);
    let has_different = contains_equality_different_token(text);
    if has_same || has_different {
        return OutputContractVerdict::Pass;
    }
    OutputContractVerdict::Reject {
        reason: format!(
            "{kind_label}: candidate missing same/different token; cannot be salvaged programmatically"
        ),
    }
}

/// §7.1 verifier 主入口：保守路线，只拦最严重 anti-pattern。
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

    // 优先按 semantic_kind 选 verifier；其它 kind 暂不强约束（v2 再扩）。
    match contract.semantic_kind {
        OutputSemanticKind::ExistenceWithPath => {
            verify_existence_with_path(contract, trimmed_candidate)
        }
        OutputSemanticKind::ScalarPathOnly => verify_scalar_path_only(contract, trimmed_candidate),
        OutputSemanticKind::HiddenEntriesCheck => {
            verify_hidden_entries_check(contract, trimmed_candidate)
        }
        OutputSemanticKind::RecentScalarEqualityCheck => {
            verify_same_or_different_only(trimmed_candidate, "recent_scalar_equality_check")
        }
        OutputSemanticKind::ScalarCount => verify_scalar_count(trimmed_candidate),
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
    fn recent_scalar_equality_accepts_chinese_same_or_different_tokens() {
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
            verify_output_contract(&contract, "前者和后者一样", ""),
            OutputContractVerdict::Pass
        );
    }

    #[test]
    fn reject_for_empty_candidate() {
        let v = verify_output_contract(&contract_existence("rustclaw.service"), "  ", "?");
        assert!(matches!(v, OutputContractVerdict::Reject { .. }));
    }

    #[test]
    fn pass_existence_with_yes_and_path() {
        // act_find_service_file 的"理想答案"：有 + 路径。
        let v = verify_output_contract(
            &contract_existence("rustclaw.service"),
            "有，rustclaw.service：/home/guagua/rustclaw/rustclaw.service",
            "检查仓库里有没有 rustclaw.service",
        );
        assert_eq!(v, OutputContractVerdict::Pass);
    }

    #[test]
    fn pass_existence_with_no_only() {
        // 否定回答允许不带路径（路径不存在）。
        let v = verify_output_contract(
            &contract_existence("missing.txt"),
            "没有，仓库里没有这个文件。",
            "有没有 missing.txt？",
        );
        assert_eq!(v, OutputContractVerdict::Pass);
    }

    #[test]
    fn reshape_existence_when_path_present_but_no_yes_no_token() {
        // chat 给了路径但忘了 yes/no 前缀 —— 程序化补 "有，"。
        let v = verify_output_contract(
            &contract_existence("rustclaw.service"),
            "/home/guagua/rustclaw/rustclaw.service",
            "?",
        );
        match v {
            OutputContractVerdict::Reshape { reshaped, .. } => {
                assert!(reshaped.starts_with("有，"), "reshaped: {reshaped}");
                assert!(
                    reshaped.contains("rustclaw.service"),
                    "reshaped: {reshaped}"
                );
            }
            other => panic!("expected Reshape, got: {other:?}"),
        }
    }

    #[test]
    fn reject_existence_when_paragraph_description_without_yes_no() {
        // act_find_service_file 真实失败现场：chat 把"有没有"答成了"这是 systemd 文件…"
        // 既无 yes/no 又像段落 → Reject。
        let candidate = "这是一个 systemd 服务单元文件，用于在系统启动时拉起 rustclaw 守护进程。";
        let v = verify_output_contract(
            &contract_existence("rustclaw.service"),
            candidate,
            "检查仓库里有没有 rustclaw.service",
        );
        assert!(
            matches!(v, OutputContractVerdict::Reject { .. }),
            "expected Reject for description paragraph, got: {v:?}"
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
    fn reshape_scalar_path_only_extracts_path_from_paragraph() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            ..IntentOutputContract::default()
        };
        // 多行段落里有路径 → Reshape 抽 path。
        let candidate = "看起来这个文件是个配置文件。\n它的位置是 /etc/passwd 这条。";
        let v = verify_output_contract(&contract, candidate, "?");
        match v {
            OutputContractVerdict::Reshape { reshaped, .. } => {
                assert_eq!(reshaped, "/etc/passwd");
            }
            other => panic!("expected Reshape extracting path, got: {other:?}"),
        }
    }

    #[test]
    fn reject_scalar_path_only_when_no_path_at_all() {
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
    fn reshape_scalar_count_extracts_int_from_paragraph() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarCount,
            ..IntentOutputContract::default()
        };
        let candidate = "目录下大概有 5 个子项，看起来都是文档。\n所以一共是 5 个。";
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
    fn yes_no_only_kinds_pass_with_either_token() {
        let contract = IntentOutputContract {
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            ..IntentOutputContract::default()
        };
        assert_eq!(
            verify_output_contract(&contract, "没有隐藏目录", "?"),
            OutputContractVerdict::Pass
        );
        assert_eq!(
            verify_output_contract(&contract, "有 .git 一个隐藏目录", "?"),
            OutputContractVerdict::Pass
        );
        let v = verify_output_contract(&contract, "看起来一切正常", "?");
        assert!(matches!(v, OutputContractVerdict::Reject { .. }));
    }

    #[test]
    fn hidden_entries_check_accepts_explanatory_sentence_with_examples() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            ..IntentOutputContract::default()
        };
        assert_eq!(
            verify_output_contract(
                &contract,
                "The current directory has hidden files such as .git and .gitignore.",
                "check hidden files",
            ),
            OutputContractVerdict::Pass
        );
        assert_eq!(
            verify_output_contract(
                &contract,
                "当前目录存在隐藏文件（如 .git、.codex），通常用于保存元数据。",
                "检查隐藏文件",
            ),
            OutputContractVerdict::Pass
        );
    }

    #[test]
    fn hidden_entries_check_scalar_accepts_yes_no_with_examples() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            ..IntentOutputContract::default()
        };
        assert_eq!(
            verify_output_contract(
                &contract,
                "有。示例：.codex, .git/, .gitignore",
                "检查隐藏文件",
            ),
            OutputContractVerdict::Pass
        );
    }

    #[test]
    fn hidden_entries_count_uses_scalar_count_contract() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Scalar,
            semantic_kind: OutputSemanticKind::ScalarCount,
            ..IntentOutputContract::default()
        };
        assert_eq!(
            verify_output_contract(&contract, "4", "count hidden entries"),
            OutputContractVerdict::Pass
        );
        assert_eq!(
            verify_output_contract(
                &contract,
                "There are 4 hidden entries in this directory.",
                "count hidden entries",
            ),
            OutputContractVerdict::Pass
        );
    }
}
