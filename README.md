# RustClaw

<img src="./RustClaw.png" width="420" />

RustClaw is a Rust-based local agent runtime stack. It is powered by `clawd` (task gateway and execution orchestration), uses Telegram / WhatsApp adapters for multi-channel messaging, and supports skills, scheduling, memory, and multimodal capabilities.

## Recent Changes (vs older versions)

- Added many new skill modules (ops, logs, config guard, service control, crypto trading, multimedia, and more).
- Introduced dual WhatsApp adapters (Cloud API + Web Bridge) with a unified channel-routing design.
- `clawd` now supports serving a local monitor UI on the same port (`UI/dist`).
- Some legacy scripts were removed or replaced and are no longer maintained:
  - `rollback.sh`
  - `setup-config.sh`
  - `script.py`
- Use the current startup and packaging scripts as the standard entry points (see "Script Reference" below).

## Core Architecture

- `crates/clawd`: HTTP API, task queue, routing, scheduling, memory, execution adapters.
- `crates/claw-core`: shared config, types, and error models.
- `crates/skill-runner`: skill process host that invokes skill binaries.
- Messaging adapters:
  - `crates/telegramd`
  - `crates/whatsappd` (Cloud API)
  - `crates/whatsapp_webd` + `services/wa-web-bridge` (WhatsApp Web)
- Skill implementations: `crates/skills/*`
- Configs: `configs/`
- Data and migrations: `data/`, `migrations/`

## Current Skill List (workspace)

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
- `health_check`
- `log_analyze`
- `service_control`
- `config_guard`
- `crypto`

## API and Local UI

The default listen address is configured in `configs/config.toml` (typically `127.0.0.1:8787`).

- `GET /v1/health`: service health, queue and process status
- `POST /v1/tasks`: submit tasks (`ask` / `run_skill`)
- `GET /v1/tasks/{task_id}`: query task result
- `POST /v1/tasks/cancel`: cancel tasks by conversation scope

Examples:

```bash
curl http://127.0.0.1:8787/v1/health
curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -d '{"user_id":1,"chat_id":1,"kind":"ask","payload":{"text":"hello","agent_mode":true}}'
```

Local monitor UI:

- URL: `http://127.0.0.1:8787/`
- Default static directory: `UI/dist`
- Override with environment variable: `RUSTCLAW_UI_DIST`

## Quick Start

1) Install Rust toolchain

```bash
rustup default stable
```

2) Build

```bash
./build-all.sh release
```

3) Start core services (recommended)

```bash
./start-all.sh
```

4) Binary-only startup (optional)

```bash
./start-all-bin.sh release
```

5) Start adapters as needed

```bash
./start-telegramd.sh
./start-whatsappd.sh
./start-whatsapp-webd.sh
./start-wa-web-bridge.sh
```

6) Check logs

```bash
./check-logs.sh -n 120
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

## Multimedia Vendor Support (Overview)

- `image_generate`: native `openai`, `google`; optional compat mode for `anthropic`, `grok`
- `image_edit`: native `openai`, `google`; optional compat mode for `anthropic`, `grok`
- `image_vision`: native `openai`, `google`, `anthropic`
- `audio_synthesize`: native `openai`, `google`; optional compat mode for `anthropic`, `grok`
- `audio_transcribe`: native `openai`, `google`; optional compat mode for `anthropic`, `grok`

Compatibility switches are in `configs/config.toml` and default to `false`:

- `image_generation.allow_compat_adapters`
- `image_edit.allow_compat_adapters`
- `audio_synthesize.allow_compat_adapters`
- `audio_transcribe.allow_compat_adapters`

## Crypto Skill (Market + Insight + Trade Guard)

`crypto` supports:

- Market: `quote`, `multi_quote`, `candles`, `indicator`
- Insight: `onchain` (news is provided by `rss_fetch`)
- Trading: `trade_preview`, `trade_submit`, `order_status`, `cancel_order`, `positions`

Default safety behavior:

- When `crypto.require_explicit_send=true`, `trade_submit` must include `confirm=true`.
- Risk limits are configurable: `max_notional_usd`, `allowed_symbols`, `allowed_exchanges`, `blocked_actions`.
- Default execution mode is `cextest` (backward-compatible alias: `paper`, writes to `data/crypto-paper-orders.jsonl`).
- Dedicated config file: `configs/crypto.toml`.
- Live exchange support: `binance` and `okx` (must be enabled and configured in `configs/crypto.toml`).

## Script Reference (Current Recommended Entrypoints)

- `build-all.sh`: build workspace binaries (supports profile selection and verification)
- `start-all.sh`: one-click startup (prefers prebuilt binaries, falls back to source startup)
- `start-all-bin.sh`: start using prebuilt binaries only
- `start-clawd.sh`: start `clawd`
- `start-clawd-ui.sh`: build `UI/dist` and start `clawd`
- `start-telegramd.sh`: start `telegramd`
- `start-whatsappd.sh`: start `whatsappd`
- `start-whatsapp-webd.sh`: start `whatsapp_webd`
- `start-wa-web-bridge.sh`: start WhatsApp Web bridge
- `start-future-adapters.sh`: start placeholder processes for future adapters
- `stop-rustclaw.sh`: stop core daemons and clean PID files
- `check-logs.sh`: inspect/follow logs
- `simulate-telegramd.sh`: locally simulate Telegram submit/poll against `clawd`
- `package-release.sh`: build release package artifacts
- `copy_rustclaw_safe.sh`: safely copy project for deployment/distribution

## Directory Reference

- `configs/config.toml`: main runtime config
- `configs/channels/*.toml`: channel config files
- `configs/command_intent/*.toml`: intent-routing rules
- `configs/i18n/*.toml`: i18n text resources
- `prompts/`: prompt templates
- `migrations/`: database migrations
- `systemd/`: service deployment templates
- `USAGE.md`: team workflow and onboarding supplement

## Notes

- Review and sanitize configs before production deployment.
- For service deployment, use units under `systemd/` as templates first.
