# NL Tests

All natural-language test scripts are centralized in this directory.

## Unified Tool

Primary entry point:

- `bash scripts/nl_tests/run_suite.sh --list`
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

Client-like continuous regression:

- Generate 100 deterministic contract-matrix seed cases without calling a model:
  `python3 scripts/nl_tests/generate_contract_matrix_cases.py --count 100 --check --report > /tmp/rustclaw-contract-cases.jsonl`
  Use `--batch N` to rotate the non-mandatory cases while preserving semantic/generic, phase, policy-decision, evidence-expression, and final-answer-shape coverage.
  Add `--history /tmp/rustclaw-contract-history.jsonl --update-history` when running repeated batches; the generator prefers case ids not already in the local history file and appends the selected ids after a successful check.
- Generate 100 live NL replay rows from the same matrix coverage:
  `python3 scripts/nl_tests/generate_contract_matrix_cases.py --count 100 --check --nl --report > /tmp/rustclaw-contract-nl.jsonl`
  Run them through the client-like path with:
  `bash scripts/nl_tests/run_client_like_continuous_suite.sh --skip-smoke --case-jsonl /tmp/rustclaw-contract-nl.jsonl --prompt-reply-only --quality-guard`
- Regenerate the safe aggregate case file:
  `python3 scripts/nl_tests/build_client_like_case_aggregate.py`
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
- Generate or check a lightweight offline regression baseline:
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --write-baseline /tmp/rustclaw-client-like-baseline.jsonl`
  `python3 scripts/nl_tests/evaluate_client_like_run.py scripts/nl_suite_logs/client_like_continuous/<run_id> --expectations scripts/nl_tests/expectations/<name>.jsonl`
  Expectation rows can assert route, planner capability/tool targets, executed tool/skill, verifier approval, finalizer stage/fallback/grounding, final text substrings, and final answer shape without making a new LLM request.
- Extract exact replay prompts and expectations from a finished or interrupted client-like run:
  `python3 scripts/nl_tests/extract_client_like_replay.py scripts/nl_suite_logs/client_like_continuous/<run_id> --case-jsonl /tmp/rustclaw-replay.jsonl --expectations /tmp/rustclaw-replay.expectations.jsonl`
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
