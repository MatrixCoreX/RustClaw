# RustClaw

<img src="./RustClaw.png" width="420" />

Chinese version: `README.zh-CN.md`

RustClaw is a Rust-based local agent runtime stack. It is powered by `clawd` (task gateway and execution orchestration), uses Telegram / WhatsApp / UI adapters for multi-channel messaging, and supports skills, scheduling, memory, multimodal capabilities, and key-based identity.

## Core Architecture

- `crates/clawd`: HTTP API, task queue, routing, scheduling, memory, execution adapters.
- `crates/claw-core`: shared config, types, and error models.
- `crates/skill-runner`: skill process host that invokes skill binaries.
- Messaging adapters:
  - `crates/telegramd`
  - `crates/whatsappd` (Cloud API)
  - `crates/whatsapp_webd` + `services/wa-web-bridge` (WhatsApp Web)
  - `crates/feishud` (Feishu/Lark app bot; webhook / long_connection; config: `configs/channels/feishu.toml`)
- Skill implementations: `crates/skills/*`
- Configs: `configs/`
- Data and migrations: `data/`, `migrations/`

## Identity and Channel Binding

- The system uses `user_key` as the primary identity. Telegram / WhatsApp / UI are bound onto that identity instead of owning separate permanent user IDs.
- Conversation scope is separated from identity: execution context, memory, and task ownership are resolved by `channel + external_chat_id`, while permissions and credentials are resolved by `user_key`.
- UI is treated as a first-class channel (`ui`) and uses the same auth model as messaging channels.
- `clawd` can auto-bootstrap the first admin key when the auth table is empty.
- Local key management:
  - `rustclaw -key list`
  - `rustclaw -key generate admin`
  - `rustclaw -key generate user`
  - `scripts/auth-key.sh list`

## Multi-Step Task Behavior

- Agent `act` tasks are split into executable substeps before execution.
- When a task is split into multiple steps, RustClaw now sends the numbered execution plan to the user first, then starts running it.
- If execution stops mid-plan, the task may carry `resume_context`; follow-up messages such as "继续", "为什么失败了", or "不用了" are classified into resume / defer / abandon flows.
- Resume only continues unfinished steps; completed steps are not replayed.

## Skill Reference (Detailed)

Use via Telegram `/run <skill> <json-args>` or agent route-to-skill actions.

- `archive_basic`: archive workflows; compress/extract/list for backup or deployment bundles.
- `audio_synthesize`: text-to-speech generation; supports voice-style output for delivery.
- `audio_transcribe`: speech-to-text; converts audio files into readable text with optional structure.
- `config_guard`: safe config mutation; focuses on minimal edits, validation, and secret-safe outputs.
- `crypto`: market/insight/trade guard; supports `quote`, `multi_quote`, `candles`, `indicator`, `onchain`, `trade_preview`, `trade_submit`, `order_status`, `cancel_order`, `positions`.
- `chat`: lightweight conversational generation; used for joke/chitchat style replies and other simple text-only generation.
- `db_basic`: SQL/data operations; query and controlled data mutations for local databases.
- `docker_basic`: container operations; inspect/log/start/stop/restart/images/compose diagnostics.
- `fs_search`: filesystem discovery; recursive file search, path filtering, and quick location tasks.
- `git_basic`: repo operations; status/diff/branch/commit/pull/merge helper actions.
- `health_check`: runtime diagnostics; summarizes critical checks and recommends next actions.
- `http_basic`: HTTP/API probing; GET/POST style diagnostics and webhook/API call checks.
- `image_edit`: image modification; applies edit instructions to existing images.
- `image_generate`: text-to-image generation; creates images from prompt descriptions.
- `image_vision`: visual understanding; describe scenes, OCR extraction, compare differences.
- `install_module`: module install helper; adds runtime/dependency modules with ecosystem awareness.
- `log_analyze`: log diagnosis; extracts failures, evidence, likely causes, and next checks.
- `package_manager`: package lifecycle; install/update/remove/list per detected ecosystem.
- `process_basic`: process lifecycle; list/find/kill/restart processes with status feedback.
- `rss_fetch`: feed retrieval; latest/category/source-layered news extraction.
- `service_control`: managed service actions; status/start/stop/restart with post-check expectations.
- `system_basic`: OS introspection; system info/resource/network/basic command diagnostics.
- `x`: X/Twitter workflow; draft/rewrite and optional publish actions with safe confirmation.

## API and Local UI

The listen address is configured in `configs/config.toml` (for example `0.0.0.0:8787`). Most authenticated APIs require `X-RustClaw-Key`.

