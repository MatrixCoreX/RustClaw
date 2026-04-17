# clawd `main.rs` Function Relocation Checklist

## Scope

Source file:

- `crates/clawd/src/main.rs`

Target:

- turn the split plan into a concrete relocation checklist
- keep this as an execution checklist for future refactor work
- do not mix behavioral redesign into the first pass

## Rules For The First Split Pass

- Prefer mechanical moves over logic changes.
- Keep function signatures unchanged unless compilation forces a small adjustment.
- Move types together with the functions that conceptually own them.
- If a function is still shared by many modules, keep it temporarily in `runtime/` or `main.rs` rather than forcing a bad boundary.

## Target Module Layout

- `crates/clawd/src/main.rs`
- `crates/clawd/src/runtime/mod.rs`
- `crates/clawd/src/runtime/state.rs`
- `crates/clawd/src/runtime/policy.rs`
- `crates/clawd/src/runtime/types.rs`
- `crates/clawd/src/bootstrap/mod.rs`
- `crates/clawd/src/bootstrap/config_loaders.rs`
- `crates/clawd/src/bootstrap/prompts.rs`
- `crates/clawd/src/bootstrap/channels.rs`
- `crates/clawd/src/worker/mod.rs`
- `crates/clawd/src/worker/recovery.rs`
- `crates/clawd/src/worker/loop.rs`
- `crates/clawd/src/worker/schedule.rs`
- `crates/clawd/src/skills/mod.rs`
- `crates/clawd/src/skills/dispatch.rs`
- `crates/clawd/src/skills/external.rs`
- `crates/clawd/src/skills/builtin.rs`
- `crates/clawd/src/skills/memory_context.rs`
- `crates/clawd/src/skills/output_dirs.rs`
- `crates/clawd/src/providers/mod.rs`
- `crates/clawd/src/providers/openai_compat.rs`
- `crates/clawd/src/providers/gemini.rs`
- `crates/clawd/src/providers/claude.rs`
- `crates/clawd/src/providers/usage.rs`
- `crates/clawd/src/providers/output.rs`
- `crates/clawd/src/tasks/mod.rs`
- `crates/clawd/src/tasks/repo.rs`
- `crates/clawd/src/tasks/flow.rs`
- `crates/clawd/src/auth/mod.rs`
- `crates/clawd/src/auth/repo.rs`
- `crates/clawd/src/system/mod.rs`
- `crates/clawd/src/system/process_stats.rs`

## Keep In `main.rs`

These should stay in `main.rs` in the first pass:

- `main`
- top-level constants
- top-level `mod` declarations
- router assembly glue
- final startup wiring that composes bootstrap, http, worker, and runtime modules

These may also stay temporarily if moving them early causes circular dependencies:

- `api_err`
- `api_ok`

## Checklist By Target File

### `runtime/state.rs`

Move these types:

- `SkillViews`
- `SkillViewsSnapshot`
- `AppState`
- `LlmProviderRuntime`
- `AgentRuntimeConfig`
- `ClaimedTask`

Move these methods and functions:

- `build_skill_views`
- `reload_skill_views`
- `AppState::snapshot`
- `AppState::get_skills_registry`
- `AppState::get_skills_list`
- `AppState::planner_visible_skills_for_task`
- `AppState::normalize_known_agent_id`
- `AppState::task_agent_id`
- `AppState::task_agent`
- `AppState::task_persona_prompt`
- `AppState::task_allows_skill`
- `AppState::task_llm_providers`
- `AppState::resolve_canonical_skill_name`
- `AppState::is_builtin_skill`
- `AppState::skill_registry_prompt_rel_path`
- `AppState::skill_kind_for_dispatch`
- `AppState::runner_name_for_skill`
- `AgentRuntimeConfig::from_config`
- `AgentRuntimeConfig::allows_skill`

Keep related result type with it:

- `ReloadSkillViewsResult`

### `runtime/types.rs`

Move these types:

- `RuntimeChannel`
- `WhatsappDeliveryRoute`
- `AskReply`
- `AgentAction`
- `RoutedMode`
- `CommandIntentRules`
- `CommandIntentRuntime`
- `ScheduleRuntime`
- `LocalInteractionContext`
- `MemoryConfigFileWrapper`
- `ScheduleIntentOutput`
- `ScheduleIntentSchedule`
- `ScheduleIntentTask`
- `ScheduledJobDue`
- `RunCmdSuggestionPayload`
- `LlmUsageSnapshot`
- `LlmProviderResponse`
- `ProviderError`
- request/response DTOs near task handlers:
  - `CancelTasksRequest`
  - `ActiveTasksRequest`
  - `ActiveTaskItem`
  - `CancelOneTaskRequest`

Move these methods:

- `AskReply::llm`
- `AskReply::non_llm`
- `AskReply::with_messages`

### `runtime/policy.rs`

Move these types:

- `RateLimiter`
- `ToolsPolicy`
- `ProviderScopedPolicy`

Move these functions and methods:

