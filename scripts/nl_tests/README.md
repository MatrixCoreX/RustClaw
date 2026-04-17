# NL Tests

All natural-language test scripts are centralized in this directory.

## Unified Tool

Primary entry point:

- `bash scripts/nl_tests/run_suite.sh --list`
- `bash scripts/nl_tests/run_suite.sh manual`
- `bash scripts/nl_tests/run_suite.sh manual trace clarify`
- `bash scripts/nl_tests/run_suite.sh self_extension`
- `bash scripts/nl_tests/run_suite.sh sensitive_flows`
- `bash scripts/nl_tests/run_suite.sh ops_deterministic`
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

Self-extension regressions:

- `bash scripts/nl_tests/run_suite.sh self_extension`
- `bash scripts/nl_tests/run_full_suite.sh --with-self-extension`
- `bash scripts/nl_tests/run_suite.sh full --with-self-extension`

Notes for `self_extension`:

- Stage 1 is deterministic and does not depend on provider availability.
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

- `bash scripts/nl_tests/run_suite.sh ops_deterministic`
- `bash scripts/nl_tests/run_suite.sh ops_http_repair`
- `bash scripts/nl_tests/run_suite.sh long_tail_flows`
- `bash scripts/regression_long_tail_nl_flows.sh`
- `bash scripts/regression_ops_http_repair_nl_flows.sh`
- `bash scripts/regression_ops_closed_loop_deterministic.sh`

Notes for `long_tail_flows`:

- Covers the new health-check OS-only summary behavior and the `ops_closed_loop` HTTP start-and-validate flow.
- Keeps source-controlled NL examples in `scripts/nl_tests/cases/nl_cases_long_tail_flows.txt`.
- Uses an isolated temp workspace plus a temporary local HTTP demo service, then cleans the process and workspace after the run.
- Logs are written under `scripts/nl_suite_logs/long_tail_flows/<timestamp>/`.
- `scripts/regression_ops_closed_loop_deterministic.sh` is the complementary local deterministic suite for the same closed-loop stack; it does not depend on provider availability.
- Category `ops` now runs both `ops_deterministic` and `long_tail_flows`.

Notes for `ops_http_repair`:

- This is the focused NL retry suite for the bilingual `ops_http_repair_then_validate_{zh,en}` cases.
- It keeps source-controlled prompts in `scripts/nl_tests/cases/nl_cases_ops_http_repair.txt`.
- It reuses the same isolated temp workspace and local HTTP repair demo flow as `long_tail_flows`, but skips unrelated health-check and start-and-validate cases.
- Logs are written under `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`.

## Core runners

- `run_suite.sh` is now the preferred user-facing tool script.
- `bash scripts/nl_tests/run_manual_test.sh`
- `bash scripts/nl_tests/run_full_suite.sh`
- `bash scripts/nl_tests/run_multi_turn_suite.sh`
- `bash scripts/regression_self_extension_suite.sh`
- `bash scripts/regression_sensitive_nl_flows.sh`
- `bash scripts/regression_ops_http_repair_nl_flows.sh`
- `bash scripts/regression_ops_closed_loop_deterministic.sh`
- `bash scripts/regression_long_tail_nl_flows.sh`

## Cases

- `scripts/nl_tests/cases/` stores all NL case files.
- Canonical files:
  - `nl_cases_manual.txt` — curated daily smoke set (see "Case file format" below)
  - `nl_cases_manual.legacy.txt` — pre-2026-04-17 60-line version, kept as backup
  - `nl_cases_singletons.txt` — consolidates the historical `nl_case_*_only.txt` singletons
  - `nl_cases_full.txt`
  - `nl_cases_trace.txt`
  - `nl_cases_text_match.txt`
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
- `tags` is comma-separated. Special tags:
  - `chat_force` — submit as `kind=run_skill skill_name=chat`, bypassing the
    intent_router. **Required** to actually exercise the builtin chat skill,
    because intent_router will short-circuit most chat prompts via
    `direct_reply_candidate`.
  - `natural` / `cn` — informational, used by triage tooling.
- Lines starting with `#` are comments; blank lines are ignored.
- 4-field rows (`suite|name|tags|prompt`) remain backward compatible.
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
- `scripts/nl_suite_logs/ops_deterministic/<timestamp>/`
- `scripts/nl_suite_logs/ops_http_repair/<timestamp>/`
- `scripts/nl_suite_logs/sensitive_flows/<timestamp>/`
- `scripts/nl_suite_logs/long_tail_flows/<timestamp>/`
