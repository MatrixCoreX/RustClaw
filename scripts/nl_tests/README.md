# NL Tests

All natural-language test scripts are centralized in this directory.

## Unified Tool

Primary entry point:

- `bash scripts/nl_tests/run_suite.sh --list`
- `bash scripts/nl_tests/run_suite.sh contract_matrix_offline`
- `bash scripts/nl_tests/run_suite.sh runtime_capability_boundary`
- `bash scripts/nl_tests/run_suite.sh manual`
- `bash scripts/nl_tests/run_suite.sh compound_single`
- `bash scripts/nl_tests/run_suite.sh task_updates`
- `bash scripts/nl_tests/run_suite.sh task_updates4`
- `bash scripts/nl_tests/run_suite.sh multistep_mixed`
- `bash scripts/nl_tests/run_suite.sh manual trace clarify`
- `bash scripts/nl_tests/run_suite.sh self_extension`
- `bash scripts/nl_tests/run_suite.sh sensitive_flows`
- `bash scripts/nl_tests/run_suite.sh ops_closed_loop`
- `bash scripts/nl_tests/run_suite.sh ops_http_repair`
- `bash scripts/nl_tests/run_suite.sh long_tail_flows`
- `bash scripts/nl_tests/run_suite.sh --category multi_turn`
- `bash scripts/nl_tests/run_suite.sh --category regression --category guard`
- `bash scripts/nl_tests/run_suite.sh --category ops`
- `bash scripts/nl_tests/run_suite.sh all`
- `bash scripts/nl_tests/run_suite.sh clarify_context_prompt`

Built-in categories:

- `smoke`
- `single_turn`
- `multi_turn`
- `regression`
- `guard`
- `ops`
- `core`
- `all`

Shared options are passed through to the underlying runners, for example:

- `bash scripts/nl_tests/run_suite.sh manual --base-url http://127.0.0.1:8787`
- `bash scripts/nl_tests/run_suite.sh --category multi_turn --chat-id 3000000`

Runner output prints each user instruction and assistant answer as `PROMPT` / `REPLY`
blocks by default. Use `--prompt-reply-only` when you want to suppress most
diagnostic output and keep only those dialog blocks.

Static runtime hard-match guard:

- `python3 scripts/check_no_nl_hardmatch.py`
- `python3 scripts/check_no_nl_hardmatch.py --self-test`

The guard fails on new user-language phrase matching in runtime Rust code. Known
legacy hits are reported with their owning plan item and should be removed by
the structured-contract migration instead of expanded with more phrases.

Static compact coverage guard:

- `python3 scripts/nl_tests/check_compact_coverage.py`
- `python3 scripts/nl_tests/check_compact_coverage.py --report`

This guard does not call `clawd` or a model. It verifies that the source-controlled
compact tier files cover the required basic skill, route/lifecycle, and media
dry-run classes, including clarify/direct-answer/act/recover control-trace and
repair-envelope cases, that default compact media rows are dry-run only, and
that X/Twitter live publish tags are not part of the compact gate.

Agent parity gate:

- `bash scripts/nl_tests/run_suite.sh agent_parity_gate`
- `bash scripts/nl_tests/run_agent_parity_gate.sh`
- `bash scripts/nl_tests/run_agent_parity_gate.sh scripts/nl_suite_logs/client_like_continuous/<run_id>`