- `RateLimiter::new`
- `RateLimiter::check_and_record`
- `ToolsPolicy::from_config`
- `ToolsPolicy::is_allowed`
- `ToolsPolicy::default_allowed`
- `provider_policy_keys`
- `normalize_capability_pattern`
- `wildcard_match`

These small helpers can stay here or later move to `providers/`:

- `llm_vendor_name`
- `llm_model_kind`

## `bootstrap/prompts.rs`

Move these functions:

- `builtin_persona_prompt`
- `load_persona_prompt`
- `load_runtime_prompt_template`
- `normalize_prompt_vendor_name`
- `render_prompt_template`
- `log_prompt_render`
- `parse_llm_json_extract_then_raw`
- `parse_llm_json_extract_or_any`
- `parse_llm_json_raw_or_any`

Reason:

- These are all prompt/template/rendering concerns.

## `bootstrap/config_loaders.rs`

Move these functions:

- `load_command_intent_runtime`
- `load_schedule_runtime`
- `load_memory_runtime_config`
- `trim_command_text`
- `strip_result_suffixes`
- `sanitize_command_before_execute`

Reason:

- These belong to runtime config loading and normalization.

## `bootstrap/channels.rs`

Move these functions:

- `resolve_ui_dist_dir`
- `load_feishu_send_config`
- `load_lark_send_config`

## `worker/recovery.rs`

Move these functions:

- `recover_stale_running_tasks_on_startup`
- `recover_stale_running_tasks_by_no_progress`
- `maybe_recover_stale_running_tasks_runtime`
- `start_task_heartbeat`

## `worker/loop.rs`

Move these functions:

- `spawn_worker`
- `spawn_cleanup_worker`
- `cleanup_once`
- `worker_once`

## `worker/schedule.rs`

Move these functions:

- `spawn_schedule_worker`
- `schedule_once`
- `maybe_notify_schedule_result`

Move schedule-support helpers if they are only used here:

- `runtime_channel_from_payload`
- `is_whatsapp_channel_value`
- `is_resume_continue_source`
- `maybe_bind_recent_failed_resume_context`
- `parse_task_status_with_rules`
- `task_payload_value`
- `task_runtime_channel`
- `task_external_chat_id`
- `send_task_channel_message`
- `resolve_whatsapp_delivery_route`

If some of these are shared by task submission or HTTP handlers, move them later to `tasks/flow.rs` instead.

## `skills/dispatch.rs`

Move these functions:

- `run_skill_with_runner`
- `build_runner_skill_context`
- `run_skill_with_runner_once`
- `execute_builtin_skill_for_task`
- `execute_ask_routed`
- `analyze_attached_images_for_ask`

Also move shared skill dispatch helpers:

- `selected_openai_api_key_for_task`
- `selected_openai_base_url_for_task`
- `selected_openai_model_for_task`
- `dynamic_chat_memory_budget_chars`
- `estimate_context_window_tokens`
- `extract_model_k_or_m_capacity_tokens`
- `estimate_text_tokens`

## `skills/external.rs`

Move these functions:

- `execute_external_skill`
- `external_reserved_arg_key`
- `value_to_cli_string`
- `build_external_cli_args`
- `resolve_external_bundle_dir`
- `resolve_external_entry_path`
- `is_bin_available`
- `verify_external_python_modules`
- `execute_external_local_script`
- `extract_external_shell_command`
- `execute_external_local_shell_recipe`
- `extract_skill_provider_model`
- `resolve_external_auth`
- `mask_endpoint_for_log`
- `execute_external_http_json`

## `skills/memory_context.rs`

Move these functions:

- `inject_skill_memory_context`
- `skill_memory_anchor`

## `skills/output_dirs.rs`

Move these functions:

- `ensure_default_output_dir_for_skill_args`
- `resolve_output_dir_from_config`
- `resolve_file_default_output_dir_from_config`

## `skills/builtin.rs`

Move these functions:

- `execute_builtin_skill`
- `ensure_args_object`
- `ensure_only_keys`
- `required_string`
- `optional_string`
- `suggested_command_from_args`
- `build_run_cmd_nl_prompt`
- `suggest_command_for_run_cmd`
- `resolve_workspace_path`
- `run_safe_command`

Move delivery and file/image helpers if primarily used by builtin skills:

- `extract_file_path_from_delivery_token`
- `trim_path_token`
- `intercept_response_payload_for_delivery`
- `is_image_file_path`
- `collect_recent_image_candidates`
- `merge_image_candidate_paths_from_args`
- `collect_image_paths_from_task_payload`

If these are shared with other task flows, move them to `tasks/flow.rs` instead.

## `providers/output.rs`

Move these functions:

- `strip_think_blocks`
- `strip_markdown_json_fence`
- `sanitize_llm_text_output`
- `maybe_sanitize_llm_text_output`
- `append_model_io_log`
- `prune_model_io_log_to_today`
- `log_color_enabled`
- `truncate_text`
- `utf8_safe_prefix`

## `providers/usage.rs`

Move these functions:

- `value_as_u64`
- `sum_u64`
- `openai_usage_snapshot`
- `gemini_usage_snapshot`
- `anthropic_usage_snapshot`

