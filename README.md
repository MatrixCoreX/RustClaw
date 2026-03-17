# RustClaw

<img src="./RustClaw.png" width="420" />

Chinese version: `README.zh-CN.md`

RustClaw is a local Rust-based agent runtime for everyday operations through Telegram, WhatsApp, Feishu/Lark, and a browser UI. It combines task routing, tool execution, memory, scheduling, multimodal skills, and `user_key` based identity into one deployable stack.

## What It Is For

RustClaw is designed to let you:

- chat with an agent from multiple channels
- run built-in skills such as file, HTTP, crypto, image, and service tasks
- manage users and permissions with `user_key`
- use a local UI for monitoring and daily administration
- keep conversation memory and resumable multi-step tasks

## Quick Start

### 1. Prerequisites

```bash
rustup default stable
python3 --version
```

Python `3.11+` is recommended.

### 2. Install the `rustclaw` command

```bash
# Standard install
bash install-rustclaw-cmd.sh

# Install without sudo
bash install-rustclaw-cmd.sh --user
```

Verify:

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

### 3. Configure your model and channels

Main config: `configs/config.toml`

Channel configs:

- `configs/channels/telegram.toml`
- `configs/channels/whatsapp.toml`
- `configs/channels/feishu.toml`

Common split configs:

- `configs/image.toml`
- `configs/audio.toml`
- `configs/crypto.toml`

### 4. Start RustClaw

```bash
# Full start with UI
rustclaw -start --vendor openai --model gpt-4.1 --profile release --channels all --with-ui --quick

# Another vendor example
rustclaw -start --vendor qwen --model qwen-max-latest --profile release --channels all --quick

# Simple mode
rustclaw -start release all
```

### 5. Daily operations

```bash
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
```

Legacy scripts are still available:

```bash
./start-all.sh
./stop-rustclaw.sh
```

## Keys, Identity, and Permissions

RustClaw uses `user_key` as the primary identity across UI and messaging channels.

- permissions are resolved by `user_key`
- channel conversations are resolved by `channel + external_chat_id`
- the UI uses the same auth model as Telegram and WhatsApp
- `clawd` can bootstrap the first admin key when the auth table is empty

Key management:

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
scripts/auth-key.sh list
```

## Local UI and API

The listen address is configured in `configs/config.toml`, usually `127.0.0.1:8787` or `0.0.0.0:8787`.

UI:

- URL: `http://127.0.0.1:8787/`
- static files: `UI/dist`
- override UI directory with `RUSTCLAW_UI_DIST`
- the browser stores a valid `user_key` locally and sends it with `X-RustClaw-Key`

Important API endpoints:

- `GET /v1/health`
- `POST /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `POST /v1/tasks/cancel`
- `GET /v1/auth/me`
- `POST /v1/auth/channel/bind`
- `GET/POST /v1/auth/crypto-credentials`

Examples:

```bash
curl http://127.0.0.1:8787/v1/health \
  -H "X-RustClaw-Key: rk-xxxx"

curl -X POST http://127.0.0.1:8787/v1/tasks \
  -H "Content-Type: application/json" \
  -H "X-RustClaw-Key: rk-xxxx" \
  -d '{"user_id":1,"chat_id":1,"user_key":"rk-xxxx","channel":"ui","external_user_id":"local-ui","external_chat_id":"local-ui","kind":"ask","payload":{"text":"hello","agent_mode":true}}'
```

## Common Telegram Commands

- `/start`
- `/help`
- `/agent on|off`
- `/status`
- `/cancel`
- `/skills`
- `/run <skill> <args>`
- `/sendfile <path>`
- `/voicemode show|voice|text|both|reset`
- `/openclaw config show|vendors|set <vendor> <model>`
- `/cryptoapi show`
- `/cryptoapi set binance <api_key> <api_secret>`
- `/cryptoapi set okx <api_key> <api_secret> <passphrase>`

## Crypto Credentials

Exchange credentials are stored per `user_key`, not as one shared global secret for all users.

- supported live exchanges: `binance`, `okx`
- risk controls are defined in `configs/crypto.toml`
- credentials are stored in `exchange_api_credentials`
- each `user_key` has its own exchange credential record

You can manage crypto credentials through:

- `GET/POST /v1/auth/crypto-credentials`
- Telegram `/cryptoapi ...` commands
- `scripts/import-crypto-credentials.sh` for legacy migration

## Built-in Skills

Common built-in skills include:

- `archive_basic`
- `audio_synthesize`
- `audio_transcribe`
- `chat`
- `config_guard`
- `crypto`
- `db_basic`
- `docker_basic`
- `fs_search`
- `git_basic`
- `health_check`
- `http_basic`
- `image_edit`
- `image_generate`
- `image_vision`
- `log_analyze`
- `package_manager`
- `process_basic`
- `rss_fetch`
- `service_control`
- `system_basic`
- `x`

## Important Files and Directories

- `configs/config.toml`: main runtime config
- `configs/channels/*.toml`: channel-specific config
- `configs/image.toml`: image skill config
- `configs/audio.toml`: audio skill config
- `configs/crypto.toml`: crypto risk and exchange config
- `configs/i18n/*.toml`: text resources
- `prompts/`: prompt templates
- `migrations/`: database migrations
- `UI/`: browser UI
- `pi_app/`: desktop mini app and small-screen monitor
- `systemd/`: service templates
- `crates/clawd`: API, routing, queue, memory, scheduling
- `crates/skills/*`: skill implementations

## Small-Screen Desktop App

The small-screen desktop app lives in `pi_app/`.

```bash
cd pi_app && ./run-small-screen.sh
cd pi_app && ./install-desktop.sh
cd pi_app && ./enable-autostart.sh
cd pi_app && ./open-small-screen.sh
```

It reads `GET /v1/health`, so `clawd` must already be running.

## Notes

- review and sanitize configs before production deployment
- if `/usr/local/bin` is not writable, use `bash install-rustclaw-cmd.sh --user`
- if `~/.local/bin` is not in `PATH`, add `export PATH="$HOME/.local/bin:$PATH"`
- for service deployment, use `systemd/` as the starting point

## License

This project uses a non-commercial source-available license.

- English legal text: `LICENSE`
- Chinese reference translation: `LICENSE.zh-CN.md`
