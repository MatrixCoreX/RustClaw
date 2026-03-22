# NL Suite Naming

Unified natural-language regression entry points:

- `bash scripts/run_nl_suite_manual.sh`
- `bash scripts/run_nl_suite_full.sh`
- `bash scripts/run_nl_suite_trace.sh`
- `bash scripts/run_nl_suite_resume.sh`
- `bash scripts/run_nl_suite_text_match.sh`
- `bash scripts/run_nl_suite_clarify.sh`
- `bash scripts/run_nl_suite_all.sh`

Unified case files:

- `scripts/nl_cases_manual.txt`
- `scripts/nl_cases_full.txt`
- `scripts/nl_cases_trace.txt`
- `scripts/nl_cases_text_match.txt`
- `scripts/nl_cases_clarify.txt`

Unified log layout:

- `scripts/nl_suite_logs/manual/<timestamp>/`
- `scripts/nl_suite_logs/full/<timestamp>/`
- `scripts/nl_suite_logs/trace/<timestamp>/`
- `scripts/nl_suite_logs/resume/<timestamp>/`
- `scripts/nl_suite_logs/text_match/<timestamp>/`
- `scripts/nl_suite_logs/clarify/<timestamp>/`
- `scripts/nl_suite_logs/all/<timestamp>/`

Compatibility:

- Old script names are still kept and can still be called directly.
- The new `run_nl_suite_*` and `nl_cases_*` names are the recommended stable entry points moving forward.
