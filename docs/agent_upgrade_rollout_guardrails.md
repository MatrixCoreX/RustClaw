# Agent Upgrade Rollout Guardrails

Last updated: 2026-06-21

This document is the P0 rollout guardrail for the agent generalization plan. It records what can be safely changed now, what is only planned, how to test each change, and how to roll it back.

## Scope

The current upgrade work touches the agent loop, verifier, finalizer, registry metadata, and natural-language execution paths. These changes must stay observable, reversible, and dry-run safe before any semantic routing responsibility moves from the first-layer route into the agent loop.

Non-negotiable boundaries:

- Runtime code must not add user natural-language phrase matching.
- Runtime, finalizer, verifier, and execution adapters must not add hardcoded user-facing reply templates.
- Deterministic paths may emit machine fields, status codes, paths, counts, `message_key`, or structured evidence.
- User prose should be rendered by finalizer, LLM, or i18n according to the request language.
- X API publish/fetch/delete and other remote side effects require dry run, mock, sandbox, or explicit confirmation.
- Image and audio real calls are outside the current NL canary scope.

## Rollout Controls

### Wired Today

These controls are read by current code and can be used for rollback after config change plus service restart.

| Control | Location | Default / Current | Effect | Rollback |
| --- | --- | --- | --- | --- |
| `max_steps` | `configs/agent_guard.toml` `[agent.loop_guard]` | `32` | Maximum agent loop steps. | Restore previous value and restart `clawd`. |
| `max_rounds` | `[agent.loop_guard]` | `4` | Maximum planner-execution rounds. | Lower to previous stable value and restart. |
| `recoverable_failure_extra_rounds` | `[agent.loop_guard]` | `1` | Extra repair rounds for structured recoverable failures near round limit. | Set to `0` or previous stable value and restart. |
| `multi_round_enabled` | `[agent.loop_guard]` | `true` | Enables controlled multi-round execution. | Set `false` to fall back toward single-round behavior. |
| `answer_verifier_retry_limit` | `[agent.loop_guard]` | `2` | Max automatic answer-verifier repair attempts. | Reduce to `0` or previous value and restart. |
| `repeat_action_limit` | `[agent.loop_guard]` | `4` | Cross-round repeat-action guard. | Restore previous value and restart. |
| `no_progress_limit` | `[agent.loop_guard]` | `1` | Stops consecutive no-progress rounds. | Restore previous value and restart. |
| `max_tool_calls` | `[agent.loop_guard]` | `12` | Maximum tool/skill calls per task. | Restore previous value and restart. |
| `budget_profiles.*` | `[agent.loop_guard.budget_profiles.*]` | profile-specific | Per-task-class loop budgets. | Revert profile values and restart. |
| `ops_closed_loop.*` | `[agent.loop_guard.ops_closed_loop]` | profile-specific | Larger budget for check-modify-validate-repair flows. | Revert profile values and restart. |

### Rollout Controls

These controls are read from `configs/agent_guard.toml` and logged as machine tokens / task-journal attribution where relevant. Treat each control separately: `semantic_route_authority` is the current route-authority lifecycle token, `registry_idempotency_guard_scope` and `answer_verifier_enforce_required_scope` are machine-token rollout scopes, `structured_evidence_required_for_selected_contracts` is default-on for selected agent-loop contracts, and older bool names are ignored historical config keys.

