## run_cmd — standalone base skill

Independent base skill for running shell commands. Use `{"type":"call_skill","skill":"run_cmd","args":{"command":"..."}}`. Do not use system_basic for running commands.

## Capability
- Executes one shell command in workspace context.
- Use for: pwd, ls, grep, cat, scripts, any single command.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `command` | yes | string | - | Full shell command to run. |
| `cwd` | no | string(path) | "." | Working directory. |

## Output
- stdout/stderr of the command (truncated if very long).
