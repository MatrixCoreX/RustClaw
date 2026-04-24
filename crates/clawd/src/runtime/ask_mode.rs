//! Phase 3.2 路由模式二元收敛（mode collapse）。
//!
//! 把原来散落的 `RoutedMode` (Chat/Act/ChatAct/AskClarify) 与两个 resume flag
//! (`direct_resume_discussion` / `direct_resume_execution`)
//! 收敛成一个二元 `AskMode` 枚举：
//!
//! - `ClarifyOrChat { entry }` —— 所有"对用户输出文本"的入口
//! - `Act { finalize }` —— 所有"调技能/工具"的入口
//!
//! 设计文档：[`docs/p32_mode_collapse_proposal.md`](../../../../docs/p32_mode_collapse_proposal.md)。
//!
//! Stage A 只引入抽象 + 转换函数，**不改任何现有调用面**；Stage B 起在
//! `RouteResult` / `PreparedAskRouting` 双轨携带，Stage C 逐文件切换 match，
//! Stage D 最终删除 `routed_mode` / `direct_resume_*`。

// Stage A 期间所有公开 API 都还没人调用；Stage B 起会被 RouteResult 等使用。
#![allow(dead_code)]

use super::types::{RouteGateKind, RoutedMode};

/// 二元收敛后的 ask 模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskMode {
    /// 对用户输出文本的入口（不调技能）。
    ClarifyOrChat { entry: ChatEntryStrategy },
    /// 调技能 / agent loop 的入口。
    Act { finalize: ActFinalizeStyle },
}

/// `ClarifyOrChat` 的入口策略，决定上下文载入方式与 prompt 选择。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatEntryStrategy {
    /// 原 `RoutedMode::Chat`：normalizer 标 mode=Chat，走标准 chat 直答。
    NormalizerThenChat,
    /// 原 `RoutedMode::AskClarify`：normalizer 标 needs_clarify=true，走反问。
    NormalizerThenClarify,
    /// 原 `direct_resume_discussion=true`：resume 上下文 + followup discussion prompt。
    ResumeFollowupDiscussion,
}

/// `Act` 的收尾风格，决定 agent loop 跑完后如何包装最终回复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActFinalizeStyle {
    /// 原 `RoutedMode::Act`：直接用 loop 产物作为最终回复。
    Plain,
    /// 原 `RoutedMode::ChatAct`：loop 跑完后再用 chat finalizer 包装一层。
    ChatWrapped,
    /// 原 `direct_resume_execution=true`：复用上次 plan，跳过 normalize/plan 阶段。
    ResumeContinue,
}

