# clawd `main.rs` Split Plan

## Scope

Target file:

- `crates/clawd/src/main.rs`

Current problem:

- The file mixes startup wiring, runtime state, worker scheduling, skill dispatch, provider adapters, database operations, auth, and HTTP-facing task flows.
- The file size is already large enough that further feature work will increase regression risk and slow down debugging.

Goal:

- Reduce `main.rs` to a true entrypoint and composition layer.
- Move cohesive logic into modules with clear runtime boundaries.
- Avoid behavior changes during the first split pass.

Non-goal:

- No architecture rewrite in the first pass.
- No behavior refactor mixed into the split unless required to preserve compilation.

## Target End State

`crates/clawd/src/main.rs` should eventually keep only:

- `mod` declarations
- top-level constants
- `main()`
- app/router assembly glue
- minimal cross-module helper code

Target size:

- Preferably under 1500 lines after the first complete split round

## Recommended Split Axes

Do not split by file length. Split by runtime responsibility.

### 1. `runtime/`

Move runtime state and shared types out of `main.rs`.

Suggested files:

- `crates/clawd/src/runtime/mod.rs`
- `crates/clawd/src/runtime/state.rs`
- `crates/clawd/src/runtime/policy.rs`
- `crates/clawd/src/runtime/types.rs`

Suggested contents:

- `AppState`
- `LlmProviderRuntime`
- `AgentRuntimeConfig`
- `ClaimedTask`
- `RuntimeChannel`
- `WhatsappDeliveryRoute`
- `AskReply`
- `RateLimiter`
- `ToolsPolicy`
- `ProviderScopedPolicy`
- `RoutedMode`
- `CommandIntentRuntime`
- `ScheduleRuntime`
- `ScheduledJobDue`

Reason:

- These definitions are foundational and reused across most of the file.
- Pulling them out first reduces noise and makes later splits easier.

### 2. `bootstrap/`

Move startup-time config loading and prompt loading logic out of `main.rs`.

Suggested files:

- `crates/clawd/src/bootstrap/mod.rs`
- `crates/clawd/src/bootstrap/config_loaders.rs`
- `crates/clawd/src/bootstrap/prompts.rs`
- `crates/clawd/src/bootstrap/channels.rs`

Suggested contents:

- `load_command_intent_runtime`
- `load_schedule_runtime`
- `builtin_persona_prompt`
- `load_persona_prompt`
- `load_runtime_prompt_template`
- `normalize_prompt_vendor_name`
- `load_memory_runtime_config`
- `resolve_ui_dist_dir`
- `load_feishu_send_config`
- `load_lark_send_config`

Reason:

- These functions are part of app construction, not task execution.
- They are relatively stable and low risk to move early.

### 3. `worker/`

Move task loop, recovery, cleanup, and scheduling workers into a worker module.

Suggested files:

- `crates/clawd/src/worker/mod.rs`
- `crates/clawd/src/worker/recovery.rs`
- `crates/clawd/src/worker/loop.rs`
- `crates/clawd/src/worker/schedule.rs`

Suggested contents:

- `recover_stale_running_tasks_on_startup`
- `recover_stale_running_tasks_by_no_progress`
- `maybe_recover_stale_running_tasks_runtime`
- `start_task_heartbeat`
- `spawn_worker`
- `spawn_cleanup_worker`
- `spawn_schedule_worker`
- `schedule_once`
- `cleanup_once`
- `worker_once`
- `maybe_notify_schedule_result`

Reason:

- This is already a coherent subsystem.
- It has a clear operational boundary and should not live in the entrypoint file.

### 4. `skills/`

Move skill dispatch and execution into a dedicated subsystem.

Suggested files:

- `crates/clawd/src/skills/mod.rs`
- `crates/clawd/src/skills/dispatch.rs`
- `crates/clawd/src/skills/external.rs`
- `crates/clawd/src/skills/builtin.rs`
- `crates/clawd/src/skills/memory_context.rs`
- `crates/clawd/src/skills/output_dirs.rs`

Suggested contents:

- `run_skill_with_runner`
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
- `build_runner_skill_context`
- `run_skill_with_runner_once`
- `inject_skill_memory_context`
- `skill_memory_anchor`
- `execute_builtin_skill`
- `execute_builtin_skill_for_task`
- output-dir helpers such as `ensure_default_output_dir_for_skill_args`

Reason:

