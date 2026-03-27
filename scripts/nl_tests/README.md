# NL Tests

All natural-language test scripts are centralized in this directory.

## Entry points

- `bash scripts/nl_tests/run_suite.sh manual`
- `bash scripts/nl_tests/run_suite.sh text_match`
- `bash scripts/nl_tests/run_suite.sh full`
- `bash scripts/nl_tests/run_suite.sh trace`
- `bash scripts/nl_tests/run_suite.sh resume`
- `bash scripts/nl_tests/run_suite.sh clarify`
- `bash scripts/nl_tests/run_suite.sh context_chain`
- `bash scripts/nl_tests/run_suite.sh all`
- `bash scripts/nl_tests/run_suite.sh clarify_context_prompt`

## Core runners

- `bash scripts/nl_tests/run_manual_test.sh`
- `bash scripts/nl_tests/run_full_suite.sh`
- `bash scripts/nl_tests/run_multi_turn_suite.sh`

## Cases

- `scripts/nl_tests/cases/` stores all NL case files.
- Canonical files:
  - `nl_cases_manual.txt`
  - `nl_cases_full.txt`
  - `nl_cases_trace.txt`
  - `nl_cases_text_match.txt`
  - `nl_cases_clarify.txt`
  - `nl_cases_context_chain_20260326.txt`
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
- `scripts/nl_suite_logs/text_match/<timestamp>/`
- `scripts/nl_suite_logs/clarify/<timestamp>/`
- `scripts/nl_suite_logs/context_chain/<timestamp>/`
