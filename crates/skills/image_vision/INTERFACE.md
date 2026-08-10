# image_vision Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the `image_vision` implementation.

## Capability Summary
- `image_vision` analyzes one or more images for description, extraction, visible-text transcription, comparison, and screenshot summaries.
- For a turn containing exactly one user-uploaded image and no typed natural-language instruction, the host uses one `describe` result as the default reply evidence: a concise image description plus `visible_text` when that array is non-empty. If no readable text is visible, the reply contains only the description and does not add an empty OCR section. A typed user instruction overrides this default and defines the requested image operation.
- For an explicit visible-text recognition request, `extract_text` is the preferred Agent capability because the independently configured image-understanding model can use layout and visual context. After exact visible-text extraction, the skill runs a separate model-review pass to restore sentence boundaries, paragraphs, punctuation, and highly certain recognition errors without translating or changing facts. Review failure returns the raw recognized text. The final UTF-8 `.txt` artifact is delivered by default. For multiple images, non-empty text remains in input order without source labels. Local Tesseract OCR is the fallback when multimodal recognition is unavailable or local processing is requested.
- Ordinary image/media download requests must not trigger `extract_text`; without an explicit conversion request, only the original images/videos are downloaded and returned.
- It never mutates source images and writes generated text only to the runtime-provided task artifact directory.
- MiniMax M3 image understanding uses its configured OpenAI-compatible chat endpoint with structured image content parts. The skill sends local image bytes as a typed data URL, never as an untyped text marker.
- It supports Mimo image understanding through OpenAI-compatible chat completions (`mimo-v2.5` / `mimo-v2-omni`); this is image understanding, not image generation.
- **Output language and review are owned by this skill end-to-end.** The host (`clawd`) does **not** add an image-specific rewrite or delivery branch.

## Actions
- `describe`
- `analyze` (compatibility alias for `describe`)
- `extract`
- `extract_text`
- `compare`
- `screenshot_summary`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions; `analyze` is normalized to `describe`. |
| all | `images` / `image` | yes | array or single image | - | Provide exactly one form. `images` has no host-defined item-count ceiling: pass the complete ordered current-task image set in one call instead of splitting it to satisfy an arbitrary count. Every array item/object must contain a non-empty `path`, `url`, or `base64` value (or valid string shorthand); empty objects are invalid. Per-image byte, request timeout, provider context, and runtime cancellation limits still apply. |
| all | `instruction` / `query` / `text` | no | string | - | Optional user instruction or question to guide the image analysis. |
| all | `response_language` | no | string | - | Preferred language tag or name for the **final** user-visible text (e.g. `zh-CN`, `English`). |
| all | `language` | no | string | - | Used only when `response_language` is absent or empty (not a parallel alias on the same tier). |
| all | `detail_level` | no | string | `normal` | For `describe`, controls verbosity. |
| all | `schema` | no | JSON | - | For `extract`, optional extraction schema hint. |
| `extract_text` | `output_name` | no | string | `image_text_ai.txt` | Plain `.txt` filename for the generated UTF-8 text artifact. The default name identifies multimodal-model output. |
| `extract_text` | `save_text_file` | no | boolean | `true` | Generate the text file. Set false only when the user explicitly requests inline text without a file. |
| `extract_text` | `deliver_to_user` | no | boolean | `true` | Return the text artifact to the originating communication channel/UI. Set false only when explicitly requested. |

## Planner Selection
- One user-uploaded image with no typed instruction -> use the current `describe` analysis once, reply with its description and all non-empty `visible_text` entries in reading order, and do not run a second OCR pass. Omit the text-recognition portion when `visible_text` is empty.
- When the user supplies a natural-language instruction with the image, follow that instruction instead of appending the attachment-only default output.
- Explicit image/screenshot visible-text recognition or OCR-to-file request -> prefer `image_vision.extract_text`, which recognizes, model-reviews, and returns a UTF-8 `.txt` artifact by default.
- Form fields, tables, or other visually structured data extraction -> use `image_vision.extract`.
- A plain media-download request is not an image-text-recognition request. Do not add `extract_text` after `media_download.download` unless the current user explicitly asks for conversion.
- If images were produced by another capability in the same turn, pass those successful current-task artifact paths into `images`; do not ask the user to upload the same images again and do not substitute an older task's artifact unless the user explicitly refers to it.
- Preserve every produced image in source order. Do not truncate or split the set merely because it contains more than six images; `image_vision` no longer has a host-side item-count ceiling.
- On a structured provider/configuration/unsupported-input failure from `extract_text`, the planner may fall back to an available local OCR capability such as `media_download.ocr`.
- Local OCR fallback accepts images only. If the downloaded artifact is video and the user requested text conversion, use `media_download.transcribe` to extract audio and recognize speech instead.
- Do not use local OCR merely because the image originated from a media-download skill; capability selection follows the requested output, not the producing skill name.

