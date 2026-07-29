## run_cmd - standalone base skill

Execute an explicit command selected by the agent loop; never translate natural
language into a command or accept `request_text`.

## Contract

- Ordinary execution requires `command`; bounds include `cwd`, `timeout_seconds`, `idle_timeout_seconds`, and `max_output_bytes`.
- For durable work use `async_start=true`. Ordinary calls use `system.run_command`
  and cannot carry a model-invented deadline. Only when the user explicitly sets
  a runtime deadline use `system.run_command_with_deadline` with their exact
  `timeout_seconds`; never infer or shorten it. An unrestricted administrator's
  explicit no-deadline request uses `system.run_command_without_deadline`.
  `poll_after_seconds` controls checks and `expires_in_seconds` controls retention.
  A completed poll window is not a process failure.
- Quiet async commands remain valid while alive; ordinary background silence does
  not use `idle_timeout_seconds` as a failure signal.
- Admin-only `disable_timeout=true` requires async start and conflicts with an
  explicit timeout. Never detach an unmanaged shell child.
- Use `system.terminal_start` for an interactive PTY or managed long-lived service; unless the user supplies them, omit idle/expiry limits. Reuse its `session_id`
  and cursor with poll/write/resize/signal/terminate; terminate it after its
  dependent validation or use completes.
- `system.preview_command_permission` and `system.preview_background_command` are no-execution machine previews.
- `inspect_cli_help` is limited to bounded help/version/path inspection.
- Non-zero execution returns structured `exit_code` and `exit_category`.
  Recover from machine fields, never stderr prose.
- Command approval, sandbox, timeout, cancellation, and async lifecycle remain
  runtime-owned.
- Reuse the observed `job_id`/`poll_ref`, process identity, and monotonic output
  cursors; poll/cancel that job instead of replacing it after an empty poll.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