This is the default lightweight gate after a Codex/Claude-style agent-loop
implementation batch. It runs the static compact coverage check, the
shared secret scan contract, Chinese-provider model catalog guard and self-test, a dry-run
Chinese-provider smoke matrix with MiniMax as the default live scope, the
offline coding-loop repair fixture expectations, bounded rollout metrics
for that fixture, and the rollout metrics portable path contract. MiMo, Qwen, and DeepSeek remain in the metadata matrix but
are recorded as out of live scope unless `--chinese-live-providers all` or a
provider CSV is passed. When you pass
one or more finished client-like run directories, it also applies the same
metrics gates to the real NL run. The defaults require
`pass_rate=1.0`, `avg_llm_calls_per_turn<=4`, no prompt truncation, and no final
provider errors. Override with `--min-pass-rate`, `--max-avg-llm-calls`,
`--max-prompt-truncations`, `--max-provider-final-errors`, or environment
variables with the same uppercase names. The Chinese-provider model catalog
guard and smoke preflight use `CHINESE_PROVIDER_ENV_FILE` or
`../runtime_env_filled.sh` when present, and record only env-file state plus
env-file source tokens plus secret-free credential metadata; artifacts do not
write the env-file path or secret values. Use `--chinese-env-file <path>` to
override it, or `--no-chinese-env-file` for a pure missing-credential preflight.
The gate writes `agent_loop_static_contracts.txt` from the route-authority
legacy-key guard, legacy route boundary guard, pre-planner removal guard, NL
hard-match scanner, and historical hardcoded-language scanner. `gate_summary.env`
records `agent_loop_static_contracts=1`, so artifact readers can tell the
Codex/Claude-style agent-loop boundary checks were part of the run. The artifact
also records `AGENT_LOOP_STATIC_SELF_TEST ...` labels before the main checks, so
the route-authority, legacy-route, pre-planner, NL-hardmatch, and hardcoded
language guards prove their self-tests before the run is trusted.
The gate also writes `runtime_hard_reply_baseline.txt` from
`scripts/check_no_runtime_hard_reply.py --self-test` plus the baseline scan.
`gate_summary.env` records `runtime_hard_reply_baseline=1`, and the artifact
must contain `RUNTIME_HARD_REPLY_ALL_SCAN` plus `new=0`, so newly-added
production Rust sentence-like literals cannot quietly become fixed user-facing
reply templates.
It also writes `policy_boundary_hard_reply.txt` from
`scripts/check_no_policy_boundary_hard_reply.py --self-test` plus the main
check. `gate_summary.env` records `policy_boundary_hard_reply=1`, and the
artifact must contain `POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok` and
`POLICY_BOUNDARY_HARD_REPLY_CHECK ok`, so policy-boundary/final-reply contracts
do not grow fixed prose reply rules.
It also writes `repair_no_user_text_fields.txt` from
`scripts/check_repair_no_user_text_fields.py --self-test` plus the main check.
`gate_summary.env` records `repair_no_user_text_fields=1`, and the artifact
must contain `SELF_TEST_OK` and `REPAIR_USER_TEXT_FIELD_CHECK ok`, so repair and
loop recovery boundaries do not treat user-visible `text/error_text` as machine
protocol.
It also writes `policy_decision_tokens.txt` from
`scripts/check_policy_decision_tokens.py --self-test` plus the main check.
`gate_summary.env` records `policy_decision_tokens=1`, and the artifact must
contain `POLICY_DECISION_TOKEN_SELF_TEST ok` and
`POLICY_DECISION_TOKEN_CHECK ok`, so permission, confirmation, and background
wait decisions keep flowing through the `PolicyDecision` machine-token enum.
It also writes `agent_loop_guard_final_scope.txt` from
`scripts/check_agent_loop_guard_final_scope.py --self-test` plus the main check.
`gate_summary.env` records `agent_loop_guard_final_scope=1`, and the artifact
must contain `AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok` and
`AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0`, so answer-verifier evidence
and registry idempotency scopes stay final `all` machine boundaries.
It also writes `registry_policy_contracts.txt`,
`skill_registry_aliases.txt`, and `long_tail_skill_contracts.txt` from
`scripts/check_registry_policy_contracts.py --self-test`,
`scripts/check_skill_registry_aliases.py --self-test`, and
`scripts/check_long_tail_skill_contracts.py --self-test` plus their main checks.
`gate_summary.env` records `registry_policy_contracts=1`,
`skill_registry_aliases=1`, and `long_tail_skill_contracts=1`, so planner
capability policy metadata, language-neutral aliases, and long-tail async
contracts are release artifact contracts. The artifacts must contain
`REGISTRY_POLICY_CONTRACT_SELF_TEST ok`, `REGISTRY_POLICY_CONTRACT_CHECK ok`,
`SKILL_REGISTRY_ALIAS_SELF_TEST ok`, `SKILL_REGISTRY_ALIAS_CHECK ok`,
`LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok`, and
`LONG_TAIL_SKILL_CONTRACT_CHECK ok`.
It also writes `task_lifecycle_contracts.txt` from
`scripts/check_task_lifecycle_contracts.py --self-test` plus the main check.
`gate_summary.env` records `task_lifecycle_contracts=1`, and the artifact must
contain `TASK_LIFECYCLE_CONTRACT_SELF_TEST ok` and
`TASK_LIFECYCLE_CONTRACT_CHECK findings=0`, so background task lifecycle,
checkpoint/resume, resume executor lease, and seeded agent-loop resume stay
machine-field driven.
It also writes `task_event_context_team_contracts.txt` from
`scripts/check_task_event_context_team_contracts.py --self-test` plus the main
check. `gate_summary.env` records `task_event_context_team_contracts=1`, and
the artifact must contain `TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok` and
`TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0`, so `task_goal`,
`context_budget`, `context_compaction`, provider prompt-budget metrics, coding
evidence, and read-only subagent/team events remain structured event-stream
fields.
It also writes `clawcli_exec_replay_contracts.txt` from
`scripts/check_clawcli_exec_replay_contracts.py --self-test` plus the main
check. `gate_summary.env` records `clawcli_exec_replay_contracts=1`, and the
artifact must contain `CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok` and
`CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0`, so the exec artifact set,
compact `exec_compact_*` output, recorded-only replay views, coverage, and
diff class contracts remain machine-field driven.
It also writes `clawcli_session_tui_contracts.txt` from
`scripts/check_clawcli_session_tui_contracts.py --self-test` plus the main
check. `gate_summary.env` records `clawcli_session_tui_contracts=1`, and the
artifact must contain `CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok` and
`CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0`, so the session store, local
session metadata, TUI selected task snapshot, selected progress/summary,
operator key tokens, and TUI control projections remain machine-field driven.
It also writes `clawcli_goal_contracts.txt` from
`scripts/check_clawcli_goal_contracts.py --self-test` plus the main check.
`gate_summary.env` records `clawcli_goal_contracts=1`, and the artifact must
contain `CLAWCLI_GOAL_CONTRACT_SELF_TEST ok` and
`CLAWCLI_GOAL_CONTRACT_CHECK findings=0`, so goal payload, goal status, goal
control summaries, resume/checkpoint fields, verification commands, and
sensitive-field redaction remain machine-field driven.
It also writes `clawcli_llm_trace_contracts.txt` from
`scripts/check_clawcli_llm_trace_contracts.py --self-test` plus the main check.
`gate_summary.env` records `clawcli_llm_trace_contracts=1`, and the artifact
must contain `CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok` and
`CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0`, so `clawcli llm-trace`,
`LLM#1..N` / `llm_call_ref=LLM#` numbering, flow/code attribution, usage
tokens, raw request/response fields, and UI teaching trace helpers remain
machine-field driven.
It also writes `clawcli_models_catalog_contracts.txt` from
`scripts/check_clawcli_models_catalog_contracts.py --self-test` plus the main
check. `gate_summary.env` records `clawcli_models_catalog_contracts=1`, and the
artifact must contain `CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok` and
`CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0`, so `clawcli models catalog`,
`model_catalog_summary`, `model_catalog_entry`, `credential_state`, provider
filtering, modalities, capability flags, async/dry-run metadata, and UI model
catalog display remain secret-free machine-field contracts.
It also writes `clawcli_models_readiness_contracts.txt` from
`scripts/check_clawcli_models_readiness_contracts.py --self-test` plus the main
check. `gate_summary.env` records `clawcli_models_readiness_contracts=1`, and
the artifact must contain `CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok` and
`CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0`, so `clawcli models
readiness`, `clawcli llm-trace`, `model_readiness_summary`,
`selected_entry_status`, `model_catalog_trace.readiness`, `credential_state`,
`ready`, capability flags, async/dry-run metadata, UI teaching trace tokens, and
missing selected-entry behavior remain secret-free machine-field contracts.
It also writes `semantic_boundary_contracts.txt` from
`scripts/check_runtime_semantic_rewrite_boundary.py --self-test`,
`scripts/check_contract_repair_loop_observation_boundary.py --self-test`,
`scripts/check_route_reason_marker_facade.py --self-test`, and
`scripts/check_output_semantic_kind_write_boundary.py --self-test` plus their
main checks. `gate_summary.env` records `semantic_boundary_contracts=1`, and
the artifact must contain `RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0`,
`CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0`,
`ROUTE_REASON_MARKER_FACADE_SELF_TEST ok`,
`ROUTE_REASON_MARKER_FACADE_CHECK findings=0`,
`OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok`, and
`OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK findings=0`.
It also writes `agent_architecture_boundary_contracts.txt` from
`scripts/check_boundary_envelope_schema.py --self-test`,
`scripts/check_intent_normalizer_boundary_schema.py --self-test`,
`scripts/check_planner_no_pre_llm_deterministic_fast_path.py --self-test`,
`scripts/check_capability_resolver_registry_only.py --self-test`,
`scripts/check_finalizer_boundary.py --self-test`, and
`scripts/check_evidence_policy_facade_boundary.py --self-test` plus their main
checks. `gate_summary.env` records
`agent_architecture_boundary_contracts=1`, and the artifact must contain
`BOUNDARY_ENVELOPE_SCHEMA_CHECK findings=0`,
`INTENT_NORMALIZER_BOUNDARY_SCHEMA_CHECK findings=0`,
`PLANNER_PRE_LLM_DETERMINISTIC_FAST_PATH_CHECK strict_tests=false findings=0`,
`CAPABILITY_RESOLVER_REGISTRY_ONLY_CHECK findings=0`,
`FINALIZER_BOUNDARY_CHECK ok`, and
`EVIDENCE_POLICY_FACADE_BOUNDARY_CHECK strict=false findings=0`.
It also writes `evidence_extractor_contracts.txt` from
`scripts/check_evidence_extractor_contracts.py --self-test` plus the main check.
`gate_summary.env` records `evidence_extractor_contracts=1`, and the artifact
must contain both `EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok` and
`EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings=0`, so structured tool observation
metadata cannot quietly drift back to strict language-text evidence paths.
The gate also writes `secret_scan_contract.json` from
`scripts/nl_tests/check_secret_scan_contract.py`, locking the shared scanner's
forbidden-field and secret-like-value finding shapes. `gate_summary.env`
records `secret_scan_contract=1` as a non-secret machine token so gate artifact
readers can tell the contract was part of the run.
It also writes `suite_wrapper_contract.json` from
`scripts/nl_tests/check_suite_wrapper_contract.py`, locking the wrapped-suite
summary/index/report finalizer wiring and the agent parity nested artifact
contract checker.
It also writes `runner_path_ref_contract.json` from
`scripts/nl_tests/check_runner_path_ref_contract.py`, locking the full/manual,
multi-turn, client-like, provider A/B, dynamic-guard, ops, task-termination,
and circuit-breaker runner console outputs to portable path refs rather than
host absolute paths.
When launched through `run_suite.sh agent_parity_gate`, the gate stores its JSON
artifacts under the same suite run directory at `agent_parity_gate/`; direct
runs still default to `logs/agent_parity_gate/<timestamp>` unless `--out-dir` is
provided. `gate_summary.env` records the artifact location as the portable
`out_dir_ref` machine field instead of a host absolute `out_dir` path. Wrapped
suite runs also write `artifact_index.txt` at the suite run
root, listing run-root-relative nested artifacts such as
`agent_parity_gate/gate_summary.env`,
`agent_parity_gate/runtime_hard_reply_baseline.txt`,
`agent_parity_gate/policy_boundary_hard_reply.txt`,
`agent_parity_gate/repair_no_user_text_fields.txt`,
`agent_parity_gate/policy_decision_tokens.txt`,
`agent_parity_gate/agent_loop_guard_final_scope.txt`,
`agent_parity_gate/registry_policy_contracts.txt`,
`agent_parity_gate/skill_registry_aliases.txt`,
`agent_parity_gate/long_tail_skill_contracts.txt`,
`agent_parity_gate/task_lifecycle_contracts.txt`,
`agent_parity_gate/task_event_context_team_contracts.txt`,
`agent_parity_gate/clawcli_exec_replay_contracts.txt`,
`agent_parity_gate/clawcli_session_tui_contracts.txt`,
`agent_parity_gate/clawcli_goal_contracts.txt`,
`agent_parity_gate/clawcli_llm_trace_contracts.txt`,
`agent_parity_gate/clawcli_models_catalog_contracts.txt`,
`agent_parity_gate/clawcli_models_readiness_contracts.txt`,
`agent_parity_gate/agent_loop_static_contracts.txt`,
`agent_parity_gate/agent_architecture_boundary_contracts.txt`,
`agent_parity_gate/evidence_extractor_contracts.txt`, and
`agent_parity_gate/secret_scan_contract.json` for easier resume and review.
They also write `suite_summary.env` with machine fields `suite`, `status`,
`exit_code`, `artifact_finalize_status`, `run_log`, and `artifact_index` so a
later task can resume from the run root without parsing prose logs. `status` and
`exit_code` reflect the wrapped command; `artifact_finalize_status` reflects the
summary/index write step. Wrapped console output and `run.log` print
`run_dir_ref`, `run_log_ref`, and artifact refs instead of host absolute paths.
The `clarify_context_prompt` helper also prints `clarify_*_ref` and
`context_*_ref` fields for Codex-style review prompts, so copied test context
does not depend on the local workspace path.
Multi-turn suite runners also print `run_dir_ref` and `run_log_ref` instead of
host absolute paths, so teaching/replay output can be shared without leaking a
local machine path.
Validate a wrapped run root with
`python3 scripts/nl_tests/check_suite_artifact_contract.py <run_dir> --json`.
Wrapped runs also write this validation result to
`suite_artifact_contract.json` and list it in `artifact_index.txt`. The
auto-generated report uses `run_dir="."` so the artifact can be moved or shared
without embedding the local workspace path. Auto-generated reports are strict:
they are created with `--require-contract-report`, so the report must also be
present in `artifact_index.txt`; the final write also uses
`--validate-contract-report-content` so the existing report must be parseable,
`ok=true`, finding-free, and aligned with the current summary and nested
contract counts. The final confirmation uses
`--require-contract-report-content-checked`, so the stored report must already
carry `contract_report_content_checked=true`. For wrapped `agent_parity_gate`
runs, the artifact contract also validates nested gate artifacts such as
`agent_parity_gate/agent_loop_static_contracts.txt` and
`agent_parity_gate/evidence_extractor_contracts.txt`,
`agent_parity_gate/runner_path_ref_contract.json`,
`agent_parity_gate/suite_artifact_contract_self_test.txt`,
`agent_parity_gate/rollout_metrics_contract.txt`, statically guards
the artifact checker's dynamic machine fields such as the Chinese provider live
scope, then checks
`agent_parity_gate/gate_summary.env` for the non-secret machine flags that prove
the runtime hard-reply, policy-boundary hard-reply, repair no-user-text,
policy-decision-token, registry-policy, registry-alias, long-tail-skill,
task-lifecycle, task-event/context/team, clawcli model-readiness, static
agent-loop, evidence-extractor, secret-scan, wrapper, no-agent-mode,
suite-artifact self-test, and raw LLM trace contracts participated in the run.
For `runtime_hard_reply_baseline.txt`, the required content includes
`SELF_TEST_OK`, `RUNTIME_HARD_REPLY_ALL_SCAN`, and `new=0`.
For `policy_boundary_hard_reply.txt`, the required content includes
`POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok` and
`POLICY_BOUNDARY_HARD_REPLY_CHECK ok`.
For `repair_no_user_text_fields.txt`, the required content includes
`SELF_TEST_OK` and `REPAIR_USER_TEXT_FIELD_CHECK ok`.
For `policy_decision_tokens.txt`, the required content includes
`POLICY_DECISION_TOKEN_SELF_TEST ok` and `POLICY_DECISION_TOKEN_CHECK ok`.
For `agent_loop_guard_final_scope.txt`, the required content includes
`AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok` and
`AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0`.
For `registry_policy_contracts.txt`, `skill_registry_aliases.txt`, and
`long_tail_skill_contracts.txt`, the required content includes their self-test
tokens plus `REGISTRY_POLICY_CONTRACT_CHECK ok`, `SKILL_REGISTRY_ALIAS_CHECK ok`,
and `LONG_TAIL_SKILL_CONTRACT_CHECK ok`.
For `task_lifecycle_contracts.txt`, the required content includes
`TASK_LIFECYCLE_CONTRACT_SELF_TEST ok` and
`TASK_LIFECYCLE_CONTRACT_CHECK findings=0`. This proves checkpoint/resume,
resume executor leases, seeded agent-loop resume, async poll/cancel projection,
and CLI/UI task lifecycle display still use machine fields instead of
user-visible `text/error_text` as protocol.
For `task_event_context_team_contracts.txt`, the required content includes
`TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok` and
`TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0`. This proves task goal,
context budget/compaction, provider prompt budget metrics, coding evidence, and
subagent/team lifecycle events stay machine-readable for CLI, UI, teaching mode,
and replay tooling.
For `clawcli_exec_replay_contracts.txt`, the required content includes
`CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok` and
`CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0`. This proves `clawcli exec` and
`clawcli code` keep their exec artifact and compact output contracts, while
`clawcli replay export/run/diff` stays recorded-only replay with coverage,
view, and diff class machine fields.
For `clawcli_session_tui_contracts.txt`, the required content includes
`CLAWCLI_SESSION_TUI_CONTRACT_SELF_TEST ok` and
`CLAWCLI_SESSION_TUI_CONTRACT_CHECK findings=0`. This proves `clawcli session`
keeps machine-readable session store metadata and resume controls, while
`clawcli tui` keeps selected task snapshots, `selected_progress`,
`selected_summary`, operator key tokens, and report/review/subagents/permission
projections as machine fields.
For `clawcli_goal_contracts.txt`, the required content includes
`CLAWCLI_GOAL_CONTRACT_SELF_TEST ok` and
`CLAWCLI_GOAL_CONTRACT_CHECK findings=0`. This proves `clawcli goal
start/status/pause/resume/edit/clear` keeps goal payload, goal status, control
summary, `done_conditions`, `verification_commands`, resume/checkpoint fields,
and sensitive-field redaction as machine-field contracts.
For `clawcli_llm_trace_contracts.txt`, the required content includes
`CLAWCLI_LLM_TRACE_CONTRACT_SELF_TEST ok` and
`CLAWCLI_LLM_TRACE_CONTRACT_CHECK findings=0`. This proves `clawcli llm-trace`
keeps `LLM#1..N` / `llm_call_ref=LLM#` numbering, flow/code attribution,
provider/model/status/usage tokens, raw request/response fields, and UI teaching
trace helpers as machine-field contracts.
For `clawcli_models_catalog_contracts.txt`, the required content includes
`CLAWCLI_MODELS_CATALOG_CONTRACT_SELF_TEST ok` and
`CLAWCLI_MODELS_CATALOG_CONTRACT_CHECK findings=0`. This proves `clawcli models
catalog` keeps `model_catalog_summary`, `model_catalog_entry`,
`credential_state`, provider filtering, modalities, capability flags,
async/dry-run metadata, and UI model catalog display as secret-free machine
fields.
For `clawcli_models_readiness_contracts.txt`, the required content includes
`CLAWCLI_MODELS_READINESS_CONTRACT_SELF_TEST ok` and
`CLAWCLI_MODELS_READINESS_CONTRACT_CHECK findings=0`. This proves `clawcli
models readiness` keeps `model_readiness_summary`,
`model_catalog_trace.readiness`, `selected_entry_status`, `credential_state`,
`ready`, capability flags, async/dry-run metadata, `clawcli llm-trace` readiness
tokens, UI teaching trace tokens, and missing selected-entry behavior as
secret-free machine fields.
For `agent_loop_static_contracts.txt`, the required content includes the six
route/frontdoor/static `AGENT_LOOP_STATIC_SELF_TEST ...` labels as well as the
main guard success tokens, including
`AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py` and
`FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0`.
For `semantic_boundary_contracts.txt`, the required content includes
`RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0`,
`CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0`,
`ROUTE_REASON_MARKER_FACADE_SELF_TEST ok`,
`ROUTE_REASON_MARKER_FACADE_CHECK findings=0`,
`OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok`, and
`OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK findings=0`.
For `agent_architecture_boundary_contracts.txt`, the required content includes
`BOUNDARY_ENVELOPE_SCHEMA_CHECK findings=0`,
`INTENT_NORMALIZER_BOUNDARY_SCHEMA_CHECK findings=0`,
`PLANNER_PRE_LLM_DETERMINISTIC_FAST_PATH_CHECK strict_tests=false findings=0`,
`CAPABILITY_RESOLVER_REGISTRY_ONLY_CHECK findings=0`,
`FINALIZER_BOUNDARY_CHECK ok`, and
`EVIDENCE_POLICY_FACADE_BOUNDARY_CHECK strict=false findings=0`.
It also checks
that gate summary path fields use portable refs such as `out_dir_ref=out_dir`
and never host absolute paths. It also checks artifact content: text reports
must contain their success tokens, and JSON reports such as
`secret_scan_contract.json`, `suite_wrapper_contract.json`, and
`runner_path_ref_contract.json` must expose
`ok=true` from a top-level object. The suite-artifact self-test covers missing,
unreadable, malformed, and non-object contract reports; invalid Chinese live-provider scope; invalid env-file state/source; unsafe Chinese-provider smoke path refs; unsafe rollout metrics source/output refs; bad UTF-8 artifact
payloads; non-object JSON ok
artifacts; non-object loader-backed artifacts such as compact coverage; base report fields (`ok`, `run_dir`,
`require_contract_report`, `findings`); bad UTF-8 suite metadata; unchecked final reports; summary
mismatch; nested `agent_parity_gate_contract` mismatch; and unexpected nested
agent parity contracts in non-agent-parity reports. If the nested agent parity
gate summary is missing, the checker returns the structured
`agent_parity_gate_summary_missing` finding instead of crashing. Enabled optional steps are also content-checked from
`gate_summary.env`: compact coverage must have no missing/forbidden rows,
Chinese model catalog must report `status=ok` with all Chinese providers, its
`chinese_model_catalog_self_test.txt` must include `CHINESE_MODEL_CATALOG_SELF_TEST ok`
after covering TOML/env-file structured failure findings, and
the coding repair fixture metrics must satisfy the configured pass-rate,
prompt-truncation, provider-error, and average-LLM-call thresholds. When
provider smoke is enabled, the contract also checks the case coverage artifact,
provider tag shape, Chinese catalog/provider matrix row-list shape, provider
summary JSONL row shape/JSON errors, and each MiniMax/MiMo/Qwen/DeepSeek matrix row for the
expected dry-run or `provider_not_in_live_scope` reason based on
`chinese_provider_live_providers`. That scope field must be `all` or a CSV of
known Chinese provider machine tokens, and env-file state/source must remain in
their allowed machine-token sets. Provider smoke metadata path fields, including
`case_coverage.json`,
(`case_file`, `output_file`, `run_dir`) must be portable refs such as
repo-relative paths, `out_dir/...`, or `external_path`, never host absolute
paths. `live_metrics` is a required
`gate_summary.env` machine field. `metrics=1` only means "the metrics gate was
not disabled"; it does not substitute for `live_metrics`. `live_metrics=1`
means run directories were provided and `run_metrics.json` / `run_metrics.txt`
were actually generated and content-checked with portable source/output refs.
Runs without run directories keep
`live_metrics=0` even when `metrics=1`.
The JSON report includes `agent_parity_gate_contract.checked=true` plus the
required artifact, flag, machine-field, and content-check counts when this
suite-specific validation runs.