- `GET /v1/health`: service health, queue and process status
- `POST /v1/tasks`: submit tasks (`ask` / `run_skill`)
- `GET /v1/tasks/{task_id}`: query task result
- `POST /v1/tasks/cancel`: cancel tasks by conversation scope
- `POST /v1/auth/ui-key/verify`: verify a UI key
- `GET /v1/auth/me`: resolve current identity from key
- `POST /v1/auth/channel/resolve`: resolve channel binding
- `POST /v1/auth/channel/bind`: bind Telegram / WhatsApp / UI to a key
- `GET/POST /v1/auth/crypto-credentials`: read/write per-user exchange credentials

Examples:

```bash
curl http://127.0.0.1:8787/v1/health \
  -H "X-RustClaw-Key: rk-xxxx"

curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -d '{"user_id":1,"chat_id":1,"user_key":"rk-xxxx","channel":"ui","external_user_id":"local-ui","external_chat_id":"local-ui","kind":"ask","payload":{"text":"hello","agent_mode":true}}'
```

Local monitor UI:

- URL: `http://127.0.0.1:8787/`
- Default static directory: `UI/dist`
- Override with environment variable: `RUSTCLAW_UI_DIST`
- The UI requires a valid `user_key`; the browser stores it locally and sends it as `X-RustClaw-Key`.

Desktop mini app / small-screen monitor:

- Directory: `pi_app/`
- Python desktop app: `pi_app/run-small-screen.sh`
- Desktop shortcut install: `cd pi_app && ./install-desktop.sh`
- Enable autostart after login: `cd pi_app && ./enable-autostart.sh`
- Browser full-screen small page: `cd pi_app && ./open-small-screen.sh`
- The desktop mini app reads `GET /v1/health`, so `clawd` should already be running.
- On first start, the Python mini app auto-generates a local `user` key and stores it at `pi_app/.rustclaw_small_screen_key`.

## Quick Start (Recommended: `rustclaw` CLI)

1) Prerequisites

```bash
rustup default stable
python3 --version   # recommend 3.11+
```

2) Install the unified command

```bash
# Standard install (tries /usr/local/bin, auto-fallback supported)
bash install-rustclaw-cmd.sh

# No sudo environments (macOS/Linux/Raspberry Pi OS)
bash install-rustclaw-cmd.sh --user
```

Verify after install:

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

Key management:

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
```

3) Build and start

```bash
# Start with full start-all feature set
rustclaw -start --vendor openai --model gpt-4.1 --profile release --channels all --with-ui --quick
rustclaw -start --vendor qwen --model qwen-max-latest --profile release --channels all --quick
rustclaw -start --vendor custom --model custom-model --profile release --channels all --quick

