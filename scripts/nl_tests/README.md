# NL Tests

All natural-language test scripts are centralized in this directory.

## Unified Tool

Primary entry point:

- `bash scripts/nl_tests/run_suite.sh --list`
- `bash scripts/nl_tests/run_suite.sh manual`
- `bash scripts/nl_tests/run_suite.sh manual trace clarify`
- `bash scripts/nl_tests/run_suite.sh self_extension`
- `bash scripts/nl_tests/run_suite.sh --category multi_turn`
- `bash scripts/nl_tests/run_suite.sh --category regression --category guard`
- `bash scripts/nl_tests/run_suite.sh all`
- `bash scripts/nl_tests/run_suite.sh clarify_context_prompt`

Built-in categories:

- `smoke`
- `single_turn`
- `multi_turn`
- `regression`
- `guard`
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

## Core runners

- `run_suite.sh` is now the preferred user-facing tool script.
- `bash scripts/nl_tests/run_manual_test.sh`
- `bash scripts/nl_tests/run_full_suite.sh`
- `bash scripts/nl_tests/run_multi_turn_suite.sh`
- `bash scripts/regression_self_extension_suite.sh`

## Cases

- `scripts/nl_tests/cases/` stores all NL case files.
- Canonical files:
  - `nl_cases_manual.txt`
  - `nl_cases_full.txt`
  - `nl_cases_trace.txt`
  - `nl_cases_text_match.txt`
  - `nl_cases_clarify.txt`
  - `nl_cases_clarify_hard.txt`
  - `nl_cases_context_chain.txt`
  - `nl_cases_dynamic_guard_manual.txt`
  - `nl_cases_dynamic_guard_clarify.txt`
  - `nl_cases_dynamic_guard_context.txt`
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