For rerun shards, use:

```bash
bash scripts/nl_tests/run_agent_parity_gate.sh \
  --dedupe-latest-case --expect-case-count 285 \
  scripts/nl_suite_logs/client_like_continuous/<run_id_1> \
  scripts/nl_suite_logs/client_like_continuous/<run_id_2>
```

This gate does not replace live affected-case NL/coding tests. It provides the
fast required preflight; after changing runtime planner, resolver, verifier,
CLI coding, or prompt layering, run the smallest affected live case file listed
below with LLM traces enabled, then feed that run directory into this gate.

Client-like continuous regression:

- Run the offline contract-matrix regression suite, including generator checks and attribution fixtures:
  `bash scripts/nl_tests/run_suite.sh contract_matrix_offline`
  This also verifies the multilingual contract-matrix generator path for zh-CN, en-US, ja-JP, ko-KR, fr-FR, and mixed-language variants.
  It first checks that the legacy client-like aggregate is up to date, so old curated NL cases and new matrix-generated cases stay in the same regression loop.
  The aggregate check also gates metadata coverage for built-in tools, skills, memory, multi-turn context, and structured transformation cases.
  The suite gates attribution fixture coverage for `model_error`, `schema_error`, `code_gap`, `contract_gap`, `tool_gap`, `permission_denied`, `budget_exhausted`, `prompt_budget_error`, `delivery_error`, and `provider_error`, plus the structured negative signals used by the evaluator. Keep multilingual behavior on contract ids, schema fields, action refs, evidence keys, and error codes; do not add runtime natural-language phrase matching for new languages.
