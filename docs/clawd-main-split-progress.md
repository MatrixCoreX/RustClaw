# clawd `main.rs` Split Progress

## Snapshot

Date:

- `2026-04-02`

Current source file:

- `crates/clawd/src/main.rs`

Current size:

- `1020` lines

Current state:

- The first split round is complete.
- The second closeout round for `main.rs` and `delivery_utils` is also complete.
- `main.rs` is now primarily startup wiring, router assembly, root facade re-exports, shared constants, and a small HTTP/admin shell surface.

## What Has Been Completed

### Runtime

Extracted to:

- `crates/clawd/src/runtime/mod.rs`
- `crates/clawd/src/runtime/state.rs`
- `crates/clawd/src/runtime/policy.rs`
- `crates/clawd/src/runtime/types.rs`

Covered:

- shared runtime state
- runtime types
- rate limit and tools policy structures
- skill registry/runtime-facing state

### Bootstrap

Extracted to:

- `crates/clawd/src/bootstrap/mod.rs`
- `crates/clawd/src/bootstrap/prompts.rs`
- `crates/clawd/src/bootstrap/config_loaders.rs`
- `crates/clawd/src/bootstrap/channels.rs`

Covered:

- prompt loading
- runtime config loading
- channel sender config loading
- UI dist path resolution

### Providers

Extracted to:

- `crates/clawd/src/providers/mod.rs`
- `crates/clawd/src/providers/client.rs`
- `crates/clawd/src/providers/output.rs`
- `crates/clawd/src/providers/usage.rs`

Covered:

- provider call chain
- usage snapshots
- output sanitation
- model I/O logging helpers

### Worker

Extracted to:

- `crates/clawd/src/worker/mod.rs`
- `crates/clawd/src/worker/ask_prepare.rs`
- `crates/clawd/src/worker/ask_finalize.rs`
- `crates/clawd/src/worker/ask_pipeline.rs`
- `crates/clawd/src/worker/channels.rs`
- `crates/clawd/src/worker/locator.rs`
- `crates/clawd/src/worker/runtime_support.rs`
- `crates/clawd/src/worker/run_skill_finalize.rs`

Covered:

- startup stale-running recovery
- runtime stale-task recovery
- heartbeat
- worker loop
- cleanup loop
- schedule loop
- ask task processing
- run-skill task processing
- worker-side channel/runtime helper logic

### Repo

Extracted to:

- `crates/clawd/src/repo/mod.rs`
- `crates/clawd/src/repo/tasks.rs`
- `crates/clawd/src/repo/audit.rs`
- `crates/clawd/src/repo/auth.rs`
- `crates/clawd/src/repo/submit.rs`

Covered:

- task state transitions
- audit writes
- auth key and binding operations
- auth schema work
- submit-task context resolution
- submit-task access/limit checks
- submit-task insert path
- dedup helpers
- task query / active / cancel paths

### Skills

Extracted to:

- `crates/clawd/src/skills.rs`
- `crates/clawd/src/skills/builtin.rs`
- `crates/clawd/src/skills/external.rs`
- `crates/clawd/src/skills/memory_context.rs`
- `crates/clawd/src/skills/output_dirs.rs`

Covered:

- skill dispatch
- builtin skill execution
- external skill execution
- runner execution
- runner context building
- credential context building
- skill memory injection
- skill output dir handling
- provider model selection from skill args
- skill metadata helpers

### Ask / Prompt / LLM Flow

Extracted to:

- `crates/clawd/src/ask_flow.rs`
- `crates/clawd/src/prompt_utils.rs`
- `crates/clawd/src/llm_gateway.rs`

Covered:

- ask routing helpers
- image analysis path for ask
- resume prompt helpers
- prompt rendering
- JSON extraction and repair helpers
- agent action extraction
- LLM fallback execution entry
- selected OpenAI config helpers

### Memory / Shared Helper Closeout

Extracted to:

- `crates/clawd/src/memory/service.rs`
- `crates/clawd/src/app_helpers.rs`

Covered:

- dynamic chat memory budget calculation
- model context window estimation helpers
- long-term summary refresh implementation
- resume-context error parsing
- i18n fallback lookup
- main-flow rule lookup
- task-status parsing
- time/schema helpers
- secret/id/exchange normalization helpers
- affirmation normalization helpers

### Delivery / Utility Closeout

Extracted to:

- `crates/clawd/src/delivery_utils.rs`
- `crates/clawd/src/delivery_utils/locator.rs`
- `crates/clawd/src/delivery_utils/directory_lookup.rs`
- `crates/clawd/src/delivery_utils/file_delivery.rs`
- `crates/clawd/src/delivery_utils/message_media.rs`
- `crates/clawd/src/delivery_utils/output_contract.rs`
- `crates/clawd/src/delivery_utils/path_helpers.rs`
- `crates/clawd/src/delivery_utils/types.rs`
- `crates/clawd/src/delivery_utils/tests.rs`

Covered:

- delivery/file token extraction
- message normalization
- image candidate collection
- output-contract shaping
- shared delivery types
- path helpers
- locator and directory/file resolution
- dedicated delivery test module

## Validation Status

Local validation completed:

- `cargo check -p clawd`
- `cargo test -p clawd --no-run`

Cross-build validation:

- attempted `./cross-build-upload.sh crate clawd`
- blocked by environment error: `Host key verification failed`

Interpretation:

- local build and test-compile validation passed for the full closeout round
- remote cross-build was attempted using the existing project script, but could not complete because the remote SSH trust state is not ready on this machine

## Current `main.rs` Responsibility Profile

What still belongs there today:

- top-level `mod` declarations
- top-level constants and embedded prompt/SQL resources
- startup wiring
- router assembly
- root facade re-exports
- task/admin HTTP handler shells

What is still physically in `main.rs`:

- `api_err`
- `api_ok`
- `main`
- `submit_task`
- `get_task`
- `authorize_task_admin_request`
- `list_active_tasks`
- `cancel_tasks`
- `cancel_one_task`
- `reload_skills_handler`

What has been intentionally removed from `main.rs` in the closeout round:

- startup stale-running recovery implementation
- memory budget helpers
- long-term summary refresh implementation
- resume-context parsing helper
- i18n fallback helper
- time/schema helpers
- main-flow and affirmation helpers
- secret/id/exchange normalization helpers

## Remaining Work

There is no further required mechanical split work for the current round.

Optional later cleanup only:

- reduce root facade re-exports if a future import-path cleanup is desired
- move HTTP handlers deeper into `http/` if the HTTP layer is refactored later
- split `skills.rs` / `providers/client.rs` further only if those subsystems become active hotspots again

## Recommendation

Recommended status:

1. Treat the 5-layer split first round as complete.
2. Treat the second closeout round as complete.
3. Stop mechanical extraction work here and move to feature or behavior work.

Reason:

- the high-risk and high-payoff extraction work is already done
- `main.rs` is no longer the default home for unrelated operational logic
- further splitting now is optional polish rather than architectural risk reduction

## Outcome

Result of the refactor round:

- `main.rs` has been reduced from a mixed execution file to an entry/composition-heavy file
- worker, delivery, memory helper, and shared utility responsibilities now have explicit homes
- the 5-layer split can be considered closed for this round, with only optional future polish remaining