| Control | Current Default | Intended Effect | Required Before Behavior Use |
| --- | --- | --- | --- |
| `answer_verifier_enforce_required_scope` | `all` | Convert high-confidence required-evidence verifier failures into structured block/retry outcomes. | Focused Rust tests and compressed release-gate-equivalent NL passed before defaulting to `all`; keep attribution review active and roll back to `selected_agent_loop` or `off` if false verifier blocks appear. |
| `answer_verifier_enforce_required` | ignored | Historical bool name; current runtime config load does not parse it. | Do not add it to new configs; use `answer_verifier_enforce_required_scope` instead. |
| `semantic_route_authority` | `agent_loop_default` | Move ordinary semantic authority for eligible low-risk buckets into the agent loop; keep `legacy` only as short-term emergency rollback and `shadow` / `agent_loop_canary` only for rollout/debug. | Do not physically delete the remaining legacy route-label control paths until 500 canary, 2100 safe aggregate or equivalent coverage, and route-delta review show no unexplained mismatch. |
| `agent_loop_canary_bucket` | unset / `none` | Narrow `agent_loop_canary` to one bucket for debugging. `agent_loop_default` ignores this and selects any eligible low-risk bucket. | Use only for targeted rollout/debug; do not treat it as the long-term architecture. |
| `registry_idempotency_guard_scope` | `all` | Drive once/dedup/idempotency from registry metadata. | Focused tests and compressed release-gate-equivalent NL passed before defaulting to `all`; keep repeat-block attribution review active and roll back to `selected_agent_loop` or `off` if false repeat blocks appear. |
| `registry_idempotency_guard` | ignored | Historical bool name; current runtime config load does not parse it. | Do not add it to new configs; use `registry_idempotency_guard_scope` instead. |
| `structured_evidence_required_for_selected_contracts` | `true` | Require structured evidence for selected agent-loop contracts before final answer. | Keep route-delta and verifier attribution in canary runs; temporarily disable only as a rollback if selected contracts show false evidence gaps. |

## Attribution Requirements

Use the existing `TaskJournal` path first. Do not create a separate opaque log path unless a stable export is needed.

Required fields for rollout comparison:

- `rollout_switches_enabled`
- `rollout_attribution[]`
- `route_result.route_gate_kind`
- `route_result.initial_gate_ref`
- `route_result.initial_hint_ref`
- `route_result.legacy_first_layer_decision` only as legacy compatibility attribution
- `route_result.route_reason`
- `output_contract.semantic_kind`
- `output_contract.response_shape`
- `answer_verifier_summary.pass`
- `answer_verifier_summary.missing_evidence_fields`
- `final_failure_attribution`
- `task_metrics.llm_calls_per_task`
- `task_metrics.llm_elapsed_ms_per_task`
- `task_metrics.by_prompt`
- `evidence_coverage`
- `ask_state_transitions`

When a shadow path is added, extend the journal with machine fields only:

- `initial_gate_ref`
- `initial_hint_ref`
- `old_first_layer_decision`
- `agent_decision`
- `decision_delta`
- `capability_delta`
- `risk_delta`
- `output_contract_delta`
- `budget_profile`
- `final_outcome`
- `verifier_pass`

Do not store full prompts, raw secret-bearing tool payloads, private user text, or API keys in rollout attribution.

Current behavior-level rollout attribution:

- `answer_verifier_enforce_required_scope` required-evidence blocks write `switch_name`, `event`, `outcome`, `reason_code`, `failure_attribution`, `missing_evidence_fields`, and `confidence`.
- `registry_idempotency_guard_scope` action-level repeat blocks write `switch_name`, `event`, `outcome`, `reason_code`, `skill`, `action`, `dedup_scope`, `fingerprint`, `repeat_count`, and `limit`.
- `semantic_route_authority` writes boundary-context semantic-routing fields, selected eligibility bucket, `route_gate_kind`, `initial_gate_ref`, `initial_hint_ref`, compatibility `old_first_layer_decision`, and first planner-action delta attribution. Under `agent_loop_default`, eligible low-risk ordinary semantic decisions are owned by the planner loop; `legacy` is the emergency rollback token.

## Test Gates

Run these before and after changing finalizer, verifier, planner boundaries, registry metadata, or loop budgets.

Minimum local gate:

```bash
python3 scripts/check_no_nl_hardmatch.py
cargo check -p clawd -p claw-core
bash scripts/nl_tests/run_suite.sh contract_matrix_offline
```

Targeted Rust tests by area:

```bash
cargo test -p clawd answer_verifier -- --nocapture
cargo test -p clawd loop_control -- --nocapture
cargo test -p clawd execution_loop -- --nocapture
cargo test -p clawd task_journal -- --nocapture
cargo test -p clawd support -- --nocapture
cargo test -p claw-core skill_registry -- --nocapture
```

Release and service gate when runtime behavior changes:

```bash
cargo build --release -p clawd -p skill-runner
setsid -f bash -lc 'set -a; source /home/guagua/runtime_env_filled.sh; set +a; exec /home/guagua/rustclaw/target/release/clawd --config /home/guagua/rustclaw/configs/config.toml >> /home/guagua/rustclaw/clawd-runtime.log 2>&1'
```

NL gates:

```bash
bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt --case-limit 20 --skip-smoke --quality-guard --verbose-turn-output
bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt --case-limit 500 --prompt-reply-only --quality-guard
bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt --prompt-reply-only --quality-guard
```

Use the printed `RESUME_HINT` after interruption; do not restart a large aggregate from the beginning unless the goal is to measure a fresh baseline.

After each client-like NL run, persist rollout metrics:

```bash
python3 scripts/nl_tests/summarize_rollout_metrics.py <RUN_DIR> --provider minimax-openai-compat --vendor minimax --budget-profile default
```

The metrics JSON is written to `logs/agent_rollout_metrics/` and aggregates only counts, enums, timings, switch names, and reason codes. It must not contain prompts, final replies, full tool JSON, or secrets.

## Rollback Thresholds

Rollback the changed switch or code path when any of these occur during focused smoke, canary, or safe aggregate:

- NL pass rate drops by more than 2% from the latest comparable baseline.
- Clarification rate rises by more than 5%.
- Verifier block rate rises by more than 3% and manual sampling shows more than 1% false blocks.
- Average LLM call count rises by more than 15%.
- Average task elapsed time rises by more than 20%.
- Retry/no-progress/repeat-action stops noticeably increase.
- Any real side-effect action bypasses confirmation or dry-run.
- `python3 scripts/check_no_nl_hardmatch.py` reports unknown production hard matches.

Rollback procedure:

1. Disable or restore the relevant config value when a wired switch exists.
2. Restart `clawd` with the known-good config and environment.
3. Re-run the focused NL cases that failed.
4. If rollback requires code, revert only the responsible change set; do not reset unrelated user work.
5. Record the failed run directory, changed control, reason code, and follow-up plan.

## Current Known Risks