## `providers/mod.rs`

Move these functions:

- `call_provider_with_retry`
- `call_provider`

## `providers/openai_compat.rs`

Move:

- `call_openai_compat`

## `providers/gemini.rs`

Move:

- `call_google_gemini`

## `providers/claude.rs`

Move:

- `call_anthropic_claude`

## `tasks/flow.rs`

Move task flow and conversational execution helpers that are not specific to worker internals:

- `parse_resume_context_error`
- `build_resume_continue_execute_prompt`
- `build_resume_followup_discussion_prompt`
- `chat_act_goal_from_prompt`
- `is_agent_action_candidate`
- `repair_invalid_json_escapes`
- `normalize_agent_action_shape`
- `collect_bare_action_args`
- `submit_task`
- `normalized_optional_task_id`
- `summarize_active_task_payload`
- `list_active_tasks_internal`

Move task and channel helpers here if not already placed under worker:

- `runtime_channel_from_payload`
- `task_payload_value`
- `task_runtime_channel`
- `task_external_chat_id`
- `send_task_channel_message`
- `resolve_whatsapp_delivery_route`

## `tasks/repo.rs`

Move database-oriented task operations:

- `claim_next_task`
- `update_task_success`
- `touch_running_task`
- `update_task_progress_result`
- `update_task_failure_with_result`
- `update_task_failure`
- `update_task_timeout`
- `insert_audit_log`
- `insert_audit_log_raw`
- `insert_memory`
- `recall_recent_memories`
- `filter_memories_for_prompt_recall`
- `select_relevant_memories_for_prompt`
- `recall_user_preferences`
- `recall_long_term_summary`
- `recall_memories_since_id`
- `read_long_term_source_memory_id`
- `upsert_long_term_summary`
- `maybe_refresh_long_term_summary`
- `task_count_by_status`
- `oldest_running_task_age_seconds`
- `find_recent_duplicate_affirmation_task`
- `find_recent_failed_resume_context`

## `auth/repo.rs`

Move auth- and identity-related DB helpers:

- `stable_i64_from_key`
- `normalize_exchange_name`
- `exchange_credential_context_for_task`
- `channel_kind_name`
- `build_conversation_chat_id`
- `build_auth_identity`
- `channel_allows_shared_ui_task_access`
- `touch_auth_key_usage`
- `is_user_allowed`
- `channel_allows_public_access`
- `upsert_public_channel_user`

## `system/process_stats.rs`

Move process and host runtime inspection helpers:

- `current_rss_bytes`
- `current_rss_bytes_from_status`
- `telegramd_process_stats`
- `channel_gateway_process_stats`
- `whatsappd_process_stats`
- `wa_webd_process_stats`
- `feishud_process_stats`
- `larkd_process_stats`
- `daemon_process_stats`
- `process_name_matches`

## Keep For A Later Pass

These are valid split candidates, but should wait until the first split settles:

- DB schema setup
- bootstrap admin/auth seeding
- migration helpers
- channel schema rebuild helpers

Specifically:

- `init_db`
- `seed_users`
- `ensure_schedule_schema`
- `ensure_memory_schema`
- `ensure_channel_schema`
- `rebuild_channel_tables_for_ui`
- `ensure_key_auth_schema`
- `rebuild_user_preferences_for_key_scope`
- `rebuild_long_term_memories_for_key_scope`
- `generate_user_key`
- `ensure_bootstrap_admin_key`
- `seed_channel_binding_rows`
- `seed_channel_bindings`
- `ensure_column_exists`

Recommended later destination:

- `repo/`
- or a new `db/` module if the persistence layer grows further

## Execution Order Checklist

### Phase 1

- [ ] Create `runtime/` modules
- [ ] Move runtime structs and enums
- [ ] Move policy helpers
- [ ] Keep `main.rs` compiling with re-exports or adjusted imports

### Phase 2

- [ ] Create `bootstrap/` modules
- [ ] Move prompt and config loading helpers
- [ ] Move UI/channel startup helpers

### Phase 3

- [ ] Create `providers/` modules
- [ ] Move provider call chain
- [ ] Move usage parsing and output sanitation

### Phase 4

- [ ] Create `worker/` modules
- [ ] Move recovery, cleanup, worker loop, and schedule functions

### Phase 5

- [ ] Create `skills/` modules
- [ ] Move external skill execution helpers
- [ ] Move builtin skill execution helpers
- [ ] Move skill memory/output helpers

### Phase 6

- [ ] Create `tasks/` and `auth/` modules
- [ ] Move task-flow helpers
- [ ] Move task repo operations
- [ ] Move auth repo operations

### Phase 7

- [ ] Shrink `main.rs` to startup/composition
- [ ] Run build and regression validation
- [ ] Only then consider cleanup refactors or API redesign

## Success Criteria

- `main.rs` stops being the default home for unrelated features
- runtime state, providers, worker flow, and skills each have a clear home
- database operations are grouped rather than scattered
- the first split pass is mostly mechanical and low-risk
