## run_cmd - standalone base skill

Execute an explicit command selected by the agent loop. This skill never
translates natural language into a command and must not receive
`request_text`.

## Contract

- Ordinary execution requires `command`; optional bounds include `cwd`,
  `timeout_seconds`, `idle_timeout_seconds`, and `max_output_bytes`.
- Use `async_start=true` plus bounded poll/expiry hints for durable
  non-interactive work. Never detach an unmanaged shell child.
- Use `system.terminal_start` only for a real PTY need, then reuse the observed
  `session_id` and cursor with poll/write/resize/signal/terminate.
- `system.preview_command_permission` and
  `system.preview_background_command` are no-execution machine previews.
- `inspect_cli_help` is limited to bounded help/version/path inspection.
- Non-zero execution returns structured `exit_code` and `exit_category`.
  Recover from machine fields, never stderr prose.
- Command approval, sandbox, timeout, cancellation, and async lifecycle remain
  runtime-owned.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