- Generate 100 deterministic contract-matrix seed cases without calling a model:
  `python3 scripts/nl_tests/generate_contract_matrix_cases.py --count 100 --check --report > /tmp/rustclaw-contract-cases.jsonl`
  Use `--batch N` to rotate the non-mandatory cases while preserving semantic/generic, phase, policy-decision, evidence-expression, and final-answer-shape coverage.
  Add `--history /tmp/rustclaw-contract-history.jsonl --update-history` when running repeated batches; the generator prefers case ids not already in the local history file and appends the selected ids after a successful check.
- Generate 100 live NL replay rows from the same matrix coverage:
  `python3 scripts/nl_tests/generate_contract_matrix_cases.py --count 100 --check --nl --report > /tmp/rustclaw-contract-nl.jsonl`
  Add `--expectations /tmp/rustclaw-contract-nl.expectations.jsonl` to write matching evaluator expectations for contract match, allowed-action phase plan refs, executed skill family, required evidence, missing-evidence status, and final answer shape.
  Add `--multilingual-variants` to emit zh-CN, en-US, ja-JP, ko-KR, fr-FR, and mixed-language prompts for each selected contract cell while preserving the same structured `[CONTRACT_TEST_HINT]`; this is the preferred regression path for checking that multilingual wording converges to the same semantic kind, allowed action, required evidence, and final answer shape without runtime natural-language hard matching.
  Because `[CONTRACT_TEST_HINT]` is a test-matrix machine protocol and is disabled in normal runtime, start `clawd` for these live replay rows with `RUSTCLAW_ENABLE_CONTRACT_TEST_HINT=1`.
  Run them through the client-like path with:
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --skip-smoke --case-jsonl /tmp/rustclaw-contract-nl.jsonl --prompt-reply-only --quality-guard`
  Then evaluate the finished run with:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --expectations /tmp/rustclaw-contract-nl.expectations.jsonl`
