## run_cmd — standalone base skill

Independent base skill for running shell commands. Use `{"type":"call_skill","skill":"run_cmd","args":{"command":"..."}}`. Do not use system_basic for running commands.

## Capability
- Executes one shell command in workspace context.
- Use for: pwd, ls, grep, cat, scripts, any single command.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `action` | no | string | - | Machine action token. Use `inspect_cli_help` only for bounded read-only CLI help/version/path probes. Leave unset for ordinary shell execution. |
| `command` | no* | string | - | Full shell command to run. |
| `request_text` | no* | string | - | Natural language request; when `command` is missing, skill asks LLM to generate one command. |
| `cwd` | no | string(path) | "." | Working directory. |
| `suggested_params` | no | object | - | Optional generic suggestion payload; `suggested_params.command` can be used as candidate command. |
| `suggest_once` | no | bool | true | Compatibility field; current behavior does not trigger a second LLM request in run_cmd. |
| `timeout_seconds` | no | integer | config default | Total wall-clock limit for this command. Use a bounded value for slow build/test/admin checks. |
| `idle_timeout_seconds` | no | integer | config default | Kill the command if stdout/stderr has no new output for this many seconds. |
| `max_output_bytes` | no | integer | config default | Stop and return truncated output after this many combined stdout/stderr bytes. |
| `async_start` | no | bool | false | Start a long-running/background command through the runtime async job contract instead of blocking this task. |
| `poll_after_seconds` | no | integer | runtime default | Suggested delay before polling a runtime async job. Use with `async_start=true`. |
| `expires_in_seconds` | no | integer | runtime default | Runtime async job lease/expiry horizon. Use with `async_start=true`. |

\* At least one of `command` or `request_text` is required.

## Output
- stdout/stderr of the command, streamed and truncated with `...` if very long.
- Interactive or endless commands must be bounded, for example `top -b -n 1`, `timeout 5s top -b`, `tail -n 200 file`, or `journalctl -n 200 --no-pager`.
- For a long-running/background operation that should be resumed or polled by RustClaw, set `async_start=true` and provide bounded `poll_after_seconds` / `expires_in_seconds` when useful. Do not synthesize `checkpoint_id`, `poll_ref`, `next_check_after`, or `status=background` inside shell output; those fields belong to the runtime async contract.
- Non-zero exits are structured errors. `extra.exit_code` is always included when available; `extra.exit_category` is derived from the exit code (`command_not_found` for 127, `command_not_executable` for 126, `command_reported_failure` for 1-125, `terminated_by_signal_or_shell_status` for 128-255), with `extra.exit_classification_source="exit_code"`.
- Use `extra.exit_category` and `extra.exit_code` for recovery or summaries instead of matching stderr text.
- For current CLI surface checks that only inspect help/version/path availability, set `action="inspect_cli_help"` and include bounded `timeout_seconds` / `max_output_bytes`. Do not use this action for scripts, installers, mutation commands, network calls, or arbitrary shell execution.
- For CLI subcommand/interface questions, inspect the most specific safe help surface first. If the requested target is a nested command, prefer `<cli> <subcommand> --help` over only `<cli> --help`; use the top-level help only when the request asks about the overall CLI or when the subcommand name is unknown.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
