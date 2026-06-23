# music_generate Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the music_generate implementation.

## Capability Summary
- `music_generate` creates provider-backed music/audio files from a musical prompt, lyrics, or instrumental request.
- The first live adapter is MiniMax-compatible; other provider slots are available for dry-run, planning, compatible gateways, and future native adapters.
- It supports `music-2.6`-style prompt, lyrics, instrumental, and cover-model fields through structured input.
- It saves the generated audio file to disk and returns machine-readable output metadata in `extra`.
- It supports `dry_run` so planner and runner paths can be tested without consuming music quota.

## Config Entry Points
- Main music config: `configs/music.toml` -> `[music_generation]`.
- Shared provider fallback: `configs/config.toml` -> `[llm.<vendor>]`.
- Current repo default: `default_vendor = "minimax"`, `default_model = "music-2.6"`.
- Optional dedicated key: `MUSIC_GENERATION_<VENDOR>_API_KEY`; otherwise use the shared provider key path.
- Non-MiniMax live calls require either a future native adapter or a provider block with `adapter_kind = "minimax_compatible"` for an endpoint that truly follows the MiniMax-compatible contract.

## Actions
- `generate`: generate a song or instrumental audio file.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| generate | `prompt` / `description` | conditional | string | - | Music style, mood, and scenario. Required for instrumental or lyrics-optimizer requests. |
| generate | `lyrics` | conditional | string | - | Song lyrics. Required unless `lyrics_optimizer=true` or `is_instrumental=true`. |
| generate | `lyrics_optimizer` | no | boolean | auto | When true and lyrics are empty, the selected adapter may generate lyrics from `prompt`. |
| generate | `is_instrumental` | no | boolean | `false` | Generate instrumental music without vocals. |
| generate | `format` / `response_format` | no | string | config default | `mp3`, `wav`, or `flac`. |
| generate | `output_path` | no | string(path) | auto | Workspace output path for generated audio. |
| generate | `audio_url` / `audio_base64` / `cover_feature_id` | no | string | - | Cover-generation inputs when using cover models. |
| generate | `dry_run` | no | boolean | `false` | Build request metadata without calling the provider. |
| generate | `vendor` | no | string | config default | Provider key such as `minimax`, `mimo`, `custom`, or another configured vendor. |
| generate | `model` | no | string | config default | Music generation model for the selected provider. |

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name.
- `model`: resolved model name.
- `model_kind`: adapter/runtime mode, such as `minimax_native` or `unsupported` in dry-run metadata.
- `output_path`: saved audio path.
- `outputs`: machine-readable output summary, currently `[{\"type\":\"audio_file\",\"path\":\"...\"}]`.
- `planned_outputs`: planned file outputs for dry-run validation responses.
- `audio_format`: normalized output format.
- `trace_id` and `extra_info`: provider metadata when returned.
- `dry_run`: present and true only for dry runs.

## Error Contract
- Missing required `prompt`/`lyrics` combination.
- Unsupported vendor, invalid output path, or path outside workspace.
- Missing API key for live generation.
- Provider generation/download/write failures.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"prompt":"Soulful blues, rainy night, slow tempo","lyrics":"[Verse]\nRain on the window\n[Chorus]\nMidnight keeps singing","format":"mp3","output_path":"music/demo.mp3"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"MUSIC_FILE:music/demo.mp3","extra":{"provider":"minimax","model":"music-2.6","model_kind":"minimax_native","output_path":"music/demo.mp3","outputs":[{"type":"audio_file","path":"music/demo.mp3"}],"audio_format":"mp3"},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"prompt":"Warm ambient piano loop","is_instrumental":true,"dry_run":true}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"MUSIC_GENERATE_DRY_RUN","extra":{"provider":"minimax","model":"music-2.6","model_kind":"minimax_native","dry_run":true,"request":{"model":"music-2.6","prompt":"Warm ambient piano loop","is_instrumental":true},"outputs":[]},"error_text":null}
