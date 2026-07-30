# Agent Runtime Skill Setup Guide

This document is for Agent Runtime self-questions such as “how do I enable this skill”, “what does this skill need”, “where should I bind the API key”, or “what config should be changed”. The goal is not fixed wording. The goal is to ground answers in the real runtime entry points that exist in this repository.

## Answering Pattern

1. Start with the real entry point. Say whether the setup lives in a config file, an environment variable, a local database/API, a local dependency, or a login/session state.
2. State the scope clearly. Distinguish machine-wide config, repo config, per-`user_key` binding, and per-session/channel login state.
3. If the user is only missing one thing, ask for it. Do not stop at abstract documentation when the next step is obvious. For ordinary parameters or paths, ask directly. For secrets, tokens, passwords, or API keys, prefer a dedicated command, UI, local config, or API path instead of asking the user to paste the sensitive value into ordinary chat.
4. Do not default to “edit this manually” when Agent Runtime can already inspect or change the workspace through existing skills.
5. Do not confuse config policy files with credential storage. Most importantly for `crypto`: exchange credentials are normally stored per `user_key` in the local database, not by editing `configs/crypto.toml`.
6. Editing files under `configs/` is an admin-only capability. If the current task is not authenticated as admin, reply with a no-permission answer and do not attempt the change.

## Skill Matrix

| Skill | Real setup entry point | What to emphasize and what to ask next |
| --- | --- | --- |
| `run_cmd` `read_file` `write_file` `list_dir` `make_dir` `remove_file` | No extra binding; depends on the current workspace and local permissions | These are base local capabilities. No separate enablement is required. |
| `schedule` | No third-party binding; depends on Agent Runtime scheduling | Ask what task should run and when it should trigger. |
| `system_basic` `process_basic` `health_check` `log_analyze` `service_control` `task_control` `config_guard` | No third-party binding; local runtime and service state only | Usually zero-config. Failures are more likely to be permissions, missing targets, or missing services. |
| `archive_basic` `fs_search` `git_basic` `package_manager` `install_module` `docker_basic` `db_basic` `http_basic` `doc_parse` `transform` | Mostly local commands, files, and network; `git_basic` reads `configs/git_basic.toml`; `db_basic` defaults to `data/agent-runtime.db` | These are not account-binding skills. Ask for the concrete path, command, DB, or target if it is missing. |
| `rss_fetch` | Reads `configs/rss.toml` for categories, feed sources, and failure tracking | This is feed-config driven, not account-bound. Ask which category or feed URL should be added. |
| `browser_web` | Requires `crates/skills/browser_web/browser_web.js`, Node.js, and Playwright; wait tuning lives in `configs/browser_web_wait_map.json` | Explain that this is a local browser dependency path. Ask whether to check/install Node.js and Playwright or adjust the wait map. |
| `web_search_extract` | Backend comes from `args.backend` / `WEB_SEARCH_BACKEND`; `serpapi` needs `SERPAPI_API_KEY`; DuckDuckGo HTML can work without a key | Explain that this is a search-backend setup question, not mainly a repo config toggle. Ask whether to keep the DuckDuckGo fallback or add `SERPAPI_API_KEY`. |
| `image_generate` `image_edit` `image_vision` | Mainly `configs/image.toml`; can inherit provider config from `configs/config.toml` `[llm.*]`; also supports env overrides such as `OPENAI_API_KEY`, `QWEN_API_KEY`, `MINIMAX_API_KEY`, plus `IMAGE_GENERATION_*` / `IMAGE_EDIT_*` / `IMAGE_VISION_*` | State the provider/model path first, then ask which provider should be configured and whether to write the key into config or environment. |
| `audio_transcribe` `audio_synthesize` | Mainly `configs/audio.toml`; can inherit `[llm.*]` from `configs/config.toml`; also supports `AUDIO_TRANSCRIBE_*` / `AUDIO_SYNTHESIZE_*` env overrides. `audio_transcribe` can also use local whisper.cpp through `[audio_transcribe.providers.custom]` with `base_url = "http://127.0.0.1:8178/v1"` and empty key for loopback only. | Explain whether the user wants cloud STT/TTS provider keys or local STT. For local whisper.cpp, emphasize the local `whisper-server` dependency, dedicated port `8178`, and multilingual models for Chinese audio. |
| `crypto` | This is an on-demand Skill Store skill. Trading policy and allow-lists live in `configs/crypto.toml`; exchange credentials live in crypto-owned storage under `database.skill_data_root`; bind them through the controlled `POST /v1/auth/crypto-credentials` management API | `configs/crypto.toml` controls policy and limits, while Binance/OKX credentials are stored per `user_key` outside the main runtime database. The credential API may add or overwrite only the current key's record. Telegram must not expose installation, enablement, or credential settings for this optional skill, and users must not send secrets through normal chat. |
| `stock` | Main config is `configs/stock.toml`; stock-name aliases live in `[stock.aliases]`; typo correction can use configured `[llm.*]` providers from `configs/config.toml` | This is not an account-binding skill. Ask whether the user wants to use a stock code directly or add an alias mapping. |
| `weather` | Main config is `configs/weather.toml`; mostly language/i18n | Usually zero-config. If the user asks how to enable it, explain that it is normally ready by default. |
| `map_merchant` | Main config is `configs/map_merchant.toml`; Amap can use `AMAP_API_KEY`; Google can use `GOOGLE_MAPS_API_KEY` / `GOOGLE_PLACES_API_KEY` or the config file | Explain the active provider first, then ask which provider key should be configured now. |
| `kb` | No third-party account binding; namespace snapshots and retrieval rows live in the KB-owned database under `database.skill_data_root` | This is an ingest-path question, not an API-binding question. Ask for the namespace and which files/directories should be ingested. Do not direct users to inspect or edit the private database. |
| `x` | Main config is `configs/x.toml`; env overrides include `X_USE_XURL`, `XURL_BIN`, `XURL_APP`, `XURL_AUTH`, `XURL_USERNAME`; actual posting depends on local `xurl` OAuth state | Explain that this is a local login/session skill, not just a static API key field. Ask whether to inspect `xurl` and current auth state or wire the config values now. |
| `photo_organize` | Main config is `configs/photo_organize.toml`; no third-party account; the real required input is an accessible `source_dir` | Ask for the absolute photo directory and whether the user wants preview, copy, or move mode. |
| `extension_manager` | Depends on `OPENAI_API_KEY`; writes under `external_skills/`, `configs/skills_registry.toml`, and `configs/config.toml` | Explain that this is a developer-facing skill for scaffolding/registering external skills. Ask what capability should be added, what the skill name should be, and whether to scaffold first. |

## Example Answer Shape

“`crypto` is installed on demand. `configs/crypto.toml` controls trading policy and limits, while Binance / OKX credentials for the current key live in crypto-owned storage. Add or replace them through the controlled `POST /v1/auth/crypto-credentials` management API.”

“Modifying an exchange API key updates the current key's credential record for that exchange through the controlled management API; it does not edit a shared global configuration file.”

“If the user asks to modify an exchange API key without naming the exchange, first inspect `crypto.execution_mode` / `crypto.default_exchange`. Ask which exchange only when no default exists, and never ask the user to provide a secret in chat.”
