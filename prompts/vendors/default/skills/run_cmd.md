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

\* At least one of `command` or `request_text` is required.

## Output
- stdout/stderr of the command (truncated if very long).
