# audio_transcribe Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the audio_transcribe implementation.

## Capability Summary
- `audio_transcribe` previews or converts audio input through the configured STT provider. A configured remote provider is the preferred transcription route; local configuration is delegated to the media skill's private local ASR fallback.
- It supports local file path input or public audio URL input, plus optional hints and backend model/vendor selection.
- Successful responses include machine-readable `extra` metadata such as `provider`, `provider_location`, `recommended_capability`, `fallback_capability`, `model`, `model_kind`, `audio_path`, and `transcription_review`.

## Planner Selection Notes
- For ordinary audio/video transcription, always use `audio.preview_transcribe` before the actual STT call. It reads configuration without reading the source file or contacting a provider.
- If preview returns `provider_location=remote`, use `audio.transcribe`. If it returns `provider_location=local`, use `media_download.transcribe` instead so local recognition stays inside that skill's private environment.
- If the configured remote `audio.transcribe` call fails, continue with `media_download.transcribe` on the same local audio source. Do not report final failure until the local fallback also fails.
- For video input, first call `media_download.transcribe` with `extract_audio_only=true` and `deliver_to_user=false`, then preview/transcribe the returned WAV path.
- Successful remote and local results both declare `transcription_review`; the shared main-model finalizer corrects recognition errors and broken sentences, uses the user's response language, and applies the same inline/file delivery rule.
- Keep the user-provided source in a structured audio field. Do not infer paths or URLs from unrelated prose.

## Actions
- `preview_transcribe`: resolve the input, provider, provider location, model, adapter plan, recommended execution capability, and local fallback without reading the source file or calling a provider.
- `transcribe`: perform actual configured-provider transcription. Use it after preview selects the remote path. This remains the default when `action` is omitted for protocol compatibility.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| preview_transcribe | `audio.path`, `audio_path`, `path`, `file`, `audio.url`, `audio_url`, or `url` | yes | string | - | Source to validate. Missing local files are reported in structured evidence and do not fail preview. |
| preview_transcribe | `vendor`, `model` | no | string | impl default | Provider/model to resolve without credential access or a provider call. |
| transcribe | `audio.path` or `audio_path` or `path` | conditional | string(path) | - | Local audio file path (`audio.path` preferred). |
| transcribe | `audio.url` or `audio_url` | conditional | string(url) | - | Public audio URL. Some native adapters prefer or require URL input. |
| transcribe | `transcribe_hint` | no | string | - | Prompt/hint to improve recognition quality. |
| transcribe | `vendor` | no | string | impl default | Backend vendor selector. |
| transcribe | `model` | no | string | impl default | Backend model selector. |
| transcribe | `response_language` | no | string | task language | Explicit final transcript language. Omit unless the user requests a different language. |

Provide one audio source: local path or URL.

## Config Entry Points
- Main STT config: `configs/audio.toml` -> `[audio_transcribe]`.
- Default STT is the managed local whisper.cpp server; Qwen `qwen3-asr-flash` is available via `/chat/completions` `input_audio` with `QWEN_API_KEY` or `[llm.qwen].api_key`.
- `qwen_chat_models` selects this structured adapter; never infer it from user-language phrases.
- Local whisper.cpp uses the OpenAI-compatible custom provider:
  - set `default_vendor = "custom"`
  - set `adapter_mode = "compat"` and `allow_compat_adapters = true`
  - set `default_model = "local-whisper"` or another configured custom model name
  - enable `[audio_transcribe.providers.custom]` with `base_url = "http://127.0.0.1:8178/v1"`
- Loopback `custom` providers may omit `api_key`; remote providers require one.
- Chinese transcription is supported when the local whisper.cpp server runs a multilingual Whisper model, not an English-only `.en` model.
- For multilingual agents, start whisper.cpp with `--language auto`; the server default may otherwise bias recognition toward English.

## Success `extra` (`status=ok`)
- Preview responses include `action=preview_transcribe`, `status=dry_run`, `dry_run=true`, `provider_call=false`, `filesystem_write=false`, `input_path`, `resolved_input_path`, and `input_exists`.
- `provider`: resolved backend provider name
- `provider_location`: `remote` or `local`, determined from the configured provider endpoint rather than the user request text.
- `recommended_capability`: `audio.transcribe` for remote configuration or `media_download.transcribe` for local configuration.
- `fallback_capability`: always `media_download.transcribe` for this configured-STT-first workflow.
- `model`: resolved model name
- `model_kind`: adapter/runtime mode chosen by implementation
- `audio_path`: original local path or URL string actually used
- `outputs`: bounded machine-readable raw STT preview.
- `transcription_review`: on actual success, contains the complete raw STT text, source provider/model, requested response language, correction requirements, and the shared 200-character delivery threshold. The protocol `text` field is `AUDIO_TRANSCRIPTION_READY`; the complete result is synthesized from this structured contract.
- `latency_ms`: reserved latency field
- `model_kind=chat_audio` identifies Qwen's compatible `input_audio` request;
  `native` and `compat` retain their provider-adapter meanings.

## Error Contract
- Missing audio source.
- Invalid/unreadable local audio path or invalid URL input.
- Compatible adapters that require local file upload return clear path-related errors.
- Native adapters that require public URL input return clear URL/configuration errors.
- Provider/runtime transcription failures return clear error text plus `fallback_recommended=true` and `fallback_capability=media_download.transcribe` where local fallback applies.
- Machine-readable failures use `error_code`, `message_key`, and `retryable` (including invalid input/size/configuration/client/request failures); runtime and UI must not parse `error_text` or expose internal transport markers.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"preview_transcribe","file":"recordings/meeting.wav"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"AUDIO_TRANSCRIBE_PREVIEW","extra":{"action":"preview_transcribe","status":"dry_run","dry_run":true,"provider_call":false,"provider":"qwen","provider_location":"remote","recommended_capability":"audio.transcribe","fallback_capability":"media_download.transcribe","model":"qwen3-asr-flash","model_kind":"chat_audio","input_path":"recordings/meeting.wav","input_exists":false},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"audio":{"path":"recordings/meeting.wav"},"transcribe_hint":"English technical discussion"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"AUDIO_TRANSCRIPTION_READY","extra":{"provider":"openai","provider_location":"remote","model":"gpt-4o-mini-transcribe","model_kind":"compat","audio_path":"recordings/meeting.wav","outputs":[{"type":"text","preview":"Transcription: ..."}],"transcription_review":{"required":true,"source":"configured_stt","raw_text":"Transcription: ...","response_language":"request-language"},"latency_ms":0},"error_text":null}
```

### Example 3: preview a local configuration and select the fallback
Request:
```json
{"request_id":"demo-3","args":{"action":"preview_transcribe","audio":{"path":"recordings/chinese.wav"},"vendor":"custom","model":"local-whisper"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"AUDIO_TRANSCRIBE_PREVIEW","extra":{"provider":"custom","provider_location":"local","recommended_capability":"media_download.transcribe","fallback_capability":"media_download.transcribe","model":"local-whisper","model_kind":"compat","input_path":"recordings/chinese.wav"},"error_text":null}
```