- Regenerate the safe aggregate case file:
  `python3 scripts/nl_tests/build_client_like_case_aggregate.py`
  By default this writes 2,100 executable rows, padding from the existing safe
  aggregate prompts with unique case names when fewer source rows are available.
  The safe aggregate excludes configured external publishing-channel skill rows
  so long regression runs do not touch publish/draft/fetch flows.
  Use `--target-rows 0` only when you need the unpadded source aggregate.
- Check the aggregate is up to date without rewriting it:
  `python3 scripts/nl_tests/build_client_like_case_aggregate.py --check`
- Run a small slice through the real client-like path:
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt --case-limit 20 --prompt-reply-only --quality-guard`
- Run the full safe aggregate when provider capacity is available:
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt --prompt-reply-only --quality-guard`
- Resume after a provider interruption by reusing the printed `RESUME_HINT`.
- Summarize a finished client-like run with the full execution flow:
  `python3 scripts/nl_tests/summarize_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --limit 20`
  This prints each case prompt, first-layer route, planner steps, verifier result, executed tool/skill evidence, LLM metrics, and final reply so regression review does not rely on a bare OK/fail line.
- Summarize compact or large-run machine metrics:
  `python3 scripts/nl_tests/summarize_rollout_metrics.py scripts/nl_suite_logs/client_like_continuous/<run_id> --print-json`
  This records pass/fail counts, LLM calls, prompt bytes/tokens when present, elapsed time, provider retries/errors, verifier-call count, lifecycle/background counts, checkpoint counts, provider blockers, tool-call counts, and prompt latency diagnostics from existing run JSON only.
  Add absolute gates after compact runs when the touched surface is expected to stay bounded, for example:
  `python3 scripts/nl_tests/summarize_rollout_metrics.py scripts/nl_suite_logs/client_like_continuous/<run_id> --min-pass-rate 1.0 --max-avg-llm-calls 4 --max-prompt-truncations 0 --max-provider-final-errors 0`
