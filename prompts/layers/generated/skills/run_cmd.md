## run_cmd — standalone base skill

Independent base skill for running shell commands. Use `{"type":"call_skill","skill":"run_cmd","args":{"command":"..."}}`. Do not use system_basic for running commands.

## Capability
- Executes one shell command in workspace context.
- Use for: pwd, ls, grep, cat, scripts, any single command.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `command` | no* | string | - | Full shell command to run. |
| `request_text` | no* | string | - | Natural language request; when `command` is missing, skill asks LLM to generate one command. |
| `cwd` | no | string(path) | "." | Working directory. |
| `suggested_params` | no | object | - | Optional generic suggestion payload; `suggested_params.command` can be used as candidate command. |
| `suggest_once` | no | bool | true | Compatibility field; current behavior does not trigger a second LLM request in run_cmd. |
| `timeout_seconds` | no | integer | config default | Total wall-clock limit for this command. Use a bounded value for slow build/test/admin checks. |
| `idle_timeout_seconds` | no | integer | config default | Kill the command if stdout/stderr has no new output for this many seconds. |
| `max_output_bytes` | no | integer | config default | Stop and return truncated output after this many combined stdout/stderr bytes. |

\* At least one of `command` or `request_text` is required.

## Output
- stdout/stderr of the command, streamed and truncated with `...` if very long.
- Interactive or endless commands must be bounded, for example `top -b -n 1`, `timeout 5s top -b`, `tail -n 200 file`, or `journalctl -n 200 --no-pager`.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
