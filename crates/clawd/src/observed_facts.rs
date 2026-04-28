use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{AppState, ClaimedTask};

const OBSERVED_FACTS_TTL_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct ObservedFacts {
    pub(crate) bound_target: Option<String>,
    pub(crate) ordered_entries: Vec<String>,
    pub(crate) selected_entry_index: Option<usize>,
    pub(crate) requested_count_limit: Option<usize>,
    pub(crate) observed_entry_count: Option<usize>,
    pub(crate) slice_spec: Option<crate::followup_frame::FollowupSliceSpec>,
    pub(crate) output_shape: Option<String>,
    pub(crate) delivery_targets: Vec<String>,
}

impl ObservedFacts {
    pub(crate) fn is_empty(&self) -> bool {
        self.bound_target.is_none()
            && self.ordered_entries.is_empty()
            && self.selected_entry_index.is_none()
            && self.requested_count_limit.is_none()
            && self.observed_entry_count.is_none()
            && self.slice_spec.is_none()
            && self.delivery_targets.is_empty()
    }
}

fn effective_user_key(task: &ClaimedTask) -> String {
    task.user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id))
}

#[cfg(test)]
#[allow(dead_code)]
fn persist_observed_facts(
    state: &AppState,
    task: &ClaimedTask,
    observed_facts: &ObservedFacts,
) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for observed facts persist: {err}"))?;
    let user_key = effective_user_key(task);
    let facts_json = serde_json::to_string(observed_facts)?;
    let now_ts = crate::now_ts_u64();
    let expires_at_ts = now_ts + OBSERVED_FACTS_TTL_SECS;
    db.execute(
        "INSERT INTO observed_facts_states (
            user_id, chat_id, user_key, facts_json, source_task_id, updated_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            facts_json = excluded.facts_json,
            source_task_id = excluded.source_task_id,
            updated_at_ts = excluded.updated_at_ts,
            expires_at_ts = excluded.expires_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            facts_json,
            task.task_id,
            now_ts as i64,
            expires_at_ts as i64,
        ],
    )?;
    Ok(())
}

