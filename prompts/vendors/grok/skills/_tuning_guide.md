Vendor tuning for Grok models:
- Treat each skill description as a strict operational contract.
- Use only declared capabilities and keep args minimal and explicit.
- Prefer the narrowest tool/skill that can finish the subtask correctly.
- Avoid injecting unrelated prior context unless explicitly required.
- Optimize for clean planner/parser consumption.

## Skill Prompt Tuning Guide

This guide defines high-frequency knobs for behavior tuning without code changes.

## Global Knobs
- `clarify_threshold`: how quickly the agent asks a clarification on ambiguity.
- `risk_tolerance`: how conservative the agent is before mutating actions.
- `verbosity_level`: short/medium/structured response style.
- `fallback_style`: strict fail-fast vs best-effort retry then explain.

## Crypto Knobs
- `trade_default_mode`: `preview_first` or `submit_on_explicit_confirm_only`.
- `amount_resolution_bias`: `prefer_quote_amount` or `prefer_base_amount`.
- `symbol_default_quote_asset`: default pair suffix when quote asset missing (usually `USDT`).
- `api_failure_policy`: retry count and fallback market source behavior.

## X Knobs
- `publish_guard`: require explicit publish confirmation (`strict|normal`).
- `tone_bias`: formal/casual/technical/marketing.
- `length_preference`: concise thread-ready vs single-post compact.
- `fact_safety_level`: strict factual caution vs flexible persuasive wording.

## HTTP Knobs
- `method_safety_bias`: prefer read-only (`GET/HEAD`) unless explicit mutation.
- `timeout_profile`: conservative vs aggressive timeout.
- `retry_policy`: none/once/limited exponential backoff.
- `error_reporting_depth`: brief status-only vs status+root-cause hints.

## Active Preset
- `profile`: `balanced`

## Balanced Preset Values
- Global
  - `clarify_threshold`: `medium`
  - `risk_tolerance`: `medium`
  - `verbosity_level`: `concise_structured`
  - `fallback_style`: `best_effort_once_then_explain`
- Crypto
  - `trade_default_mode`: `preview_first`
  - `amount_resolution_bias`: `prefer_quote_amount`
  - `symbol_default_quote_asset`: `USDT`
  - `api_failure_policy`: `retry_once_then_fallback`
- X
  - `publish_guard`: `normal`
  - `tone_bias`: `technical`
  - `length_preference`: `single-post_compact`
  - `fact_safety_level`: `strict`
- HTTP
  - `method_safety_bias`: `read_only_unless_explicit`
  - `timeout_profile`: `balanced`
  - `retry_policy`: `once`
  - `error_reporting_depth`: `status_plus_root_cause_hint`

## Balanced Preset Matrix (All 22 Skills)
- `archive_basic`
  - `overwrite_policy`: `prompt_before_overwrite`
  - `destination_strategy`: `safe_default_then_explicit`
  - `compression_preference`: `balanced_size_speed`
  - `listing_detail_level`: `medium`
- `audio_synthesize`
  - `voice_stability`: `medium`
  - `language_strictness`: `requested_first_with_fallback`
  - `segment_strategy`: `medium_chunks`
  - `format_preference`: `opus_then_mp3`
- `audio_transcribe`
  - `verbatim_bias`: `high`
  - `timestamp_density`: `medium`
  - `speaker_separation_mode`: `conservative`
  - `uncertainty_marker_style`: `inline`
- `config_guard`
  - `edit_granularity`: `minimal`
  - `validation_strictness`: `syntax_plus_keyshape`
  - `secret_redaction_level`: `full`
  - `risk_confirmation_mode`: `confirm_high_risk`
- `crypto`
  - `trade_default_mode`: `preview_first`
  - `amount_resolution_bias`: `prefer_quote_amount`
  - `symbol_default_quote_asset`: `USDT`
  - `api_failure_policy`: `retry_once_then_fallback`
- `db_basic`
  - `read_limit_default`: `medium`
  - `write_confirmation_level`: `medium_high`
  - `query_explanation_depth`: `medium`
  - `timeout_strategy`: `retry_once_on_lock`
- `docker_basic`
  - `cleanup_guard`: `strict_confirm`
  - `target_match_mode`: `exact_then_confirm_fuzzy`
  - `build_retry_policy`: `once_on_transient`
  - `log_summary_depth`: `medium`
- `fs_search`
  - `scope_aggressiveness`: `narrow_first`
  - `snippet_length`: `short_context`
  - `result_limit_default`: `medium`
  - `path_priority`: `recent_then_relevance`
