# local_asr Interface

## Capability Summary

- Run local speech recognition in an isolated, on-demand process package.
- Support a host-provided `whisper.cpp` binary/model and a package-owned FunASR/ModelScope/Torch backend.
- Load the selected model for one invocation only; the model never remains resident in `clawd`.
- Split long or large inputs into bounded 16 kHz mono segments, recognize them in source order, and merge every non-empty part without truncation.
- Return raw transcript evidence for the shared model finalizer, which performs language-preserving correction and user delivery.

## Actions

| Action | Required | Optional | Result |
|---|---|---|---|
| `capabilities` | none | none | Engine and lifecycle metadata. |
| `transcribe` | `input_path` | `engine`, `language`, `response_language`, `whisper_bin`, `whisper_model`, `threads`, `translate`, `fast`, `no_gpu`, `funasr_model`, `funasr_vad_model`, `funasr_batch_size_s`, `rich_text` | A private raw transcript artifact and `transcription_review` contract. |

`engine` is `whisper` by default and may be `funasr`. The host selects this capability only after `audio.preview_transcribe` reports a local provider. Remote provider failure does not authorize a local fallback.

## Dependencies and Storage

- Host dependency: `ffmpeg`; Whisper also requires `whisper-cli` and a configured local model.
- FunASR is installed only with this optional package. Its locked ModelScope, Torch, and Torchaudio dependencies are not part of ordinary media downloading.
- Managed SenseVoice and FSMN VAD assets live in this skill's private directory storage.
- Runtime network access is disabled. Missing assets fail with structured `dependency_unavailable` data rather than downloading at first use.
- Inputs longer than eight minutes are segmented before recognition. If duration probing is unavailable, files larger than 64 MiB use the same bounded path. Temporary segments are removed at invocation end.

## Error Contract

Errors return `status=error`, readable `error_text`, and canonical `extra.{schema_version,source_skill,status,error_code,message_key,retryable}`. Runtime decisions must use those structured fields, not `text` or `error_text`.

## Examples

```json
{"request_id":"local-asr-1","args":{"action":"capabilities"},"context":{},"user_id":1,"chat_id":1}
```

```json
{"request_id":"local-asr-2","args":{"action":"transcribe","input_path":"recordings/meeting.wav","engine":"whisper","response_language":"zh-CN"},"context":{"workspace_root":"/workspace","artifact_output_directory":"/workspace/artifacts","skill_storage":{"storage_kind":"directory","directory_path":"/data/skills/local_asr"}},"user_id":1,"chat_id":1}
```

```json
{"request_id":"local-asr-2","status":"ok","text":"LOCAL_ASR_READY","error_text":null,"extra":{"schema_version":1,"source_skill":"local_asr","status":"ok","action":"transcribe","engine":"whisper","artifacts":[],"saved_files":[{"path":"/workspace/artifacts/meeting_transcript.txt","artifact_role":"transcript_text"}],"delivery":{"intent":"model_synthesis","deliver_to_user":true},"transcription_review":{"schema_version":1,"required":true,"source":"local_asr","source_engine":"whisper","raw_text":"...","response_language":"zh-CN"}}}
```