impl AskMode {
    /// 从历史的 (RoutedMode, resume flag) 组合构造 `AskMode`。
    ///
    /// 在 Stage B 双轨期，`intent_router` 与 `worker/ask_prepare` 计算完旧字段后
    /// 立即调用此函数填充 `ask_mode` 字段。
    ///
    /// 优先级（高 → 低，互斥）：
    /// 1. `direct_resume_execution=true` → `Act { ResumeContinue }`
    /// 2. `direct_resume_discussion=true` → `ClarifyOrChat { ResumeFollowupDiscussion }`
    /// 3. 否则按 `RoutedMode` 直接映射
    ///
    /// Plan §3.2 映射表见 [proposal §等价映射表](../../../../docs/p32_mode_collapse_proposal.md)。
    pub(crate) fn from_legacy(
        routed: RoutedMode,
        direct_resume_discussion: bool,
        direct_resume_execution: bool,
    ) -> Self {
        if direct_resume_execution {
            return AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            };
        }
        if direct_resume_discussion {
            return AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            };
        }
        AskMode::from_routed_mode(routed)
    }

    /// 纯 `RoutedMode` → `AskMode` 的最小映射，不考虑任何 flag。
    pub(crate) fn from_routed_mode(routed: RoutedMode) -> Self {
        match routed {
            RoutedMode::Chat => AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            },
            RoutedMode::AskClarify => AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            },
            RoutedMode::Act => AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            },
            RoutedMode::ChatAct => AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            },
        }
    }

    /// 反向回退到 `RoutedMode`，给"还没切换到 AskMode"的下游代码喂值。
    ///
    /// Stage D 完成后此函数可删（双轨结束）。
    pub(crate) fn to_routed_mode(&self) -> RoutedMode {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            } => RoutedMode::Chat,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            } => RoutedMode::AskClarify,
            // resume_followup_discussion 历史上跟 RoutedMode::Chat 共存。
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            } => RoutedMode::Chat,
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            } => RoutedMode::Act,
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            } => RoutedMode::ChatAct,
            // resume_continue 历史上跟 RoutedMode::Act/ChatAct 都可能共存；
            // 反向取 Act（更保守，Plain finalize）。
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            } => RoutedMode::Act,
        }
    }

    pub(crate) fn is_act(&self) -> bool {
        self.is_execute_gate()
    }

    pub(crate) fn gate_kind(&self) -> RouteGateKind {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            } => RouteGateKind::Clarify,
            AskMode::ClarifyOrChat { .. } => RouteGateKind::Chat,
            AskMode::Act { .. } => RouteGateKind::Execute,
        }
    }

    pub(crate) fn is_execute_gate(&self) -> bool {
        matches!(self.gate_kind(), RouteGateKind::Execute)
    }

    pub(crate) fn is_chat_gate(&self) -> bool {
        matches!(self.gate_kind(), RouteGateKind::Chat)
    }

    pub(crate) fn is_clarify_gate(&self) -> bool {
        matches!(self.gate_kind(), RouteGateKind::Clarify)
    }

    /// 等价于历史 `route.routed_mode == RoutedMode::Act`：
    /// 仅命中 `Act { Plain }`，不包括 `ChatWrapped` / `ResumeContinue`。
    pub(crate) fn is_plain_act(&self) -> bool {
        matches!(
            self,
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            }
        )
    }

    pub(crate) fn is_clarify_only(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            }
        )
    }

    /// 等价于历史 `route.routed_mode == RoutedMode::Chat`：
    /// 仅命中 `ClarifyOrChat { NormalizerThenChat }`，不命中
    /// ResumeFollowupDiscussion 这种"形式上也算 Chat 但语义不同"的 entry。
    pub(crate) fn is_normalizer_chat(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            }
        )
    }

    pub(crate) fn is_resume_discussion(&self) -> bool {
        matches!(
            self,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            }
        )
    }

    pub(crate) fn finalize_chat_wrapped(&self) -> bool {
        matches!(
            self,
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            }
        )
    }

    pub(crate) fn resume_execution(&self) -> bool {
        matches!(
            self,
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            }
        )
    }

    /// Stable string id for logging / journal payloads.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat,
            } => "clarify_or_chat:normalizer_chat",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify,
            } => "clarify_or_chat:normalizer_clarify",
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::ResumeFollowupDiscussion,
            } => "clarify_or_chat:resume_followup_discussion",
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain,
            } => "act:plain",
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped,
            } => "act:chat_wrapped",
            AskMode::Act {
                finalize: ActFinalizeStyle::ResumeContinue,
            } => "act:resume_continue",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_chat_maps_to_normalizer_chat() {
        let m = AskMode::from_legacy(RoutedMode::Chat, false, false);
        assert_eq!(
            m,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat
            }
        );
        assert_eq!(m.to_routed_mode(), RoutedMode::Chat);
    }

    #[test]
    fn legacy_ask_clarify_maps_to_normalizer_clarify() {
        let m = AskMode::from_legacy(RoutedMode::AskClarify, false, false);
        assert_eq!(
            m,
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify
            }
        );
        assert!(m.is_clarify_only());
        assert!(!m.is_act());
        assert_eq!(m.to_routed_mode(), RoutedMode::AskClarify);
    }

    #[test]
    fn legacy_resume_discussion_wins_over_routed_mode() {
        let m = AskMode::from_legacy(RoutedMode::Chat, true, false);
        assert!(m.is_resume_discussion());
        assert_eq!(m.to_routed_mode(), RoutedMode::Chat);
    }

    #[test]
    fn legacy_resume_execution_wins_over_everything() {
        // resume_execution 优先级最高，因为它是"复用上次 plan，立刻执行"。
        let m = AskMode::from_legacy(RoutedMode::Chat, true, true);
        assert!(m.resume_execution());
        assert!(m.is_act());
        assert_eq!(m.to_routed_mode(), RoutedMode::Act);
    }

    #[test]
    fn legacy_act_maps_to_plain() {
        let m = AskMode::from_legacy(RoutedMode::Act, false, false);
        assert_eq!(
            m,
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain
            }
        );
        assert!(m.is_act());
        assert!(!m.finalize_chat_wrapped());
        assert_eq!(m.to_routed_mode(), RoutedMode::Act);
    }

    #[test]
    fn legacy_chat_act_maps_to_chat_wrapped() {
        let m = AskMode::from_legacy(RoutedMode::ChatAct, false, false);
        assert_eq!(
            m,
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped
            }
        );
        assert!(m.is_act());
        assert!(m.finalize_chat_wrapped());
        assert_eq!(m.to_routed_mode(), RoutedMode::ChatAct);
    }

    #[test]
    fn from_routed_mode_pure_mapping_ignores_flags() {
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::Chat),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenChat
            }
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::AskClarify),
            AskMode::ClarifyOrChat {
                entry: ChatEntryStrategy::NormalizerThenClarify
            }
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::Act),
            AskMode::Act {
                finalize: ActFinalizeStyle::Plain
            }
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::ChatAct),
            AskMode::Act {
                finalize: ActFinalizeStyle::ChatWrapped
            }
        );
    }

    #[test]
    fn round_trip_for_pure_routed_modes() {
        for routed in [
            RoutedMode::Chat,
            RoutedMode::Act,
            RoutedMode::ChatAct,
            RoutedMode::AskClarify,
        ] {
            let m = AskMode::from_routed_mode(routed);
            assert_eq!(
                m.to_routed_mode(),
                routed,
                "round trip failed for {routed:?}"
            );
        }
    }

    #[test]
    fn as_str_uses_stable_ids() {
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::Chat).as_str(),
            "clarify_or_chat:normalizer_chat"
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::AskClarify).as_str(),
            "clarify_or_chat:normalizer_clarify"
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::Act).as_str(),
            "act:plain"
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::ChatAct).as_str(),
            "act:chat_wrapped"
        );
        let rd = AskMode::from_legacy(RoutedMode::Chat, true, false);
        assert_eq!(rd.as_str(), "clarify_or_chat:resume_followup_discussion");
        let re = AskMode::from_legacy(RoutedMode::Chat, false, true);
        assert_eq!(re.as_str(), "act:resume_continue");
    }

    #[test]
    fn is_plain_act_only_for_plain_finalize() {
        assert!(AskMode::from_routed_mode(RoutedMode::Act).is_plain_act());
        assert!(!AskMode::from_routed_mode(RoutedMode::ChatAct).is_plain_act());
        assert!(!AskMode::from_routed_mode(RoutedMode::Chat).is_plain_act());
        assert!(!AskMode::from_routed_mode(RoutedMode::AskClarify).is_plain_act());
        let resume = AskMode::from_legacy(RoutedMode::Act, false, true);
        assert!(!resume.is_plain_act(), "ResumeContinue must not be plain");
        assert!(resume.is_act());
    }

    #[test]
    fn helpers_are_disjoint_for_each_variant() {
        let cases = [
            AskMode::from_routed_mode(RoutedMode::Chat),
            AskMode::from_routed_mode(RoutedMode::AskClarify),
            AskMode::from_routed_mode(RoutedMode::Act),
            AskMode::from_routed_mode(RoutedMode::ChatAct),
            AskMode::from_legacy(RoutedMode::Chat, true, false),
            AskMode::from_legacy(RoutedMode::Chat, false, true),
        ];
        for m in &cases {
            let mut hits = 0;
            if m.is_clarify_only() {
                hits += 1;
            }
            if m.is_resume_discussion() {
                hits += 1;
            }
            if m.finalize_chat_wrapped() {
                hits += 1;
            }
            if m.resume_execution() {
                hits += 1;
            }
            // 普通 Chat / Plain Act 0 个谓词命中；其他正好命中 1 个。
            assert!(hits <= 1, "predicate overlap on {m:?} (hits={hits})");
        }
    }

    #[test]
    fn gate_kind_collapses_legacy_modes_to_three_gates() {
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::Chat).gate_kind(),
            RouteGateKind::Chat
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::AskClarify).gate_kind(),
            RouteGateKind::Clarify
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::Act).gate_kind(),
            RouteGateKind::Execute
        );
        assert_eq!(
            AskMode::from_routed_mode(RoutedMode::ChatAct).gate_kind(),
            RouteGateKind::Execute
        );
    }

    #[test]
    fn legacy_shortcuts_map_to_expected_gate_kinds() {
        assert_eq!(
            AskMode::from_legacy(RoutedMode::Chat, true, false).gate_kind(),
            RouteGateKind::Chat
        );
        assert_eq!(
            AskMode::from_legacy(RoutedMode::Chat, false, true).gate_kind(),
            RouteGateKind::Execute
        );
    }
}
