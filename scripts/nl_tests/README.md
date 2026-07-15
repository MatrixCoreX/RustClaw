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
Chinese-provider model catalog guard, a dry-run Chinese-provider smoke matrix
with summary validation, the offline coding-loop repair fixture expectations,
and bounded rollout metrics for that fixture. When you pass one or more
finished client-like run directories, it also applies the same metrics gates to
the real NL run. The defaults require
`pass_rate=1.0`, `avg_llm_calls_per_turn<=4`, no prompt truncation, and no final
provider errors. Override with `--min-pass-rate`, `--max-avg-llm-calls`,
`--max-prompt-truncations`, `--max-provider-final-errors`, or environment
variables with the same uppercase names.

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
  secret-free credential metadata.
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