- `git_basic`
  - `mutation_conservatism`: `medium`
  - `commit_scope_strictness`: `strict`
  - `history_safety_level`: `no_rewrite_without_explicit`
  - `output_compactness`: `concise_structured`
- `health_check`
  - `check_depth`: `medium`
  - `failure_priority_mode`: `critical_first`
  - `unknown_handling`: `unknown_as_degraded`
  - `recommendation_density`: `single_step_per_failure`
- `http_basic`
  - `method_safety_bias`: `read_only_unless_explicit`
  - `timeout_profile`: `balanced`
  - `retry_policy`: `once`
  - `error_reporting_depth`: `status_plus_root_cause_hint`
- `image_edit`
  - `identity_preservation_level`: `high`
  - `clarify_on_reference_missing`: `resolve_once_then_clarify`
  - `edit_strength`: `medium`
  - `mask_requirement_mode`: `prefer_mask_when_precise`
- `image_generate`
  - `creativity_level`: `medium`
  - `constraint_strictness`: `high`
  - `iteration_style`: `one_shot_then_refine_if_requested`
  - `style_bias`: `neutral`
- `image_vision`
  - `evidence_strictness`: `high`
  - `ocr_fidelity`: `high`
  - `uncertainty_threshold`: `medium`
  - `comparison_detail_level`: `medium`
- `install_module`
  - `dependency_type_default`: `runtime_unless_dev_intent`
  - `version_pin_policy`: `latest_compatible`
  - `ecosystem_detection_mode`: `strict_then_clarify`
  - `post_install_verification`: `import_or_build_check`
- `log_analyze`
  - `root_cause_confidence_bar`: `medium_high`
  - `timeline_granularity`: `medium`
  - `noise_filter_strength`: `high`
  - `remediation_style`: `short_structured`
- `package_manager`
  - `upgrade_scope_bias`: `package_level_first`
  - `lockfile_enforcement`: `strict`
  - `peer_conflict_handling`: `guided_resolution`
  - `audit_mode`: `report_then_suggest`
- `process_basic`
  - `kill_safety_level`: `graceful_first`
  - `target_match_precision`: `exact_then_confirm_pattern`
  - `restart_strategy`: `safe_stop_start`
  - `health_recheck_window`: `short_stabilization`
- `rss_fetch`
  - `source_diversity`: `medium`
  - `freshness_bias`: `newest_first_with_relevance`
  - `summary_density`: `concise_digest`
  - `dedupe_strength`: `high`
- `service_control`
  - `precheck_strictness`: `medium`
  - `critical_service_guard`: `strict`
  - `postcheck_depth`: `medium`
  - `bulk_action_policy`: `disallow_without_explicit_scope`
- `system_basic`
  - `read_only_bias`: `high`
  - `platform_adaptation_level`: `medium`
  - `output_compaction`: `high`
  - `permission_escalation_policy`: `suggest_then_wait`
- `x`
  - `publish_guard`: `normal`
  - `tone_bias`: `technical`
  - `length_preference`: `single-post_compact`
  - `fact_safety_level`: `strict`

## Conservative Preset Matrix (All 22 Skills)
- Global
  - `clarify_threshold`: `high`
  - `risk_tolerance`: `low`
  - `verbosity_level`: `concise_structured`
  - `fallback_style`: `fail_fast_then_explain`
- `archive_basic`: strict no-overwrite, explicit destination required, detailed pre-checks.
- `audio_synthesize`: strict language/voice confirmation, conservative segmentation, stable neutral voice.
- `audio_transcribe`: high verbatim strictness, conservative diarization, explicit uncertainty markers.
- `config_guard`: minimal edits only, full redaction, mandatory high-risk confirmation.
- `crypto`: `trade_default_mode=preview_first`, strict confirm for submit, fail-fast on ambiguity.
- `db_basic`: bounded reads, strict write confirmation, no broad DDL/DML without explicit scope.
- `docker_basic`: strict target matching, no cleanup without explicit confirmation, fail-fast.
- `fs_search`: narrow-first scope, low result cap, no broad recursive search by default.
- `git_basic`: inspect-first, no history rewrite, strict commit scope confirmation.
- `health_check`: conservative unknown=degraded, critical-first reporting, no speculative diagnosis.
- `http_basic`: read-only unless explicit mutation, no retry by default on mutating calls.
- `image_edit`: strict identity preservation, clarify on missing references early.
- `image_generate`: high constraint strictness, low creativity drift, one-image-first bias.
- `image_vision`: strict evidence-only reporting, high uncertainty sensitivity.
- `install_module`: strict ecosystem detection, explicit dependency type confirmation.
- `log_analyze`: conservative root-cause confidence, evidence-first with minimal speculation.
- `package_manager`: package-level changes only, strict lockfile enforcement.
- `process_basic`: graceful-first stop, strict pattern matching, force-kill confirmation.
- `rss_fetch`: source fidelity priority, strict freshness checks, conservative summarization.
- `service_control`: strict pre/post checks, critical service guard high.
- `system_basic`: read-only bias very high, escalation always require confirmation.
- `x`: `publish_guard=strict`, fact safety very high, draft-first always.