- Generate or check a lightweight offline regression baseline:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --write-baseline /tmp/rustclaw-client-like-baseline.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --expectations scripts/nl_tests/expectations/<name>.jsonl`
  Expectation rows can assert route, planner capability/tool targets, exact planned `skill.action` refs when present in trace, executed tool/skill, structured `error_kind`, execution failure attribution, stop-signal attribution, verifier issue attribution, contract policy decision, contract match, evidence coverage, verifier approval, finalizer stage/fallback/grounding, finalizer answer shape/class, final text substrings, and final answer shape without making a new LLM request.
- Extract exact replay prompts and expectations from a finished or interrupted client-like run:
  `python3 scripts/nl_tests/extract_client_like_replay.py scripts/nl_suite_logs/client_like_continuous/<run_id> --case-jsonl /tmp/rustclaw-replay.jsonl --expectations /tmp/rustclaw-replay.expectations.jsonl`
  Add `--min-repro /tmp/rustclaw-replay.min-repro.jsonl` to also write a sanitized reproduction summary containing the request, route contract, planned/requested actions, observed and missing evidence, failure attribution, and final answer preview.
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --skip-smoke --case-jsonl /tmp/rustclaw-replay.jsonl --quality-guard --prompt-reply-only`
- Focused runtime capability boundary smoke:
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --skip-smoke --case-file scripts/nl_tests/cases/nl_cases_runtime_capability_boundary_smoke_20260515.txt --quality-guard`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --expectations scripts/nl_tests/expectations/runtime_capability_boundary_smoke_20260515.jsonl`
- Focused runtime capability boundary regression:
  `bash scripts/nl_tests/run_runtime_capability_boundary_regression.sh`
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --skip-smoke --case-file scripts/nl_tests/cases/nl_cases_runtime_capability_boundary_regression_20260515.txt --quality-guard --prompt-reply-only`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --expectations scripts/nl_tests/expectations/runtime_capability_boundary_regression_20260515.jsonl`
  The dedicated wrapper runs the fixed 20-case set, prints the full flow
  summary, and evaluates the source-controlled expectations. Use it first after
  changing prompt, registry, resolver, verifier, or observed finalizer logic.
- Offline observed-finalizer fixture smoke:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/observed_finalizer_scalar --expectations scripts/nl_tests/expectations/observed_finalizer_scalar_fixture.jsonl`
- Offline verifier issue fixture smoke:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/verifier_issue_missing_arg --expectations scripts/nl_tests/expectations/verifier_issue_missing_arg_fixture.jsonl`
- Offline contract-rejection attribution fixture smoke:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/contract_rejection_attribution --expectations scripts/nl_tests/expectations/contract_rejection_attribution_fixture.jsonl`
- Offline budget-exhausted attribution fixture smoke:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/budget_exhausted_attribution --expectations scripts/nl_tests/expectations/budget_exhausted_attribution_fixture.jsonl`
- Offline code-gap/permission/schema/tool/provider/delivery/prompt-budget attribution fixture smoke:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/code_gap_attribution --expectations scripts/nl_tests/expectations/code_gap_attribution_fixture.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/permission_denied_attribution --expectations scripts/nl_tests/expectations/permission_denied_attribution_fixture.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/schema_error_attribution --expectations scripts/nl_tests/expectations/schema_error_attribution_fixture.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/tool_gap_attribution --expectations scripts/nl_tests/expectations/tool_gap_attribution_fixture.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/provider_error_attribution --expectations scripts/nl_tests/expectations/provider_error_attribution_fixture.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/delivery_error_attribution --expectations scripts/nl_tests/expectations/delivery_error_attribution_fixture.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/prompt_budget_error_attribution --expectations scripts/nl_tests/expectations/prompt_budget_error_attribution_fixture.jsonl`
- Offline coding-loop repair fixture smoke:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_tests/fixtures/client_like_runs/coding_loop_repair --expectations scripts/nl_tests/expectations/coding_loop_repair_fixture.jsonl`
  This fixture is separate from user-facing NL evals. It checks the machine
  event contract for a small coding task that records a failing verification
  command, a file-edit checkpoint, a rerun verification checkpoint, and final
  `coding_evidence` without requiring a live model or mutating the repository.