# Simple start
rustclaw -start release all
```

4) Daily ops

```bash
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
```

5) Legacy script mode (still supported)

```bash
./start-all.sh
./stop-rustclaw.sh
```

## Common Telegram Commands

- `/start`, `/help`
- `/agent on|off`
- `/status`
- `/cancel`
- `/skills`
- `/run <skill> <args>`
- `/sendfile <path>`
- `/voicemode show|voice|text|both|reset` (admin)
- `/openclaw config show|vendors|set <vendor> <model>` (admin)

## Skill Behavior Notes

- Multimedia compatibility flags are now in split config files and default to `false`:
  - `image_generation.allow_compat_adapters`
  - `image_edit.allow_compat_adapters`
  - `audio_synthesize.allow_compat_adapters`
  - `audio_transcribe.allow_compat_adapters`
- Config split and precedence:
  - `configs/config.toml`: global/base settings
  - `configs/image.toml`: image skill settings (`image_edit` / `image_generation` / `image_vision`)
  - `configs/audio.toml`: audio skill settings (`audio_synthesize` / `audio_transcribe`)
  - Runtime precedence (same key): `config.toml` explicit value overrides split-file defaults.
- Native adapter model routing:
  - `native_models` in `configs/image.toml` / `configs/audio.toml` controls which Qwen models prefer native adapters in `auto` mode.
  - If a model is not listed in `native_models`, RustClaw prefers compat adapters when compat is allowed.
- Chat-skill prompt source:
  - `chat-skill` now loads its default system prompts from prompt files at runtime.
  - Default files:
    - `prompts/vendors/default/chat_skill_system_prompt.md`
    - `prompts/vendors/default/chat_skill_joke_system_prompt.md`
  - If a caller explicitly passes `system_prompt`, that inline prompt overrides the file-backed default.
- Image/audio model priority:
  - `request.model > default_model > <vendor>_models[0] > models[0] > llm.<vendor>.model`
  - Native/compat channel selection is independent from model priority and is driven by `native_models` plus `adapter_mode`.
- Crypto safety defaults:
  - Whether to require user confirmation before `trade_submit` is decided by the planner (no runtime guard).
  - Main risk fields: `max_notional_usd`, `allowed_symbols`, `allowed_exchanges`, `blocked_actions`.
  - Default execution exchange is `binance`; live exchanges include `binance` and `okx`.
  - Exchange API credentials are stored per `user_key` in `exchange_api_credentials`, not as shared global runtime secrets.
  - Use `GET/POST /v1/auth/crypto-credentials` or Telegram-side crypto credential flows to manage a specific user's exchange keys.

## Script Reference (Current Recommended Entrypoints)

- `rustclaw`: unified runtime command (`-start/-stop/-restart/-status/-logs/-health/-build/-h`)
- `install-rustclaw-cmd.sh`: install `rustclaw` command (cross-platform options: `--user`, `--dir`, `--force-build`)
- `cross-build-upload.sh`: build on the configured remote builder and upload artifacts back
- `build-all.sh`: build workspace binaries (supports profile selection and verification)
- `start-all.sh`: one-click startup (prefers prebuilt binaries, falls back to source startup)
- `start-all-bin.sh`: start using prebuilt binaries only
- `start-clawd.sh`: start `clawd`
- `start-clawd-ui.sh`: build `UI/dist` and start `clawd`
- `start-telegramd.sh`: start `telegramd`
- `start-whatsappd.sh`: start `whatsappd`
- `start-whatsapp-webd.sh`: start `whatsapp_webd`
- `start-feishud.sh`: start `feishud` (Feishu app bot; config `configs/channels/feishu.toml`; webhook / long_connection)
- `start-wa-web-bridge.sh`: start WhatsApp Web bridge
- `start-future-adapters.sh`: start placeholder processes for future adapters
- `stop-rustclaw.sh`: stop core daemons and clean PID files
- `check-logs.sh`: inspect/follow logs
- `simulate-telegramd.sh`: locally simulate Telegram submit/poll against `clawd`
- `package-release.sh`: build release package artifacts
- `copy_rustclaw_safe.sh`: safely copy project for deployment/distribution
- `scripts/auth-key.sh`: low-level auth key management helper
- `scripts/import-crypto-credentials.sh`: import exchange credentials from legacy config into per-user DB storage, with optional config scrubbing
- `pi_app/run-small-screen.sh`: launch the Python desktop mini app in foreground for debugging
- `pi_app/run-small-screen-launcher.sh`: desktop/autostart launcher that fills GUI env vars
- `pi_app/install-desktop.sh`: create `~/Desktop/RustClaw.desktop`
- `pi_app/enable-autostart.sh`: enable desktop mini app autostart
- `pi_app/disable-autostart.sh`: disable desktop mini app autostart
- `pi_app/open-small-screen.sh`: open the browser-based small-screen page in fullscreen

## Cross-Platform Notes

- Target platforms: Linux, Ubuntu, Debian/Raspberry Pi OS, macOS.
- If `/usr/local/bin` is not writable, use `bash install-rustclaw-cmd.sh --user`.
- If `~/.local/bin` is not in `PATH`, add:
  - `export PATH="$HOME/.local/bin:$PATH"`
- Startup scripts rely on Python TOML parsing (`tomllib`), so Python `3.11+` is recommended.

## Directory Reference

- `configs/config.toml`: main runtime config
- `configs/image.toml`: image skill config (default + vendor candidates)
- `configs/audio.toml`: audio skill config (default + vendor candidates)
- `configs/hard_rules/main_flow.toml`: main hard-rule / routing / summary / resume markers
- `configs/channels/*.toml`: channel config files
- `configs/command_intent/*.toml`: intent-routing rules
- `configs/i18n/*.toml`: i18n text resources
- `prompts/`: prompt templates
- `prompts/vendors/default/chat_skill_system_prompt.md`: default normal-chat system prompt for `chat-skill`
- `prompts/vendors/default/chat_skill_joke_system_prompt.md`: default joke-mode system prompt for `chat-skill`
- `migrations/`: database migrations
- `pi_app/`: desktop mini app / Raspberry Pi small-screen monitor
- `systemd/`: service deployment templates
- `USAGE.md`: team workflow and onboarding supplement

## Notes

- Review and sanitize configs before production deployment.
- For service deployment, use units under `systemd/` as templates first.

## License

This project uses a non-commercial source-available license:

- English legal text: `LICENSE`
- Chinese reference translation: `LICENSE.zh-CN.md`
