use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalStatus {
    Success,
    Failure,
    Clarify,
    ResumeFailure,
}

impl TaskJournalFinalStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Clarify => "clarify",
            Self::ResumeFailure => "resume_failure",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalizerStage {
    General,
    ObservedRead,
    ObservedListDir,
    ObservedGeneric,
}

impl TaskJournalFinalizerStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::ObservedRead => "observed_read",
            Self::ObservedListDir => "observed_list_dir",
            Self::ObservedGeneric => "observed_generic",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskJournalFinalizerFallback {
    RawText,
    NoAnswerNonQualified,
    NoAnswerParseFailed,
}

impl TaskJournalFinalizerFallback {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RawText => "raw_text",
            Self::NoAnswerNonQualified => "no_answer_nonqualified",
            Self::NoAnswerParseFailed => "no_answer_parse_failed",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalVerifyIssue {
    pub(crate) step_id: String,
    pub(crate) kind: crate::verifier::VerifyIssueKind,
    pub(crate) detail: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalVerifySummary {
    pub(crate) mode: crate::verifier::VerifyMode,
    pub(crate) approved: bool,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) shadow_blocked_reason: Option<String>,
    pub(crate) needs_confirmation: bool,
    pub(crate) issues: Vec<TaskJournalVerifyIssue>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalRoundTrace {
    pub(crate) round_no: usize,
    pub(crate) goal: String,
    pub(crate) execution_recipe_summary: Option<String>,
    pub(crate) plan_result: Option<crate::PlanResult>,
    pub(crate) verify_result: Option<TaskJournalVerifySummary>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalStepTrace {
    pub(crate) step_id: String,
    pub(crate) skill: String,
    pub(crate) status: crate::executor::StepExecutionStatus,
    pub(crate) output_excerpt: Option<String>,
    pub(crate) error_excerpt: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) finished_at: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalFinalizerSummary {
    pub(crate) stage: Option<TaskJournalFinalizerStage>,
    pub(crate) disposition: Option<crate::finalize::FinalizerDisposition>,
    pub(crate) fallback: Option<TaskJournalFinalizerFallback>,
    pub(crate) parsed: bool,
    pub(crate) contract_ok: bool,
    pub(crate) completion_ok: Option<bool>,
    pub(crate) grounded_ok: Option<bool>,
    pub(crate) format_ok: Option<bool>,
    pub(crate) needs_clarify: Option<bool>,
    pub(crate) confidence: Option<f64>,
    pub(crate) used_evidence_ids_count: usize,
    pub(crate) evidence_quotes_count: usize,
}

fn verify_summary_json(verify: &TaskJournalVerifySummary) -> Value {
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "needs_confirmation": verify.needs_confirmation,
        "issue_count": verify.issues.len(),
    })
}

fn verify_trace_json(verify: &TaskJournalVerifySummary) -> Value {
    json!({
        "approved": verify.approved,
        "mode": verify.mode.as_str(),
        "blocked_reason": verify.blocked_reason.as_deref().map(crate::truncate_for_log),
        "shadow_blocked_reason": verify.shadow_blocked_reason.as_deref().map(crate::truncate_for_log),
        "needs_confirmation": verify.needs_confirmation,
        "issues": verify.issues.iter().map(|issue| {
            json!({
                "step_id": &issue.step_id,
                "kind": issue.kind.as_str(),
                "detail": crate::truncate_for_log(&issue.detail),
            })
        }).collect::<Vec<_>>(),
    })
}

fn finalizer_summary_json(summary: &TaskJournalFinalizerSummary) -> Value {
    json!({
        "stage": summary.stage.map(TaskJournalFinalizerStage::as_str),
        "disposition": summary.disposition.map(crate::finalize::FinalizerDisposition::as_str),
        "fallback": summary.fallback.map(TaskJournalFinalizerFallback::as_str),
        "parsed": summary.parsed,
        "contract_ok": summary.contract_ok,
        "completion_ok": summary.completion_ok,
        "grounded_ok": summary.grounded_ok,
        "format_ok": summary.format_ok,
        "needs_clarify": summary.needs_clarify,
        "confidence": summary.confidence,
        "used_evidence_ids_count": summary.used_evidence_ids_count,
        "evidence_quotes_count": summary.evidence_quotes_count,
    })
}

fn plan_summary_json(plan: &crate::PlanResult) -> Value {
    json!({
        "goal": crate::truncate_for_log(&plan.goal),
        "plan_kind": plan.plan_kind.as_str(),
        "step_count": plan.steps.len(),
        "missing_slots": plan.missing_slots,
        "needs_confirmation": plan.needs_confirmation,
    })
}

fn plan_trace_json(plan: &crate::PlanResult) -> Value {
    json!({
        "goal": crate::truncate_for_log(&plan.goal),
        "plan_kind": plan.plan_kind.as_str(),
        "planner_notes": crate::truncate_for_log(&plan.planner_notes),
        "raw_plan_text": crate::truncate_for_log(&plan.raw_plan_text),
        "step_count": plan.steps.len(),
        "steps": plan.steps.iter().map(|step| {
            json!({
                "step_id": &step.step_id,
                "action_type": &step.action_type,
                "skill": &step.skill,
                "depends_on": &step.depends_on,
            })
        }).collect::<Vec<_>>(),
    })
}

fn route_result_json(route: &crate::RouteResult) -> Value {
    json!({
        "routed_mode": route.routed_mode.as_str(),
        "needs_clarify": route.needs_clarify,
        "should_refresh_long_term_memory": route.should_refresh_long_term_memory,
        "agent_display_name_hint": route.agent_display_name_hint,
        "route_reason": crate::truncate_for_log(&route.route_reason),
        "risk_ceiling": route.risk_ceiling.as_str(),
        "self_extension": {
            "mode": route.output_contract.self_extension.mode.as_str(),
            "trigger": route.output_contract.self_extension.trigger.as_str(),
            "execute_now": route.output_contract.self_extension.execute_now,
        },
    })
}

fn step_trace_json(step: &TaskJournalStepTrace) -> Value {
    json!({
        "step_id": &step.step_id,
        "skill": &step.skill,
        "status": step.status.as_str(),
        "output_excerpt": step.output_excerpt.as_deref(),
        "error_excerpt": step.error_excerpt.as_deref(),
        "started_at": step.started_at,
        "finished_at": step.finished_at,
    })
}

/// §3.1: 单条 ask 状态机 transition 的 JSON 序列化。
fn ask_transition_json(t: &crate::AskTransition) -> Value {
    json!({
        "from": t.from.map(crate::AskState::as_str),
        "to": t.to.as_str(),
        "reason": crate::truncate_for_log(&t.reason),
        "at_ms": t.at_ms,
        "round_no": t.round_no,
    })
}

fn task_metrics_json(metrics: &TaskJournalTaskMetrics) -> Value {
    let by_prompt_value = metrics.by_prompt.as_ref().map(|map| {
        let mut entries: Vec<(&String, &crate::LlmPromptBucket)> = map.iter().collect();
        // 按 count 降序输出，方便人眼一眼看到"哪个 prompt 把额度烧光了"。
        entries.sort_by(|a, b| {
            b.1.count
                .cmp(&a.1.count)
                .then_with(|| b.1.elapsed_ms.cmp(&a.1.elapsed_ms))
                .then_with(|| a.0.cmp(b.0))
        });
        let object: serde_json::Map<String, Value> = entries
            .into_iter()
            .map(|(label, bucket)| {
                (
                    label.clone(),
                    json!({
                        "count": bucket.count,
                        "elapsed_ms": bucket.elapsed_ms,
                    }),
                )
            })
            .collect();
        Value::Object(object)
    });
    json!({
        "used_evidence_ids_count": metrics.used_evidence_ids_count,
        "delivery_consistent": metrics.delivery_consistent,
        "llm_calls_per_task": metrics.llm_calls_per_task,
        "llm_elapsed_ms_per_task": metrics.llm_elapsed_ms_per_task,
        "by_prompt": by_prompt_value,
    })
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournalTaskMetrics {
    pub(crate) used_evidence_ids_count: Option<usize>,
    pub(crate) delivery_consistent: Option<bool>,
    pub(crate) llm_calls_per_task: Option<u64>,
    /// Phase 1.3/1.4: 本次任务期间累计的 LLM 调用耗时（ms），与
    /// `llm_calls_per_task` 一起暴露，方便快速识别"某条任务把
    /// 预算耗在了 LLM 上 vs. 耗在了 skill/runner 上"。
    pub(crate) llm_elapsed_ms_per_task: Option<u64>,
    /// Phase 1.5: per-task 按 prompt label 分桶的 (count, elapsed_ms)。
    /// 取自 [`crate::AppState::task_llm_by_prompt`]。
    /// 用于在 `task_journal_summary.task_metrics.by_prompt` 暴露细分维度。
    pub(crate) by_prompt: Option<std::collections::HashMap<String, crate::LlmPromptBucket>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskJournal {
    pub(crate) task_id: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) input_text: String,
    pub(crate) context_bundle_summary: Option<String>,
    pub(crate) route_result: Option<crate::RouteResult>,
    pub(crate) plan_result: Option<crate::PlanResult>,
    pub(crate) verify_result: Option<TaskJournalVerifySummary>,
    pub(crate) rounds: Vec<TaskJournalRoundTrace>,
    pub(crate) step_results: Vec<TaskJournalStepTrace>,
    pub(crate) finalizer_summary: Option<TaskJournalFinalizerSummary>,
    pub(crate) task_metrics: TaskJournalTaskMetrics,
    pub(crate) final_answer: Option<String>,
    pub(crate) final_status: Option<TaskJournalFinalStatus>,
    /// §3.1: ask 状态机 transition 序列。由 `log_ask_transition` 在每次状态切换时
    /// 追加。Stage A 仅占位，Stage B 起 logger 接入，Stage D 进 journal JSON 输出。
    pub(crate) transitions: Vec<crate::AskTransition>,
}

fn summarize_verify_result(
    verify_result: &crate::verifier::VerifyResult,
) -> TaskJournalVerifySummary {
    TaskJournalVerifySummary {
        mode: verify_result.mode,
        approved: verify_result.approved,
        blocked_reason: verify_result.blocked_reason.clone(),
        shadow_blocked_reason: verify_result.shadow_blocked_reason.clone(),
        needs_confirmation: verify_result.needs_confirmation,
        issues: verify_result
            .issues
            .iter()
            .map(|issue| TaskJournalVerifyIssue {
                step_id: issue.step_id.clone(),
                kind: issue.kind,
                detail: issue.detail.clone(),
            })
            .collect(),
    }
}

#[allow(dead_code)]
impl TaskJournal {
    pub(crate) fn new(input_text: impl Into<String>) -> Self {
        Self {
            input_text: input_text.into(),
            ..Self::default()
        }
    }

    pub(crate) fn for_task(
        task_id: impl Into<String>,
        kind: impl Into<String>,
        input_text: impl Into<String>,
    ) -> Self {
        let mut journal = Self::new(input_text);
        journal.task_id = Some(task_id.into());
        journal.kind = Some(kind.into());
        journal
    }

    pub(crate) fn set_task_identity(
        &mut self,
        task_id: impl Into<String>,
        kind: impl Into<String>,
    ) {
        self.task_id = Some(task_id.into());
        self.kind = Some(kind.into());
    }

    pub(crate) fn record_context_bundle_summary(&mut self, summary: impl Into<String>) {
        self.context_bundle_summary = Some(summary.into());
    }

    pub(crate) fn record_route_result(&mut self, route_result: &crate::RouteResult) {
        self.route_result = Some(route_result.clone());
    }

    pub(crate) fn record_plan_result(&mut self, plan_result: &crate::PlanResult) {
        self.plan_result = Some(plan_result.clone());
    }

    pub(crate) fn record_verify_result(&mut self, verify_result: &crate::verifier::VerifyResult) {
        self.verify_result = Some(summarize_verify_result(verify_result));
    }

    pub(crate) fn push_round_trace(
        &mut self,
        round_no: usize,
        goal: impl Into<String>,
        execution_recipe_summary: Option<String>,
        plan_result: &crate::PlanResult,
        verify_result: &crate::verifier::VerifyResult,
    ) {
        self.plan_result = Some(plan_result.clone());
        self.verify_result = Some(summarize_verify_result(verify_result));
        self.rounds.push(TaskJournalRoundTrace {
            round_no,
            goal: goal.into(),
            execution_recipe_summary,
            plan_result: Some(plan_result.clone()),
            verify_result: Some(summarize_verify_result(verify_result)),
        });
    }

    pub(crate) fn push_step_result(&mut self, step_result: &crate::executor::StepExecutionResult) {
        self.step_results.push(TaskJournalStepTrace {
            step_id: step_result.step_id.clone(),
            skill: step_result.skill.clone(),
            status: step_result.status,
            output_excerpt: step_result.output.as_deref().map(crate::truncate_for_log),
            error_excerpt: step_result.error.as_deref().map(crate::truncate_for_log),
            started_at: step_result.started_at,
            finished_at: step_result.finished_at,
        });
    }

    pub(crate) fn record_finalizer_summary(
        &mut self,
        finalizer_summary: TaskJournalFinalizerSummary,
    ) {
        self.task_metrics.used_evidence_ids_count = Some(finalizer_summary.used_evidence_ids_count);
        self.finalizer_summary = Some(finalizer_summary);
    }

    pub(crate) fn record_used_evidence_ids_count(&mut self, used_evidence_ids_count: usize) {
        self.task_metrics.used_evidence_ids_count = Some(used_evidence_ids_count);
    }

    pub(crate) fn record_delivery_consistent(&mut self, delivery_consistent: bool) {
        self.task_metrics.delivery_consistent = Some(delivery_consistent);
    }

    pub(crate) fn record_llm_calls_per_task(&mut self, llm_calls_per_task: u64) {
        self.task_metrics.llm_calls_per_task = Some(llm_calls_per_task);
    }

    pub(crate) fn record_llm_elapsed_ms_per_task(&mut self, llm_elapsed_ms_per_task: u64) {
        self.task_metrics.llm_elapsed_ms_per_task = Some(llm_elapsed_ms_per_task);
    }

    /// Phase 1.5: 写入 per-task LLM 调用的 by-prompt 分桶。
    /// 来源是 [`crate::AppState::task_llm_by_prompt`] 在收口时取的快照。
    /// 空 map 也接受（表示这次没产生任何 LLM 调用）。
    pub(crate) fn record_llm_by_prompt(
        &mut self,
        by_prompt: std::collections::HashMap<String, crate::LlmPromptBucket>,
    ) {
        self.task_metrics.by_prompt = Some(by_prompt);
    }

    pub(crate) fn record_final_answer(&mut self, final_answer: impl Into<String>) {
        self.final_answer = Some(final_answer.into());
    }

    pub(crate) fn record_final_status(&mut self, final_status: TaskJournalFinalStatus) {
        self.final_status = Some(final_status);
    }

    pub(crate) fn merge_from(&mut self, other: &TaskJournal) {
        if self.task_id.is_none() {
            self.task_id = other.task_id.clone();
        }
        if self.kind.is_none() {
            self.kind = other.kind.clone();
        }
        if self.input_text.trim().is_empty() {
            self.input_text = other.input_text.clone();
        }
        if self.context_bundle_summary.is_none() {
            self.context_bundle_summary = other.context_bundle_summary.clone();
        }
        if self.route_result.is_none() {
            self.route_result = other.route_result.clone();
        }
        if self.plan_result.is_none() {
            self.plan_result = other.plan_result.clone();
        }
        if self.verify_result.is_none() {
            self.verify_result = other.verify_result.clone();
        }
        if self.rounds.is_empty() {
            self.rounds = other.rounds.clone();
        }
        if self.step_results.is_empty() {
            self.step_results = other.step_results.clone();
        }
        if self.finalizer_summary.is_none() {
            self.finalizer_summary = other.finalizer_summary.clone();
        }
        if self.task_metrics.used_evidence_ids_count.is_none() {
            self.task_metrics.used_evidence_ids_count = other.task_metrics.used_evidence_ids_count;
        }
        if self.task_metrics.delivery_consistent.is_none() {
            self.task_metrics.delivery_consistent = other.task_metrics.delivery_consistent;
        }
        if self.task_metrics.llm_calls_per_task.is_none() {
            self.task_metrics.llm_calls_per_task = other.task_metrics.llm_calls_per_task;
        }
        if self.task_metrics.llm_elapsed_ms_per_task.is_none() {
            self.task_metrics.llm_elapsed_ms_per_task = other.task_metrics.llm_elapsed_ms_per_task;
        }
        if self.task_metrics.by_prompt.is_none() {
            self.task_metrics.by_prompt = other.task_metrics.by_prompt.clone();
        }
        if self.final_answer.is_none() {
            self.final_answer = other.final_answer.clone();
        }
        if self.final_status.is_none() {
            self.final_status = other.final_status.clone();
        }
    }

    pub(crate) fn attach_to_result(&self, mut result: Value) -> Value {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "task_journal".to_string(),
                json!({
                    "summary": self.to_summary_json(),
                    "trace": self.to_trace_json(),
                }),
            );
            result
        } else {
            json!({
                "result": result,
                "task_journal": {
                    "summary": self.to_summary_json(),
                    "trace": self.to_trace_json(),
                }
            })
        }
    }