fn persist_observed_facts_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    observed_facts: &ObservedFacts,
) -> Result<()> {
    let user_key = effective_user_key(task);
    let facts_json = serde_json::to_string(observed_facts)?;
    let now_ts = crate::now_ts_u64();
    let expires_at_ts = now_ts + OBSERVED_FACTS_TTL_SECS;
    tx.execute(
        "INSERT INTO observed_facts_states (
            user_id, chat_id, user_key, facts_json, source_task_id, updated_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            facts_json = excluded.facts_json,
            source_task_id = excluded.source_task_id,
            updated_at_ts = excluded.updated_at_ts,
            expires_at_ts = excluded.expires_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            facts_json,
            task.task_id,
            now_ts as i64,
            expires_at_ts as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn clear_active_observed_facts(state: &AppState, task: &ClaimedTask) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for observed facts clear: {err}"))?;
    let user_key = effective_user_key(task);
    db.execute(
        "DELETE FROM observed_facts_states
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

fn clear_active_observed_facts_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
) -> Result<()> {
    let user_key = effective_user_key(task);
    tx.execute(
        "DELETE FROM observed_facts_states
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        params![task.user_id, task.chat_id, user_key],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn load_active_observed_facts(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<ObservedFacts> {
    load_active_observed_facts_snapshot(state, task).map(|(facts, _)| facts)
}

#[allow(dead_code)]
pub(crate) fn load_active_observed_facts_snapshot(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<(ObservedFacts, String)> {
    let db = state.core.db.get().ok()?;
    let user_key = effective_user_key(task);
    let mut stmt = db
        .prepare(
            "SELECT facts_json, source_task_id, expires_at_ts
             FROM observed_facts_states
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        )
        .ok()?;
    let (facts_json, source_task_id, expires_at_ts) = stmt
        .query_row(params![task.user_id, task.chat_id, user_key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .ok()?;
    if expires_at_ts <= crate::now_ts_u64() as i64 {
        let _ = clear_active_observed_facts(state, task);
        return None;
    }
    serde_json::from_str::<ObservedFacts>(&facts_json)
        .ok()
        .filter(|facts| !facts.is_empty())
        .map(|facts| (facts, source_task_id))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn replace_active_observed_facts_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let observed_facts =
        derive_observed_facts_from_ask_outcome(answer_text, answer_messages, journal, route_result);
    let result = if observed_facts.is_empty() {
        clear_active_observed_facts(state, task)
    } else {
        persist_observed_facts(state, task, &observed_facts)
    };
    if let Err(err) = result {
        tracing::warn!(
            "observed_facts persist failed task_id={} err={}",
            task.task_id,
            err
        );
        return None;
    }
    (!observed_facts.is_empty()).then(|| task.task_id.clone())
}

pub(crate) fn sync_active_observed_facts_from_ask_outcome_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
) -> Result<Option<String>> {
    let request_surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let observed_facts = derive_observed_facts_from_ask_outcome_with_surface(
        answer_text,
        answer_messages,
        journal,
        route_result,
        &request_surface,
    );
    if observed_facts.is_empty() {
        clear_active_observed_facts_tx(tx, task)?;
        return Ok(None);
    }
    persist_observed_facts_tx(tx, task, &observed_facts)?;
    Ok(Some(task.task_id.clone()))
}

#[cfg(test)]
pub(crate) fn derive_observed_facts_from_ask_outcome(
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
    route_result: &crate::RouteResult,
) -> ObservedFacts {
    let request_surface =
        crate::intent::surface_signals::analyze_prompt_surface(&route_result.resolved_intent);
    derive_observed_facts_from_ask_outcome_with_surface(
        answer_text,
        answer_messages,
        journal,
        route_result,
        &request_surface,
    )
}

pub(crate) fn derive_observed_facts_from_ask_outcome_with_surface(
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
    route_result: &crate::RouteResult,
    request_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
) -> ObservedFacts {
    let mut combined = answer_text.trim().to_string();
    let publishable_messages = answer_messages
        .iter()
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>();
    if !publishable_messages.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(
            &publishable_messages
                .iter()
                .map(|message| message.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    let mut ordered_entries = crate::followup_frame::extract_ordered_entries_from_text(&combined);
    if ordered_entries.is_empty() {
        ordered_entries = crate::followup_frame::derive_ordered_entries_from_journal(journal);
    }
    ordered_entries.truncate(crate::followup_frame::MAX_ORDERED_ENTRIES);

    let mut delivery_targets = crate::extract_delivery_file_tokens(answer_text)
        .into_iter()
        .filter_map(|token| crate::delivery_utils::extract_file_path_from_delivery_token(&token))
        .collect::<Vec<_>>();
    for message in publishable_messages {
        delivery_targets.extend(
            crate::extract_delivery_file_tokens(message)
                .into_iter()
                .filter_map(|token| {
                    crate::delivery_utils::extract_file_path_from_delivery_token(&token)
                }),
        );
    }
    delivery_targets.sort();
    delivery_targets.dedup();

    let bound_target = crate::followup_frame::derive_bound_target_from_journal(journal)
        .or_else(|| {
            crate::followup_frame::derive_bound_target_from_answer(answer_text, answer_messages)
        })
        .or_else(|| {
            let hint = route_result.output_contract.locator_hint.trim();
            (!hint.is_empty()).then(|| hint.to_string())
        });
    let selected_entry_index = bound_target.as_deref().and_then(|target| {
        crate::followup_frame::selected_entry_index_for_target(
            &crate::followup_frame::FollowupFrame {
                bound_target: bound_target.clone(),
                ordered_entries: ordered_entries.clone(),
                ..crate::followup_frame::FollowupFrame::default()
            },
            target,
        )
    });

    let requested_count_limit = request_surface.requested_listing_limit;
    let observed_entry_count = (!ordered_entries.is_empty()).then_some(ordered_entries.len());

    ObservedFacts {
        bound_target,
        ordered_entries,
        selected_entry_index,
        requested_count_limit,
        observed_entry_count,
        slice_spec: crate::followup_frame::derive_slice_spec_from_journal(journal).or_else(|| {
            request_surface
                .requested_read_range
                .map(crate::followup_frame::followup_slice_spec_from_requested_range_for_tests)
        }),
        output_shape: (!matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free
        ))
        .then(|| {
            route_result
                .output_contract
                .response_shape
                .as_str()
                .to_string()
        }),
        delivery_targets,
    }
}

#[cfg(test)]
mod tests {
    use super::{derive_observed_facts_from_ask_outcome, ObservedFacts};

    fn dummy_route_result() -> crate::RouteResult {
        crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn derives_ordered_entries_from_numbered_answer_text() {
        let journal = crate::task_journal::TaskJournal::new("list");
        let facts = derive_observed_facts_from_ask_outcome(
            "1. README.md\n2. Cargo.toml\n3. configs",
            &[],
            &journal,
            &dummy_route_result(),
        );
        assert_eq!(
            facts.ordered_entries,
            vec![
                "README.md".to_string(),
                "Cargo.toml".to_string(),
                "configs".to_string()
            ]
        );
        assert_eq!(facts.selected_entry_index, None);
    }

    #[test]
    fn derives_delivery_targets_from_file_tokens() {
        let journal = crate::task_journal::TaskJournal::new("send");
        let facts = derive_observed_facts_from_ask_outcome(
            "FILE:/tmp/a.log",
            &["FILE:/tmp/b.log".to_string()],
            &journal,
            &dummy_route_result(),
        );
        assert_eq!(
            facts.delivery_targets,
            vec!["/tmp/a.log".to_string(), "/tmp/b.log".to_string()]
        );
    }

    #[test]
    fn ignores_execution_summary_messages_for_observed_facts() {
        let journal = crate::task_journal::TaskJournal::new("send");
        let facts = derive_observed_facts_from_ask_outcome(
            "1. real.log\nFILE:/tmp/real.log",
            &[
                "**执行过程**\n1. wrong.log\nFILE:/tmp/wrong.log".to_string(),
                "2. final.log".to_string(),
            ],
            &journal,
            &dummy_route_result(),
        );

        assert_eq!(facts.delivery_targets, vec!["/tmp/real.log".to_string()]);
        assert!(facts.ordered_entries.contains(&"real.log".to_string()));
        assert!(facts.ordered_entries.contains(&"final.log".to_string()));
        assert!(!facts.ordered_entries.contains(&"wrong.log".to_string()));
        assert!(!facts
            .delivery_targets
            .contains(&"/tmp/wrong.log".to_string()));
    }

    #[test]
    fn derives_selected_entry_index_from_bound_target_and_ordered_entries() {
        let mut journal = crate::task_journal::TaskJournal::new("read");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "s1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "read_range",
                        "resolved_path": "logs/clawd.log",
                        "mode": "tail",
                        "n": 2,
                        "excerpt": "1|a\n2|b"
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let facts = derive_observed_facts_from_ask_outcome(
            "1. act_plan.log\n2. clawd.log\n3. clawd.run.log",
            &[],
            &journal,
            &dummy_route_result(),
        );
        assert_eq!(facts.selected_entry_index, Some(1));
    }

    #[test]
    fn derives_slice_spec_from_requested_n_when_range_output_omits_n() {
        let mut journal = crate::task_journal::TaskJournal::new("read");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "s1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    serde_json::json!({
                        "action": "read_range",
                        "resolved_path": "logs/model_io.log",
                        "mode": "tail",
                        "requested_n": 5,
                        "excerpt": "6|a\n7|b"
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let facts =
            derive_observed_facts_from_ask_outcome("", &[], &journal, &dummy_route_result());
        assert_eq!(
            facts.slice_spec,
            Some(crate::followup_frame::FollowupSliceSpec {
                kind: crate::followup_frame::FollowupSliceKind::Tail,
                n: Some(5),
                start_line: None,
                end_line: None,
            })
        );
    }

    #[test]
    fn derives_slice_spec_from_resolved_intent_when_journal_has_no_range_step() {
        let journal = crate::task_journal::TaskJournal::new("clarify_rewrite");
        let mut route = dummy_route_result();
        route.resolved_intent =
            "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target/content: /tmp/model_io.log"
                .to_string();
        route.output_contract.requires_content_evidence = true;
        route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/model_io.log".to_string();

        let facts = derive_observed_facts_from_ask_outcome(
            "line1\nline2\nline3\nline4\nline5",
            &[],
            &journal,
            &route,
        );

        assert_eq!(
            facts.slice_spec,
            Some(crate::followup_frame::FollowupSliceSpec {
                kind: crate::followup_frame::FollowupSliceKind::Tail,
                n: Some(5),
                start_line: None,
                end_line: None,
            })
        );
        assert_eq!(facts.bound_target.as_deref(), Some("/tmp/model_io.log"));
    }

    #[test]
    fn uses_route_locator_hint_and_listing_limit_when_journal_lacks_scope() {
        let journal = crate::task_journal::TaskJournal::new("list");
        let mut route = dummy_route_result();
        route.resolved_intent = "先列出 logs 目录下前 5 个文件名".to_string();
        route.output_contract.locator_hint = "logs".to_string();
        let facts = derive_observed_facts_from_ask_outcome(
            "1. act_plan.log\n2. clawd.log\n3. clawd.run.log",
            &[],
            &journal,
            &route,
        );
        assert_eq!(facts.bound_target.as_deref(), Some("logs"));
        assert_eq!(facts.requested_count_limit, Some(5));
        assert_eq!(facts.observed_entry_count, Some(3));
    }

    #[test]
    fn derives_output_shape_hint_from_route_contract() {
        let journal = crate::task_journal::TaskJournal::new("send");
        let mut route = dummy_route_result();
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        let facts =
            derive_observed_facts_from_ask_outcome("FILE:/tmp/a.log", &[], &journal, &route);
        assert_eq!(facts.output_shape.as_deref(), Some("file_token"));
    }

    #[test]
    fn empty_observed_facts_reports_empty() {
        assert!(ObservedFacts::default().is_empty());
        assert!(!ObservedFacts {
            bound_target: Some("README.md".to_string()),
            ..ObservedFacts::default()
        }
        .is_empty());
    }

    #[test]
    fn separates_requested_count_limit_from_observed_entry_count() {
        let journal = crate::task_journal::TaskJournal::new("list");
        let mut route = dummy_route_result();
        route.resolved_intent = "先列出 logs 目录下前 5 个文件名".to_string();
        let facts = derive_observed_facts_from_ask_outcome(
            "1. act_plan.log\n2. clawd.log\n3. clawd.run.log",
            &[],
            &journal,
            &route,
        );
        assert_eq!(facts.requested_count_limit, Some(5));
        assert_eq!(facts.observed_entry_count, Some(3));
    }
}