- `answer_verifier_enforce_required_scope` is now config-read and current config uses `all`. The required-evidence failure payload is structured and behavior attribution is available in `rollout_attribution[]`; if false blocks appear, roll back to `selected_agent_loop` or `off`.
- `semantic_route_authority=agent_loop_default` is the current default. The remaining route-label deletion work is release-gated: do not remove global legacy fallback / rollback paths until 500 canary, 2100 safe aggregate or equivalent coverage, and route-delta review show no unexplained mismatch.
- Some finalizer paths still emit structured machine fields directly. This is allowed for exact machine contracts, but directory/user-summary classes need continued audit under P1/P5.
- Directory summary requests have a focused repair for non-explicit command-output routes with machine `directory_purpose` repair markers, and `system_basic.inventory_dir` now exposes `size_summary`. Composite directory requests can still straddle `directory_entry_groups` and `directory_purpose_summary`; keep this boundary under P1/P2 review before expanding directory-summary canaries.
- The `prompts/schemas` focused case now passes by emitting structured `largest.*`, `size_bytes`, `content_excerpt`, and `directory_purpose_summary` evidence when model synthesis conflicts with observed file sizes. This is safer than a wrong prose answer, but user-facing readability remains a P1/P5 follow-up; do not solve it with runtime zh/en fixed templates.
- Registry `effect`, `once_per_task`, `dedup_scope`, and `idempotent` parse through `claw-core`; both main and docker registries explicitly declare those action-level governance fields for existing `planner_capabilities`. Execution-loop consumption now uses `registry_idempotency_guard_scope=all` and records rollout attribution when it blocks action-level repeats; roll back to `selected_agent_loop` or `off` if false repeat blocks appear.
- `execution_recipe::classify_skill_action_effect()` remains registry-first for `planner_capabilities[].effect`; it now also uses registry `side_effect=false` as a read-only fallback before legacy skill-name compatibility branches. Keep `run_cmd` / `http_basic` / `service_control` protocol-specific effect detection until equivalent registry metadata and tests exist.
- `configs/agent_guard.toml` domain action lists are now marked `DEPRECATED` and covered by a support test proving they do not affect `AgentLoopGuardPolicy`. Prompt text / `dynamic_rules` still need P4 ownership cleanup before any runtime use.
- Crypto account access failures now prefer skill-provided structured fields (`extra.error_kind`, `message_key`, `exchange`, `detail`, `status_code`) and deterministic runtime output uses machine fields instead of fixed English reply templates. Legacy sentinel parsing remains only as compatibility fallback.
- Contract-matrix preflight action/argument rejections now carry stable machine `reason_code` values in `extra` (`contract_action_rejected`, `contract_arg_rejected`), with focused `contract_matrix_preflight` tests covering both paths. P4 still needs a full allow/block/repair reason-code audit before enabling broader behavior changes.
- Verifier issues now expose stable `verify_*` reason codes in task journal summary/trace; the human-readable `blocked_reason` remains secondary context and must not be parsed for control flow.
- Verifier unresolved-template and unresolved-capability fallback responses now emit `message_key` + `reason_code` machine payloads instead of zh/en fixed user-facing templates. A renderer/i18n layer should turn these into prose when product UX needs it.
- Memory alias acknowledgements and runtime approval-wait status direct responses now emit `message_key` + `reason_code` machine payloads (`clawd.msg.memory.alias_*`, `clawd.msg.runtime.approval_wait_status`) instead of fixed zh/en prose. Keep renderer/i18n responsible for final user-language prose.
- Policy-block default delivery and normalized skill errors now emit `clawd.msg.policy.<reason_code>` + `reason_code` + `observed_facts` machine payloads. Policy boundary prose remains only contract guidance for synthesis, not deterministic user-facing fallback text.
- Ask runtime failure default delivery now emits `clawd.msg.ask_runtime_failure` + `ask_runtime_failure` machine payload. The fallback contract still carries user-request context and safety boundaries for synthesis.
- Resume `ExecutionFailedStep` deterministic answers now emit `clawd.msg.execution.failed_step` + `execution_failed_step` with action/command/exit_code/detail machine fields instead of zh/en formatted failure prose.
- Direct config edit deterministic fallbacks now emit `clawd.msg.config_edit.*` + `config_edit_*` machine payloads with path/field/value/valid/risk fields instead of zh/en success, plan, validation, guard, or read-back prose.
- Self-extension temporary/permanent plan, success, and failure defaults now emit `clawd.msg.self_extension.*` + `self_extension_*` machine payloads with skill_path/phase/detail/count/status fields instead of zh/en deterministic prose.
- Agent resume step failure defaults now emit `clawd.msg.execution.step_failed` / `clawd.msg.execution.step_error_missing` machine payloads instead of zh/en "step could not be completed" prose.
- Dispatch support deterministic observed execution status uses the same `clawd.msg.execution.step_error_missing` machine payload for missing error text.
- RustClaw config risk deterministic fallback now emits `clawd.msg.config_risk.summary` + `config_risk_*` machine payloads with path/risk_count/risks fields instead of fixed risk/no-risk prose.
- Docker and main registry copies are now covered by `scripts/check_skill_registry_parity.py`; run `--mode p3 --strict` and `--mode all --strict` before registry-governed behavior changes.

## Current Exclusions

Do not run these as real external calls in the current NL canary:

- X API publish, fetch, delete, or remote write operations, except dry run / mock / sandbox.
- Exchange order submission or remote mutation without explicit confirmation and dry run policy.
- Image generation/editing and audio transcription/synthesis real calls.
- Any skill requiring secrets that are not available through `/home/guagua/runtime_env_filled.sh`.