- This is one of the largest and most coupled sections in the file.
- It should become a first-class subsystem, not a set of helper functions.

### 5. `providers/`

Move LLM/provider calling and output sanitation into provider-focused modules.

Suggested files:

- `crates/clawd/src/providers/mod.rs`
- `crates/clawd/src/providers/openai_compat.rs`
- `crates/clawd/src/providers/gemini.rs`
- `crates/clawd/src/providers/claude.rs`
- `crates/clawd/src/providers/usage.rs`
- `crates/clawd/src/providers/output.rs`

Suggested contents:

- `call_provider_with_retry`
- `LlmUsageSnapshot`
- `LlmProviderResponse`
- `ProviderError`
- usage snapshot helpers
- `call_provider`
- `call_openai_compat`
- `call_google_gemini`
- `call_anthropic_claude`
- `strip_think_blocks`
- `strip_markdown_json_fence`
- `sanitize_llm_text_output`
- `maybe_sanitize_llm_text_output`
- model I/O log helpers

Reason:

- Provider integrations change frequently and should be isolated from core task orchestration.

## Recommended Migration Order

Do not attempt a one-shot rewrite.

### Phase 1. Move pure types and pure helpers

Move the lowest-risk parts first:

- runtime structs and enums
- prompt rendering helpers
- small provider output sanitation helpers

Why:

- These have fewer dependencies and are easier to compile after relocation.

### Phase 2. Move bootstrap/config loading

After the shared types are stable, move startup-only logic:

- config loaders
- prompt loaders
- channel sender config loaders
- UI dist resolution

Why:

- This keeps startup wiring together and shrinks `main.rs` without touching task execution.

### Phase 3. Move providers and worker flow

Then extract:

- provider call chain
- worker, cleanup, heartbeat, recovery, and schedule loops

Why:

- These are large subsystems with clearer boundaries once shared runtime types are out.

### Phase 4. Move the skills subsystem

Extract the skill execution path last in the first round.

Why:

- It touches `AppState`, DB, provider selection, memory, and external processes.
- It is the highest-coupling section and should be moved after the surrounding types are stable.

### Phase 5. Reduce `main.rs` to composition

Once the moves are complete:

- keep only startup assembly
- router assembly
- module imports
- minimal glue logic

## Boundaries To Preserve

### Keep `AppState` stable in the first pass

Do not try to redesign `AppState` while splitting files.

Reason:

- A file split is already a large mechanical change.
- Mixing interface redesign into the same pass will multiply risk.

### Do not mix DB refactor into the first split

Functions like:

- task claiming/updating
- memory insert/recall
- schema setup
- auth bootstrap

should remain behaviorally unchanged during the first split.

Reason:

- These are important, but they are a second-pass concern.
- First make ownership clearer, then decide whether to move them into `repo/` or a new persistence layer.

### Keep HTTP handlers out of `main.rs`

The project already has `mod http;`.

Direction should be:

- move more handler logic into `http/`

Not:

- bring handler logic back into `main.rs`

## Practical File Ownership Map

If executed as a first-pass split, the ownership should look like this:

- `main.rs`: startup and composition only
- `runtime/`: shared runtime state and policies
- `bootstrap/`: config and prompt loading
- `worker/`: background loops and recovery
- `skills/`: builtin and external skill execution
- `providers/`: model/provider integrations
- `repo/`: existing and later-expanded persistence operations
- `http/`: request/response handlers and route wiring

## Risk Notes

Main risks during split:

- circular imports around `AppState`
- overusing `pub(crate)` without clear ownership
- mixing mechanical file moves with behavior changes
- moving DB helpers too early
- breaking async call chains while relocating worker or provider code

Mitigation:

- move code in compile-safe phases
- keep function signatures unchanged in the first pass
- prefer mechanical relocation first, cleanup second

## Success Criteria

The split is successful when:

- `crates/clawd/src/main.rs` is reduced to entrypoint/composition logic
- provider logic is isolated from task orchestration
- worker loop code is isolated from startup code
- skill execution is grouped into a dedicated subsystem
- no behavior change is introduced during the first pass
- the crate still builds and existing tests continue to pass

## Suggested Next Step

Before editing code, prepare a function-by-function relocation checklist for:

- `runtime/`
- `bootstrap/`
- `worker/`
- `providers/`
- `skills/`

That checklist should name the exact destination file for each function currently in `main.rs`.