## Config Entry Points
- Independent image-understanding provider/model: `configs/image.toml` -> `[image_vision].default_vendor` / `default_model`. This selection is independent from the main `[llm]` provider/model.
- The default `minimax` provider uses `MiniMax-M3` through OpenAI-compatible multimodal chat. If it is unavailable or fails, `image_vision` reports an execution failure so the Agent can use local OCR; it does not silently switch to the main text model.
- Recommended shared credential for the same provider: `configs/config.toml` -> `[llm.minimax].api_key`, or environment `MINIMAX_API_KEY`.
- Optional dedicated image-understanding connection/credential: `configs/image.toml` -> `[image_vision.providers.minimax]`, or environment `IMAGE_VISION_MINIMAX_API_KEY`.
- MiniMax compatible chat receives images only as typed `image_url` content parts; Base64 is never interpolated into ordinary prompt text.

### Language behavior (skill-side only)
1. **Host vs skill (target language):**
   - The host (`clawd`) does **not** infer or inject a **default** target language for `image_vision` (no image_vision-specific language shaping on the platform).
   - Explicit user-provided `response_language` / `language` in the request are still **forwarded unchanged** to the skill when present.
   - **Fallback** target-language selection and **final** output-language behavior (prompt + optional rewrite) are owned by this skill; the host does not rewrite skill result text.
2. **Priority (target language for prompts + optional rewrite):**
   - Non-empty `args.response_language`
   - Else non-empty `args.language`
   - Else non-empty `context.response_language` or `context.language` on the generic runner `context` object (if present)
   - Else `args._memory.lang_hint` when skill memory injection is enabled
   - Else `response_language` / `language` entries inside `args._memory.preferences` (last matching entry wins, same idea as structured preferences)
   - Else optional OpenAI-compatible **`/v1/chat/completions`** inference using `prompts/language_infer_prompt.md` over `args._memory.context` when that block is non-empty and not `<none>`
   - Else default neutral language hints (no forced target language)
3. **Prompt:** The vision request is built with `prompts/image_vision_language_hint_with_target.md` or `image_vision_language_hint_default.md` so the multimodal model is instructed in the chosen language (or default neutral hint when no target is resolved).
4. **Narrative action schema guard:** For `describe`, `compare`, and `screenshot_summary`, the skill validates the model JSON against authored in-repo schemas before using it. When validation succeeds, the structured payload is exposed under `extra.structured`, and `text` is rendered from that structured result instead of forwarding raw JSON directly.
5. **Optional same-turn rewrite (narrative actions only):** For `describe`, `compare`, and `screenshot_summary`, when a target language is set, the skill may run an additional OpenAI-compatible **`/v1/chat/completions`** pass using `prompts/image_output_rewrite_prompt.md` to align the final rendered text with `__TARGET_LANGUAGE__`, preserving facts. If that step fails or returns empty output, the skill returns the schema-rendered text unchanged.
6. **`extract`:** Relies on the vision prompt and schema only.
7. **`extract_text`:** Validates exact page-ordered recognition first, then runs `prompts/image_text_revision_prompt.md` as a separate skill-owned text-model pass. That pass preserves source languages and facts while repairing sentence boundaries, paragraph layout, punctuation, obvious OCR errors, and typos. It is chunked without dropping Unicode text. Failure returns the exact raw recognition result. Only the resulting text is written to the task artifact directory.

**Note:** Steps that read `args._memory` require `[memory].skill_memory_enabled` and a runner skill that supports generic memory injection so the host injects the `_memory` blob; when memory injection is off, only explicit args, runner `context`, and defaults apply.

## Error Contract
- Missing/empty `images` input array.
- Unsupported action.
- Invalid image source/path/URL/base64 decode failures.
- Missing runtime artifact directory or an invalid `output_name` for `extract_text`.
- No configured image-understanding provider returned visible text. This is an execution failure and permits planner fallback to `media_download.ocr`; no empty text artifact is created.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"describe","images":[{"path":"assets/screen.png"}]}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"The screenshot shows ...","error_text":null}
```

### Example 2 — Target language
Request:
```json
{"request_id":"demo-2","args":{"action":"describe","response_language":"zh-CN","images":[{"path":"assets/screen.png"}]}}
```
Final `text` is produced entirely inside the skill (prompt + optional rewrite as above); the host does not post-process it.

### Example 3 — Explicit image text recognition with default file delivery
Request:
```json
{"request_id":"demo-3","args":{"action":"extract_text","response_language":"zh-CN","images":[{"path":"artifacts/note-1.jpg"},{"path":"artifacts/note-2.jpg"}]},"context":{"artifact_output_directory":"/runtime/artifacts/invocation"}}
```
Response includes one continuous reviewed document in `text` without image labels, a UTF-8 `image_text_ai.txt` entry in `extra.artifacts`, `extra.recognition.source="multimodal_model"`, `extra.recognition.reviewed_by_model`, structured `extra.recognition_review` diagnostics, and `extra.delivery.deliver_to_user=true`. If review is unavailable, the raw recognized text is returned with `reviewed_by_model=false`. If `deliver_to_user=false`, the file is returned under `extra.saved_files` instead and is not sent.
