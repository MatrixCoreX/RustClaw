# image_edit Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the `image_edit` implementation.

## Capability Summary
- `image_edit` modifies existing images using natural language instructions.
- It supports generic edit/outpaint/restyle/add-remove operations.
- **Input recovery is implemented inside this skill** (not in `clawd`): if `image` is omitted, the skill uses host-provided generic context (`context.recent_image_paths`, newest first) and optional `args._memory.context` to pick a source image or ask for clarification.
- Successful responses include machine-readable `extra` metadata such as `provider`, `model`, normalized `action`, and `outputs`.

## Actions
- `edit`
- `outpaint`
- `restyle`
- `add_remove`

## Request shape (skill-runner → child)
- `args`: object — parameters below.
- `context`: optional object — host may include **`recent_image_paths`**: string array of workspace-relative image paths (generic delivery; the skill may ignore it when `image` is already set).

## Config Entry Points
- Default edit provider/model: `configs/image.toml` -> `[image_edit].default_vendor` / `default_model`.
- Current default: `minimax` + `image-01`.
- Preferred dedicated keys: `IMAGE_EDIT_<VENDOR>_API_KEY` or `[image_edit.providers.<vendor>].api_key`.
- If a provider override exists but its dedicated key is empty, the skill may reuse the same vendor's global key (for example `MINIMAX_API_KEY`) from `[llm.<vendor>]` / environment.
- MiniMax uses the native `/image_generation` reference-image adapter with `subject_reference`; this adapter requires `image.url` input.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | no | string | `edit` | Must be one of supported edit actions. |
| all | `instruction` | yes | string | - | Edit instruction text. |
| all | `image` | conditional | string/object | - | Input image path/url/base64. **Required** unless the skill can recover a path from `context.recent_image_paths` (single candidate, or LLM disambiguation when multiple). |
| all | `images` | no | array | - | If present with paths, treated as “has image” for recovery checks (same as host vision payloads). |
| all | `mask` | no | string/object | - | Optional mask for local edits. |
| all | `size` | no | string | `1024x1024` | Output size hint for providers that support it. |
| all | `quality` | no | string | impl default | Quality/performance tradeoff hint. |
| all | `n` | no | integer | `1` | Number of edited outputs requested; implementation clamps this value. |
| all | `output_path` | no | string(path) | auto | Output location for edited asset. |
| all | `response_language` or `language` | no | string | impl/config default | Language for the human-readable success text. |
| all | `vendor` | no | string | impl default | Backend vendor selector. |
| all | `model` | no | string | impl default | Backend model selector. |
| all | `timeout_seconds` | no | integer | impl/config default | Per-request timeout, clamped by implementation. |
| all | `_memory` | no | object | - | Optional injected memory blob; **`_memory.context`** is passed into the image-reference resolver prompt when multiple `recent_image_paths` exist. |

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name
- `model`: resolved model name
- `model_kind`: adapter/runtime mode chosen by implementation
- `latency_ms`: reserved latency field
- `action`: final normalized action actually executed
- `outputs`: machine-readable output summary, currently `[{\"type\":\"image_file\",\"path\":\"...\"}]`

## Recovery & clarification (skill-side)
1. If `image` (or `images` with paths) is already set → use it; normalize as today.
2. Else read `context.recent_image_paths` (array of strings).
3. **0 paths** → return **`error_text`** explaining that no recent image was found; user should upload, set `image.path` / url, or name the file.
4. **1 path** → use that path as `image.path`.
5. **2+ paths** → call an OpenAI-compatible **`/v1/chat/completions`** resolver using `prompts/image_reference_resolver_prompt.md` (or bundled default) with `__MEMORY_TEXT__` from `_memory.context` and `__GOAL__` = `instruction`. Expect JSON `{"selected_index":N}`. Invalid / negative index / LLM failure → **`error_text`** listing candidate indices and asking the user to set `image.path` or choose an index.

## Error Contract
- Missing `instruction`.
- Unsupported action.
- Missing/invalid source image when it cannot be recovered from context.
- Ambiguous multiple images when resolver cannot choose.
- MiniMax reference edits require a URL source image; local path/base64 inputs should use a provider with native local-image support.
- Provider/runtime edit failures should return clear error text.

## Request/Response Examples
### Example 1 — Explicit path
Request:
```json
{"request_id":"demo-1","args":{"action":"restyle","instruction":"pixel-art style","image":{"path":"assets/logo.png"}},"context":{}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"Edited successfully and saved: image/out-1.png\nFILE:image/out-1.png\nEPHEMERAL:IMAGE_SAVED","extra":{"provider":"openai","model":"gpt-image-1","model_kind":"native","latency_ms":0,"action":"restyle","outputs":[{"type":"image_file","path":"image/out-1.png"}]},"error_text":null}
```

### Example 2 — Recover from context (host provides `recent_image_paths`)
Request:
```json
{"request_id":"demo-2","args":{"instruction":"remove background"},"context":{"recent_image_paths":["image/out-1.png"]}}
```
The skill fills `image.path` internally from the single candidate.