    pub(crate) fn to_summary_json(&self) -> Value {
        json!({
            "task_id": self.task_id.as_deref(),
            "kind": self.kind.as_deref(),
            "round_count": self.rounds.len(),
            "step_count": self.step_results.len(),
            "final_status": self.final_status.map(TaskJournalFinalStatus::as_str),
            "input_text": crate::truncate_for_log(&self.input_text),
            "context_bundle_summary": self.context_bundle_summary.as_deref().map(crate::truncate_for_log),
            "route_result": self.route_result.as_ref().map(route_result_json),
            "latest_execution_recipe_summary": self
                .rounds
                .last()
                .and_then(|round| round.execution_recipe_summary.as_deref())
                .map(crate::truncate_for_log),
            "plan_result": self.plan_result.as_ref().map(plan_summary_json),
            "verify_result": self.verify_result.as_ref().map(verify_summary_json),
            "finalizer_summary": self.finalizer_summary.as_ref().map(finalizer_summary_json),
            "task_metrics": task_metrics_json(&self.task_metrics),
            "final_answer": self.final_answer.as_deref().map(crate::truncate_for_log),
        })
    }

    pub(crate) fn to_trace_json(&self) -> Value {
        json!({
            "rounds": self.rounds.iter().map(|round| {
                json!({
                    "round_no": round.round_no,
                    "goal": crate::truncate_for_log(&round.goal),
                    "execution_recipe_summary": round
                        .execution_recipe_summary
                        .as_deref()
                        .map(crate::truncate_for_log),
                    "plan_result": round.plan_result.as_ref().map(plan_trace_json),
                    "verify_result": round.verify_result.as_ref().map(verify_trace_json),
                })
            }).collect::<Vec<_>>(),
            "step_results": self.step_results.iter().map(step_trace_json).collect::<Vec<_>>(),
            "finalizer_summary": self.finalizer_summary.as_ref().map(finalizer_summary_json),
            "task_metrics": task_metrics_json(&self.task_metrics),
            "ask_state_transitions": self.transitions.iter().map(ask_transition_json).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn to_log_json(&self) -> Value {
        json!({
            "summary": self.to_summary_json(),
            "trace": self.to_trace_json(),
        })
    }
}

pub(crate) fn delivery_payload_consistent(text: &str, messages: &[String]) -> bool {
    let text = text.trim();
    let last_message = messages.iter().rev().find_map(|message| {
        let trimmed = message.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    match last_message {
        Some(message) => message == text,
        None => messages.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{
        delivery_payload_consistent, TaskJournal, TaskJournalFinalizerFallback,
        TaskJournalFinalizerStage, TaskJournalFinalizerSummary,
    };

    #[test]
    fn summary_json_includes_finalizer_and_task_metrics() {
        let mut journal = TaskJournal::for_task("task-1", "ask", "总结 README");
        journal.record_route_result(&crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "不要用现有技能，先规划一个新能力".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "structured self_extension contract".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                self_extension: crate::SelfExtensionContract {
                    mode: crate::SelfExtensionMode::PermanentExtension,
                    trigger: crate::SelfExtensionTrigger::ExplicitUserRequest,
                    execute_now: true,
                },
                ..Default::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        });
        journal.record_finalizer_summary(TaskJournalFinalizerSummary {
            stage: Some(TaskJournalFinalizerStage::General),
            disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
            fallback: Some(TaskJournalFinalizerFallback::RawText),
            parsed: false,
            contract_ok: false,
            completion_ok: None,
            grounded_ok: None,
            format_ok: None,
            needs_clarify: None,
            confidence: None,
            used_evidence_ids_count: 2,
            evidence_quotes_count: 0,
        });
        journal.record_delivery_consistent(true);
        journal.record_llm_calls_per_task(3);
        let summary = journal.to_summary_json();

        assert_eq!(
            summary.get("task_id").and_then(Value::as_str),
            Some("task-1")
        );
        assert_eq!(
            summary
                .get("finalizer_summary")
                .and_then(|v| v.get("stage"))
                .and_then(Value::as_str),
            Some("general")
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("used_evidence_ids_count"))
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("delivery_consistent"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            summary
                .get("task_metrics")
                .and_then(|v| v.get("llm_calls_per_task"))
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            summary
                .get("route_result")
                .and_then(|v| v.get("self_extension"))
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str),
            Some("permanent_extension")
        );
        assert_eq!(
            summary
                .get("route_result")
                .and_then(|v| v.get("self_extension"))
                .and_then(|v| v.get("trigger"))
                .and_then(Value::as_str),
            Some("explicit_user_request")
        );
        assert_eq!(
            summary
                .get("route_result")
                .and_then(|v| v.get("self_extension"))
                .and_then(|v| v.get("execute_now"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn delivery_payload_consistency_uses_last_non_empty_message() {
        assert!(delivery_payload_consistent(
            "最终结果",
            &["".to_string(), "最终结果".to_string()]
        ));
        assert!(!delivery_payload_consistent(
            "最终结果",
            &["中间消息".to_string(), "别的结果".to_string()]
        ));
        assert!(delivery_payload_consistent("任意文本", &[]));
    }

    #[test]
    fn trace_json_includes_execution_recipe_summary() {
        let mut journal = TaskJournal::for_task("task-2", "ask", "修复并验证");
        journal.rounds.push(super::TaskJournalRoundTrace {
            round_no: 1,
            goal: "repair service".to_string(),
            execution_recipe_summary: Some(
                "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false".to_string(),
            ),
            ..Default::default()
        });

        let summary = journal.to_summary_json();
        let trace = journal.to_trace_json();

        assert_eq!(
            summary
                .get("latest_execution_recipe_summary")
                .and_then(Value::as_str),
            Some(
                "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false"
            )
        );
        assert_eq!(
            trace.get("rounds")
                .and_then(Value::as_array)
                .and_then(|rounds| rounds.first())
                .and_then(|round| round.get("execution_recipe_summary"))
                .and_then(Value::as_str),
            Some(
                "kind=ops_closed_loop profile=code_change target_scope=external_workspace phase=validate inspect_first=true validation_required=true repair_count=1 max_repairs=3 saw_inspect=true saw_mutation=true saw_validation=false saw_external_target=true saw_greenfield_creation=false"
            )
        );
    }
}