### Compact coverage tiers

Use the smallest suite that covers the surface touched by the code change. This
keeps normal development fast while preserving traceability back to the larger
safe aggregate.

- Basic skill coverage: `scripts/nl_tests/cases/nl_cases_minimal_basic_skill_coverage_20260621.txt`
  covers the planner-facing local/basic skills in 15 cases. Use this after
  registry, resolver, verifier, or basic tool changes.
- Runtime parity smoke: `scripts/nl_tests/cases/nl_cases_codex_parity_runtime_smoke_20260623.txt`
  covers agent-loop runtime boundaries in 8 cases: observed execution,
  checkpoint/background surfaces, task lifecycle, hooks, subagents, and CLI
  resume affordances.
- Codex CLI continuous development smoke: `scripts/nl_tests/cases/nl_cases_codex_cli_continuous_dev_20260711.txt`
  covers a compact create -> extend -> verify -> inspect coding sequence with
  real local file edits and verification commands. It is included in the static
  compact coverage gate so coding-agent regressions are not treated as optional
  after CLI or loop changes.
- Chinese-provider adapter smoke: `scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt`
  covers MiniMax, MiMo, Qwen, and DeepSeek adapter boundaries, including
  OpenAI-compatible provider config, vendor patches, strict JSON behavior,
  large-context metadata, and MiniMax multimodal-understanding versus media
  generation-skill separation. It is metadata-gated by the compact coverage
  check and does not require live provider generation. After running
  `scripts/nl_tests/run_chinese_provider_smoke_matrix.sh`, validate the emitted
  `matrix_summary.json` with
  `python3 scripts/nl_tests/check_chinese_provider_smoke_summary.py <matrix_summary.json>`;
  this checks provider rows, readiness counters, live-scope counters, and
  secret-free credential metadata. The shared `check_secret_scan_contract.py`
  guard locks forbidden secret fields and secret-like values so catalog and
  smoke artifacts cannot silently drift. The default live scope is MiniMax
  because it is the current purchased provider; use `--live-providers all` only
  when all provider accounts are intentionally in scope. For parity-gate runs,
  the parent `run_agent_parity_gate.sh` passes `CHINESE_PROVIDER_ENV_FILE` or
  `../runtime_env_filled.sh` to both catalog validation and smoke preflight when
  present.
- Task execution async lifecycle: `scripts/nl_tests/cases/nl_cases_task_execution_async_lifecycle_20260626.txt`
  covers representative async start, local-process poll, cancel contract,
  timeout expiry, terminal projection, and media async dry-run handoff without
  live provider generation or publishing-channel side effects.
- Multimodal focused smoke: `scripts/nl_tests/cases/nl_cases_multimodal_focused_20260621.txt`
  covers image and audio planner selection in 4 optional cases. Treat live media
  generation as quota-gated; prefer dry-run media capability cases when provider
  quota is low.
- Media dry-run capability: `scripts/nl_tests/cases/nl_cases_media_dry_run_capability_20260623.txt`
  covers image generation, speech synthesis, video generation, and music
  generation with `dry_run=true`, expected `planned_outputs`, and no external
  provider generation side effects.
- Release-gate equivalent: `scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent.txt`
  is generated by `python3 scripts/nl_tests/build_release_gate_subset.py` from
  the safe aggregate metadata. It currently selects 353 rows from the 2,089-row
  source aggregate and covers all reported metadata categories in
  `nl_cases_client_like_release_gate_equivalent_coverage.json`.

Before running live compact NL, check the metadata coverage:

```bash
python3 scripts/nl_tests/check_compact_coverage.py --report
```

Recommended commands:

```bash
bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_minimal_basic_skill_coverage_20260621.txt \
  --prompt-reply-only --quality-guard

bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_codex_parity_runtime_smoke_20260623.txt \
  --prompt-reply-only --quality-guard

bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_codex_cli_continuous_dev_20260711.txt \
  --prompt-reply-only --quality-guard

bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt \
  --prompt-reply-only --quality-guard

bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_task_execution_async_lifecycle_20260626.txt \
  --prompt-reply-only --quality-guard

bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_media_dry_run_capability_20260623.txt \
  --prompt-reply-only --quality-guard

bash scripts/nl_tests/run_client_like_continuous_suite.sh \
  --skip-smoke \
  --case-file scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent.txt \
  --prompt-reply-only --quality-guard \
  --exclude-case-tag image --exclude-case-tag audio --exclude-case-tag voice \
  --exclude-case-tag x --exclude-case-tag twitter --exclude-case-tag tweet \
  --exclude-case-tag x_api --exclude-case-tag post_tweet \
  --exclude-case-tag publish_tweet
```

Run the 2,100-row safe aggregate only for major route/provider migrations,
physical deletion of old compat paths, or final release gates. Keep X/Twitter
posting cases dry-run unless a live publish test is explicitly approved.

Self-extension regressions:

- `bash scripts/nl_tests/run_suite.sh self_extension`
- `bash scripts/nl_tests/run_full_suite.sh --with-self-extension`
- `bash scripts/nl_tests/run_suite.sh full --with-self-extension`

Notes for `self_extension`:

- Stage 1 is local backend validation and does not depend on provider availability.
- Stage 2 verifies natural-language `ask -> self_extension` handoff.
- If the provider is unavailable, stage 2 is reported as `SKIP` instead of a product failure.

Sensitive-flow regressions:

- `bash scripts/nl_tests/run_suite.sh sensitive_flows`
- `bash scripts/regression_sensitive_nl_flows.sh --rounds 2`

Notes for `sensitive_flows`:

- Covers high-risk config mutation guard, crypto unbound hints, and self-extension NL trigger.
- Keeps source-controlled NL examples in `scripts/nl_tests/cases/nl_cases_sensitive_flows.txt`.
- Logs are written under `scripts/nl_suite_logs/sensitive_flows/<timestamp>/`.

Long-tail regressions:

- `bash scripts/nl_tests/run_suite.sh ops_closed_loop`
- `bash scripts/nl_tests/run_suite.sh ops_http_repair`
- `bash scripts/nl_tests/run_suite.sh long_tail_flows`
- `bash scripts/regression_long_tail_nl_flows.sh`
- `bash scripts/regression_ops_http_repair_nl_flows.sh`
- `bash scripts/regression_ops_closed_loop.sh`

Notes for `long_tail_flows`:

- Covers the new health-check OS-only summary behavior and the `ops_closed_loop` HTTP start-and-validate flow.
- Keeps source-controlled NL examples in `scripts/nl_tests/cases/nl_cases_long_tail_flows.txt`.
- Uses an isolated temp workspace plus a temporary local HTTP demo service, then cleans the process and workspace after the run.
- Logs are written under `scripts/nl_suite_logs/long_tail_flows/<timestamp>/`.
- `scripts/regression_ops_closed_loop.sh` is the complementary local backend suite for the same closed-loop stack; it does not depend on provider availability.
- Category `ops` now runs both `ops_closed_loop` and `long_tail_flows`.

Notes for `ops_http_repair`:

- This is the focused NL retry suite for the bilingual `ops_http_repair_then_validate_{zh,en}` cases.
- It keeps source-controlled prompts in `scripts/nl_tests/cases/nl_cases_ops_http_repair.txt`.
- It reuses the same isolated temp workspace and local HTTP repair demo flow as `long_tail_flows`, but skips unrelated health-check and start-and-validate cases.
- Logs are written under `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`.

## Core runners

- `run_suite.sh` is now the preferred user-facing tool script.
- `bash scripts/nl_tests/run_manual_test.sh`
- `bash scripts/nl_tests/run_compound_single_suite.sh`
- `bash scripts/nl_tests/run_task_updates_suite.sh`
- `bash scripts/nl_tests/run_multistep_mixed_suite.sh`
- `bash scripts/nl_tests/run_full_suite.sh`
- `bash scripts/nl_tests/run_multi_turn_suite.sh`
- `bash scripts/regression_self_extension_suite.sh`
- `bash scripts/regression_sensitive_nl_flows.sh`
- `bash scripts/regression_ops_http_repair_nl_flows.sh`
- `bash scripts/regression_ops_closed_loop.sh`
- `bash scripts/regression_long_tail_nl_flows.sh`
- `bash scripts/nl_tests/run_runtime_capability_boundary_regression.sh`

## Cases

- `scripts/nl_tests/cases/` stores all NL case files.
- Canonical files:
  - `nl_cases_manual.txt` — curated daily smoke set (see "Case file format" below)
  - `nl_cases_manual.legacy.txt` — pre-2026-04-17 60-line version, kept as backup
  - `nl_cases_singletons.txt` — consolidates the historical `nl_case_*_only.txt` singletons
  - `nl_cases_full.txt`
  - `nl_cases_trace.txt`
  - `nl_cases_text_match.txt`
  - `nl_cases_compound_single_language.txt`
  - `nl_cases_task_updates_single_language.txt`
  - `nl_cases_task_updates_four_turn.txt`
  - `nl_cases_multistep_mixed_language.txt`
  - `nl_cases_clarify.txt`
  - `nl_cases_clarify_hard.txt`
  - `nl_cases_context_chain.txt`
  - `nl_cases_dynamic_guard_manual.txt`
  - `nl_cases_dynamic_guard_clarify.txt`
  - `nl_cases_dynamic_guard_context.txt`
  - `nl_cases_sensitive_flows.txt`
  - `nl_cases_ops_http_repair.txt`
  - `nl_cases_long_tail_flows.txt`

### Case file format (2026-04-17 onward)

```
suite|name|tags|prompt|expect=<substring>
```

- 5th field (`expect=...`) is **optional** and asserts the final response text
  contains the literal substring AND status=succeeded. Missing/failed → marked
  `assertion=fail` in the summary.
- `tags` is comma-separated. `natural` / `cn` are informational and used by
  triage tooling.
- Lines starting with `#` are comments; blank lines are ignored.
- 4-field rows (`suite|name|tags|prompt`) remain backward compatible.
- Multi-turn case files use `case_name|turn1|turn2|...`; use
  `run_multi_turn_suite.sh --turn-count N` for custom turn counts such as
  `nl_cases_task_updates_four_turn.txt`.
- Additional test text files now also live here:
  - `regression_trace_ask_cases_real.txt`
  - `regression_trace_ask_cases_minimax_think.txt`
  - `regression_user_instruction_cases.txt`
  - `regression_generated_crypto_safe_cases.txt`
  - `regression_generated_mixed_cases.txt`

## Logs

- `scripts/nl_suite_logs/manual/<timestamp>/`
- `scripts/nl_suite_logs/full/<timestamp>/`
- `scripts/nl_suite_logs/trace/<timestamp>/`
- `scripts/nl_suite_logs/resume/<timestamp>/`
- `scripts/nl_suite_logs/self_extension/<timestamp>/`
- `scripts/nl_suite_logs/text_match/<timestamp>/`
- `scripts/nl_suite_logs/clarify/<timestamp>/`
- `scripts/nl_suite_logs/context_chain/<timestamp>/`
- `scripts/nl_suite_logs/ops_closed_loop/<timestamp>/`
- `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`
- `scripts/nl_suite_logs/sensitive_flows/<timestamp>/`
- `scripts/nl_suite_logs/long_tail_flows/<timestamp>/`
