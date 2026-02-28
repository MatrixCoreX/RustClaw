# RustClaw

RustClaw is a lightweight Rust daemon suite for Raspberry Pi-first deployments.

## Workspace

- `crates/claw-core`: shared types, config, errors
- `crates/clawd`: core HTTP daemon
- `crates/telegramd`: telegram channel daemon (M1 placeholder loop)
- `crates/skill-runner`: skill execution protocol runner
- `crates/skills/*`: built-in operational skills
- `configs/config.toml`: default 1g-friendly config
- `migrations/`: sqlite schema init
- `systemd/`: service files

## Quick Start

1. Install Rust toolchain:
   - `rustup default stable`
2. Build all crates:
   - `cargo build --workspace`
3. Run clawd:
   - `cargo run -p clawd`
4. Run telegramd in another shell:
   - `cargo run -p telegramd`

## HTTP Smoke Test

Check health:

`curl http://127.0.0.1:8787/v1/health`

Health response now also includes telegram daemon check fields:
- `telegramd_healthy` (optional)
- `telegramd_process_count` (optional)

Submit task:

`curl -X POST http://127.0.0.1:8787/v1/tasks -H "Content-Type: application/json" -d "{\"user_id\":1,\"chat_id\":1,\"kind\":\"ask\",\"payload\":{\"text\":\"hello\"}}"`

## Notes

- This generation is M1/M2 scaffold level.
- Telegram bot command handling and LLM provider routing are not wired yet.
- Database migration execution should be added in next step.

## Image Skills

RustClaw now includes three image skills:

- `image_vision`: image understanding (`describe`, `extract`, `compare`, `screenshot_summary`)
- `image_generate`: text-to-image (`prompt -> image file`)
- `image_edit`: image editing (`edit`, `outpaint`, `restyle`, `add_remove`)

All three follow the same `run_skill` protocol and can be called via `call_skill`.

Minimal examples:

- Vision:
  `{"type":"call_skill","skill":"image_vision","args":{"action":"describe","images":[{"path":"image/demo.png"}]}}`
- Generate:
  `{"type":"call_skill","skill":"image_generate","args":{"prompt":"A cyberpunk cat","size":"1024x1024"}}`
- Edit:
  `{"type":"call_skill","skill":"image_edit","args":{"action":"restyle","image":{"path":"image/in.png"},"instruction":"turn it into watercolor style"}}`

## X Skill

`x` skill posts content to X API v2 (`/tweets`) through `skill-runner`.
Default mode uses official `xurl` CLI for user-context auth.

Standalone config file:
- `configs/x.toml`
- Optional override path: `X_CONFIG_PATH=/custom/path/x.toml`

Args:
- Minimal: `{"text":"hello from RustClaw"}`
- Dry run: `{"text":"hello from RustClaw","dry_run":true}`
- Publish: `{"text":"hello from RustClaw","send":true}`

Environment variables:
- `XURL_BIN` (optional, default `xurl`)
- `XURL_APP` (optional, xurl app profile name)
- `XURL_AUTH` (optional, e.g. `oauth2`)
- `XURL_USERNAME` (optional, select OAuth2 user in xurl)
- `XURL_TIMEOUT_SECONDS` (optional, default `30`)
- `X_REQUIRE_EXPLICIT_SEND` (optional, default `true`; when true, missing `send=true` will only preview)
- `X_MAX_TEXT_CHARS` (optional, default `280`)

Safety default:
- Without explicit `send=true`, the skill returns preview text and does not publish.

Quick xurl setup:
1. Install xurl
2. `xurl auth apps add my-app --client-id <id> --client-secret <secret>`
3. `xurl auth oauth2`
4. (optional) set default app/user by `xurl auth default`
