# RustClaw

RustClaw is a Rust-based local agent stack with:
- `clawd` (HTTP task daemon),
- `telegramd` (Telegram bridge + command surface),
- `skill-runner` (skill process host),
- and multiple built-in operational/media skills.

## Current Workspace Layout

- `crates/claw-core`: shared config/types/error models
- `crates/clawd`: main HTTP daemon, queue/worker, routing, scheduling
- `crates/telegramd`: Telegram bot runtime and command handlers
- `crates/skill-runner`: dispatches skill requests to child skill binaries
- `crates/skills/*`: built-in skills (system/http/git/files/db/docker/media/X/etc.)
- `configs/config.toml`: main runtime config
- `configs/command_intent/`: intent routing rules
- `configs/x.toml`: X OAuth/app config
- `migrations/`: SQLite schema initialization
- `prompts/`: prompt templates (including schedule/voice-related prompts)
- `systemd/`: service unit files

## Built-in Skills (Current)

- `x`
- `system_basic`
- `http_basic`
- `git_basic`
- `install_module`
- `process_basic`
- `package_manager`
- `archive_basic`
- `db_basic`
- `docker_basic`
- `fs_search`
- `rss_fetch`
- `image_vision`
- `image_generate`
- `image_edit`
- `audio_transcribe`
- `audio_synthesize`

## HTTP API (clawd)

Default listen address comes from `configs/config.toml` (typically `127.0.0.1:8787`).

- `GET /v1/health`  
  Returns daemon/queue/worker status, uptime/version, and Telegram process health hints.
- `POST /v1/tasks`  
  Submits `ask` or `run_skill` tasks.
- `GET /v1/tasks/{task_id}`  
  Polls task status/result.
- `POST /v1/tasks/cancel`  
  Cancels queued/running tasks for `(user_id, chat_id)`.

Example health check:

`curl http://127.0.0.1:8787/v1/health`

Example ask task:

`curl -X POST http://127.0.0.1:8787/v1/tasks -H "Content-Type: application/json" -d "{\"user_id\":1,\"chat_id\":1,\"kind\":\"ask\",\"payload\":{\"text\":\"hello\",\"agent_mode\":true}}"`

## Quick Start

1. Install Rust toolchain:
   - `rustup default stable`
2. Build all workspace binaries:
   - `./build-all.sh release`
3. Configure secrets and runtime options:
   - `./setup-config.sh`
4. Start both daemons:
   - Source mode: `./start-all.sh`
   - Binary mode: `./start-all-bin.sh release`
5. Check logs:
   - `./check-logs.sh -n 120`

## Telegram Commands (Current)

`telegramd` currently supports:
- `/start`, `/help`
- `/agent on|off`
- `/status`
- `/cancel`
- `/skills`
- `/run <skill> <args>`
- `/sendfile <path>`
- `/voicemode show|voice|text|both|reset` (admin)
- `/openclaw config show|vendors|set <vendor> <model>` (admin)

## Media Vendor Support Matrix

Current support in built-in media skills:

- `image_generate`
  - Native: `openai`, `google`
  - Optional compatible mode for `anthropic`, `grok` (default off)
- `image_edit`
  - Native: `openai`, `google`
  - Optional compatible mode for `anthropic`, `grok` (default off)
- `image_vision`
  - Native: `openai`, `google`, `anthropic`
- `audio_synthesize`
  - Native: `openai`, `google`
  - Optional compatible mode for `anthropic`, `grok` (default off)
- `audio_transcribe`
  - Native: `openai`, `google`
  - Optional compatible mode for `anthropic`, `grok` (default off)

Resolution priority for media vendor/model selection:

- vendor: request args `vendor` > skill section `default_vendor` > `llm.selected_vendor`
- model: request args `model` > skill section `default_model` > `llm.<vendor>.model`

Compatibility switches in `configs/config.toml` (all default `false`):

- `image_generation.allow_compat_adapters`
- `image_edit.allow_compat_adapters`
- `audio_synthesize.allow_compat_adapters`
- `audio_transcribe.allow_compat_adapters`

## Shell Scripts and What They Do

- `build-all.sh`  
  Builds all workspace binaries (`release` or `debug`), optionally runs `cargo clean`, verifies required binaries, and syncs `skills.skill_runner_path` in `configs/config.toml`.

- `setup-config.sh`  
  Interactive config bootstrap for Telegram token/admin, model vendor/model, selected provider API key, and key tool limits.

- `start-clawd.sh`  
  Starts `clawd` via `cargo run -p clawd`, with first-run interactive model/provider safeguards and config persistence.

- `start-telegramd.sh`  
  Starts `telegramd` via `cargo run -p telegramd` after preflight checks for duplicate polling workers and webhook/polling conflicts.

- `start-all.sh`  
  One-command daemon startup wrapper: **prefers prebuilt binaries first** (`target/<profile>/clawd` and `target/<profile>/telegramd`), and falls back to source mode (`start-clawd.sh` + `start-telegramd.sh`) only when binaries are missing. Supports provider/model overrides.

- `start-all-bin.sh`  
  One-command daemon startup wrapper (binary mode): starts prebuilt `target/<profile>/clawd` and `target/<profile>/telegramd`.

- `stop-rustclaw.sh`  
  Stops both daemons using PID files first, then process-pattern fallback; clears stale PID files.

- `check-logs.sh`  
  Prints recent `clawd`/`telegramd` logs, highlights error keywords, and optionally follows logs (`-f`).

- `simulate-telegramd.sh`  
  Simulates Telegram-side submit/poll behavior against `clawd` without calling Telegram API (supports `ask` and `run_skill`).

- `x-oauth-login.sh`  
  Local OAuth2 PKCE helper for X: opens browser login, captures callback code, exchanges tokens, and writes `configs/x.toml`.

- `rollback.sh`  
  Hard rollback helper (`git reset --hard` + `git clean -fd`) to restore a target commit-ish. **Dangerous: removes uncommitted and untracked changes.**

## Notes

- Runtime behavior is config-driven; review `configs/config.toml` before production deployment.
- For service deployment, use the files under `systemd/` as a base.
