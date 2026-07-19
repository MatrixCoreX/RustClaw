use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::super::LoopState;
use super::{
    answer_is_direct_observation_passthrough, archive_list_raw_passthrough_replacement,
    archive_list_summary_from_body, compound_listing_content_delivery_guard_entry,
    cross_turn_observed_output_entries, dir_compare_direct_answer_candidate,
    execution_failed_step_guard_entry, extract_answer_from_finalizer_envelope_text,
    extract_direct_answer_from_generic_output, extract_direct_answer_from_generic_output_i18n,
    extract_direct_scalar_from_generic_output, extract_direct_scalar_from_generic_output_i18n,
    extract_direct_scalar_from_generic_output_with_locator_hint,
    extract_field_direct_answer_candidate, freeform_observed_answer_fallback,
    has_observed_answer_candidates, inventory_dir_direct_answer_candidate,
    multi_count_quantity_comparison_guard_entry, multi_field_machine_record_is_language_neutral,
    non_code_markdown_text, normalize_system_basic_match_path, normalized_observed_listing,
    observed_answer_fallback_prompt_logical_path, observed_answer_language_compatible,
    observed_answer_language_compatible_for_route, observed_contract_json,
    observed_language_supports_bilingual_template, observed_output_entries,
    observed_request_language_hint, observed_request_prefers_english_template,
    observed_response_style_hint, recent_generated_output_from_user_request,
    replace_internal_missing_sentinel_with_structured_observation,
    route_allows_path_batch_scalar_path_observed_answer, route_allows_raw_listing_direct_answer,
    route_disallows_direct_observation_passthrough, route_observation_facts_entry,
    route_prefers_plain_fs_search_paths,
    route_quantity_comparison_requires_model_language_synthesis, route_requests_scalar_path_only,
    route_requires_synthesized_delivery, scalar_count_diagnostic_line_for_answer,
    scalar_count_diagnostic_machine_answer, scalar_route_prefers_structured_observed_answer,
    strip_bare_json_language_prefix, structured_observed_body,
    tree_summary_direct_answer_candidate, try_synthesize_answer_from_observed_output,
    AgentRunContext, ObservedAnswerFallbackOut, OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    AppState, ClaimedTask, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, SkillViewsSnapshot,
};
use claw_core::skill_registry::SkillsRegistry;

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

fn error_step(step_id: &str, skill: &str, error: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(error.to_string()),
        started_at: 0,
        finished_at: 0,
    }
}

fn test_state_with_registry(toml: &str, skills: &[&str]) -> AppState {
    let path = std::env::temp_dir().join(format!(
        "observed_output_registry_{}_{}.toml",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_file(path);
    let mut state = AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(
            skills
                .iter()
                .map(|skill| (*skill).to_string())
                .collect::<HashSet<_>>(),
        ),
    })));
    state
}

fn claimed_task(task_id: &str) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

include!("observed_output_tests/core_scalar_structured_inventory.rs");
include!("observed_output_tests/core_scalar_structured_tail.rs");
include!("observed_output_tests/fs_search_contracts.rs");
include!("observed_output_tests/git_text_boundary.rs");
include!("observed_output_tests/observed_fallback_read_range.rs");
include!("observed_output_tests/service_control_text_boundary.rs");
include!("observed_output_tests/structured_scalar_text_boundary.rs");
include!("observed_output_tests/system_basic_info_text_boundary.rs");
include!("observed_output_tests/success_text_boundary.rs");
include!("observed_output_tests/strict_raw_tail_text_boundary.rs");
include!("observed_output_tests/structured_listing_path.rs");
include!("observed_output_tests/structured_listing_path_tail.rs");
include!("observed_output_tests/system_archive_path_package.rs");
include!("observed_output_tests/sqlite_archive_quantity_git.rs");
include!("observed_output_tests/raw_health_service_http_log.rs");
include!("observed_output_tests/raw_health_service_http_log_tail.rs");
include!("observed_output_tests/run_cmd_command_output.rs");
include!("observed_output_tests/text_parsing.rs");