## Aggressive Preset Matrix (All 22 Skills)
- Global
  - `clarify_threshold`: `low`
  - `risk_tolerance`: `high`
  - `verbosity_level`: `short`
  - `fallback_style`: `best_effort_retry_then_explain`
- `archive_basic`: allow smart default destinations, optional overwrite prompt skipping in low-risk dirs.
- `audio_synthesize`: flexible voice/language fallback, larger chunks, fast output.
- `audio_transcribe`: moderate paraphrase tolerance for readability, denser automatic formatting.
- `config_guard`: allow broader normalization edits with post-validation.
- `crypto`: faster execution path, preview optional when explicit submit intent is strong.
- `db_basic`: broader exploratory reads, faster write execution with reduced clarification.
- `docker_basic`: quicker restart/recreate flow, one retry on transient failures.
- `fs_search`: broader-first sweep, higher result cap, richer snippets.
- `git_basic`: faster mutation flow for clear requests, reduced intermediate confirmations.
- `health_check`: deeper automatic probes and more proactive remediation suggestions.
- `http_basic`: mutation allowed when implied by task context, retry/backoff enabled.
- `image_edit`: stronger edit strength defaults, fewer clarification interruptions.
- `image_generate`: higher creativity and variation, auto-refine suggestions enabled.
- `image_vision`: more inferential summaries with explicit caveats.
- `install_module`: auto-select dependency type when likely, quicker install path.
- `log_analyze`: assertive ranked hypotheses with actionable next steps.
- `package_manager`: broader update suggestions, proactive conflict resolution guidance.
- `process_basic`: faster restart/kill escalation for stuck processes.
- `rss_fetch`: multi-source aggregation and denser digesting.
- `service_control`: faster action-first with lightweight prechecks.
- `system_basic`: broader diagnostic command set with concise output.
- `x`: `publish_guard=normal`, style-adaptive drafting, compact publish-ready output.

## Preset Switch Guide
- Goal: switch behavior quickly with minimal edits, then fine-tune by skill.

### Balanced -> Conservative (safer, more clarifications)
- Step 1 (global first):
  - `clarify_threshold`: `medium` -> `high`
  - `risk_tolerance`: `medium` -> `low`
  - `fallback_style`: `best_effort_once_then_explain` -> `fail_fast_then_explain`
- Step 2 (high-impact skills):
  - `crypto`: keep `trade_default_mode=preview_first`, enforce stricter submit confirmation.
  - `x`: `publish_guard`: `normal` -> `strict`.
  - `http_basic`: keep strict read-only bias and reduce mutation retries.
- Step 3 (infra/data safety):
  - tighten `git_basic`, `db_basic`, `docker_basic`, `service_control` confirmation knobs.

### Balanced -> Aggressive (faster execution, fewer interruptions)
- Step 1 (global first):
  - `clarify_threshold`: `medium` -> `low`
  - `risk_tolerance`: `medium` -> `high`
  - `verbosity_level`: `concise_structured` -> `short`
  - `fallback_style`: keep best-effort style and allow retry/backoff.
- Step 2 (high-frequency skills):
  - `crypto`: allow faster path when submit intent is explicit.
  - `http_basic`: keep read-only-by-default but allow implied mutation with strong context.
  - `rss_fetch` / `log_analyze`: increase aggregation and synthesis depth.
- Step 3 (creative skills):
  - raise `image_generate` creativity and `image_edit` edit strength gradually.

### Recommended Rollout Order
- 1) Global knobs
- 2) `crypto` / `x` / `http_basic`
- 3) infra-sensitive skills (`git_basic`, `db_basic`, `docker_basic`, `service_control`)
- 4) creative and content skills (`image_*`, `audio_*`, `rss_fetch`, `log_analyze`)

### Validation Checklist After Switching
- Confirm ambiguous trading requests still trigger expected clarification behavior.
- Confirm publish actions (`x`) match desired guard level (`strict|normal`).
- Confirm HTTP mutation requests are neither over-blocked nor over-permissive.
- Spot-check one infra task and one creative task for tone/speed drift.