## Recent Baseline References

- `scripts/nl_suite_logs/contract_matrix_offline/20260604_130322`: contract matrix offline suite passed.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_125206`: aggregate case 1289 single passed after Git state language synthesis fix.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_125321`: aggregate cases 1289-1294 passed.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_125950`: aggregate cases 1286-1288 passed.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_131321`: default-off rollout switch smoke, aggregate case 1294 passed after release restart.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_132211`: default-off rollout switch smoke, aggregate case 1294 passed after release rebuild/restart; checked `clawd` PID `1045027`.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_132453`: focused smoke cases 1-17 passed; case 18 failed on schema directory summary completeness.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_134241`: aggregate case 18 passed after adding `system_basic.inventory_dir.size_summary`; count/list completeness remains a P1 risk.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_134518`: focused smoke cases 19-20 passed after release rebuild/restart; checked `clawd` PID `1061754`.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_135647`: aggregate case 18 passed after contract repair, using `fs_basic.list_dir` with `counts.files=22`; checked `clawd` PID `1069411`.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_151931`: aggregate case 1278 passed with structured `process_basic.port_list` evidence.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_152051`: aggregate case 1275 passed; no clarify regression, with high LLM cost noted.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_152643`: aggregate case 1276 passed using `config_basic.read_field`; earlier `run_20260604_152327` failed before business logic due provider/model connectivity.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_153430`: aggregate case 1278 passed after answer-verifier payload change and release restart; checked `clawd` PID `1080137`.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_162631`: focused case 9 passed after generic path content summary contract repair.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_162707`: focused cases 14-17 passed; case 18 failed due directory-purpose finalizer/verifier mismatch.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_164422`: aggregate case 18 failed because model synthesis still misreported largest file despite correct structured evidence.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_165156`: aggregate case 18 passed after finalizer required true file token + true `size_bytes` before reusing synthesis and otherwise emitted structured observed evidence.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_165428`: focused cases 19-20 passed after latest release restart; checked `clawd` PID `1132886`.
- `scripts/nl_suite_logs/contract_matrix_offline/20260604_170148`: contract matrix offline suite passed after `git_basic` generated prompt sync.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_170223`: git status case 47 passed with `git_basic.structured_json_v1` and structured `extra.field_value` evidence.
- `cargo test -p rss-fetch-skill -- --nocapture`: P1.1 long-text skill evidence scan passed with existing `extra.field_value/items` coverage.
- `cargo test -p web-search-extract-skill -- --nocapture`: P1.1 long-text skill evidence scan passed after adding non-empty candidate evidence coverage.
- `cargo test -p image-generate-skill -- --nocapture`, `cargo test -p image-edit-skill -- --nocapture`, `cargo test -p image-vision-skill -- --nocapture`: image skill contract checks passed offline; real image/audio NL calls remain excluded from aggregate canaries.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_171011`: RSS case 743 failed before fix because provider-safe evidence redacted `extra.field_value.titles[1]` as secret-like text.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_171916`: RSS case 743 passed after title-array evidence redaction fix.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_173318`: RSS case 743 failed after retry exhaustion because model synthesis mismatched `extra.items[].source_host`; final failure attribution was `contract_gap`.
- `cargo test -p clawd loop_control -- --nocapture`: passed after adding RSS verifier-exhausted structured recovery and pass-after-verifier `source_host` fidelity fallback.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_172122`: browser_web case 744 passed; web_search_extract case 745 failed because no search backend env/API was configured.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_172534`: web_search_extract case 745 passed with `WEB_SEARCH_ALLOW_DDG=1`; result_count was zero, so backend quality remains a configuration/network risk rather than a routing/evidence regression.
- `scripts/nl_suite_logs/contract_matrix_offline/20260604_172747`: contract matrix offline suite passed after RSS title-array evidence redaction fix.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_174614`: RSS case 743 passed, but trace showed final prose still omitted `source_host` machine tokens, so the pass-after-verifier fidelity guard was added before treating this as closed.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_175408`: RSS case 743 passed with final `title/source_host/date` field list; trace logged `rss_source_host_fidelity_recovered_with_structured_items count=3`.
- `cargo test -p clawd finalize::task -- --nocapture`: passed after gating non-loop verifier forced failure behind `answer_verifier_enforce_required`.
- `cargo test -p clawd agent_engine::support -- --nocapture`: passed after reusing `agent_guard.toml` rollout switch parsing for the non-loop gate.
- `cargo build --release -p clawd -p skill-runner`: passed after non-loop verifier gate; latest restarted `clawd` PID `1167678`.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_180235`: focused cases 1-20 passed after non-loop verifier gate; `NL_ATTRIBUTION_OK finalizer_overwrite=2 pass=18`, `PROMPT_BUDGET_OK prompt_truncations=0`. Cost watch: heavy turns included cases 14, 15, 16, 18, 19 and case 1.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_221105`: missing-file clarify/path follow-up cases 128-129 passed after path-only language inheritance fix; final reply stayed Chinese and did not include execution summary.
- `scripts/nl_suite_logs/client_like_continuous/run_20260604_221226`: config-file clarify/path follow-up cases 112-113 passed after active-clarify fast path; case 113 used `llm_calls=0`, `steps=1`.
- `logs/agent_rollout_metrics/run_20260604_221105_rollout_metrics.json` and `logs/agent_rollout_metrics/run_20260604_221226_rollout_metrics.json`: focused rollout metric summaries generated with provider/vendor/budget/language/semantic/capability buckets.
- `scripts/check_skill_registry_parity.py --mode p3 --strict` and `--mode all --strict`: passed after syncing `docker/config/skills_registry.toml` to the main registry; latest P3 JSON report at `logs/agent_rollout_metrics/skill_registry_parity_p3_20260604_after_sync.json`.
- `cargo test -p claw-core skill_registry -- --nocapture`: passed after adding registry action governance fields `once_per_task`, `dedup_scope`, and `idempotent`.
- `python3 scripts/sync_registry_governance_fields.py`: passed with `missing=0` for both `configs/skills_registry.toml` and `docker/config/skills_registry.toml`.
- `cargo test -p clawd registry_idempotency_guard -- --nocapture`, `cargo test -p clawd execution_loop -- --nocapture`, and `cargo test -p clawd rollout_attribution -- --nocapture`: passed after registry idempotency guard consumption and rollout attribution; this originally validated selected-scope canary behavior, and current config has since advanced to `registry_idempotency_guard_scope=all`.
- `cargo test -p clawd support --quiet`, `cargo test -p clawd answer_verifier --quiet`, `cargo test -p clawd execution_loop --quiet`, and `cargo test -p clawd finalize --quiet`: passed after adding `answer_verifier_enforce_required_scope=selected_agent_loop`; this originally validated selected-scope canary behavior, and current config has since advanced to `answer_verifier_enforce_required_scope=all`.
- `scripts/nl_suite_logs/client_like_continuous/run_20260617_223640` and `run_20260617_223838`: MiniMax verifier selected-scope canary passed 3/3; rollout metrics at `logs/agent_rollout_metrics/multi_2_run_20260617_223640_to_run_20260617_223838_rollout_metrics.json`, route-delta `unexplained_mismatch_count=0`, verifier pass count 3.
- `cargo test -p clawd market_quote_scalar -- --nocapture`: passed after replacing the `crypto|stock` observed-output skill-name branch with registry `semantic_tags=["market_quote_scalar"]`.
- `cargo test -p crypto-skill account_access -- --nocapture`, `cargo test -p clawd account_access -- --nocapture`, and `cargo test -p clawd crypto_account_error -- --nocapture`: passed after moving crypto account errors to structured `extra.error_kind/message_key` and machine-field deterministic output.
- `cargo test -p clawd visible_text -- --nocapture`: passed after preserving allowed i18n `message_key` machine fields during user-visible sanitization.
