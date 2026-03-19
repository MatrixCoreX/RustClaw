# image_edit Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the `image_edit` implementation.

## Capability Summary
- `image_edit` modifies existing images using natural language instructions.
- It supports generic edit/outpaint/restyle/add-remove operations.
- **Input recovery is implemented inside this skill** (not in `clawd`): if `image` is omitted, the skill uses host-provided generic context (`context.recent_image_paths`, newest first) and optional `args._memory.context` to pick a source image or ask for clarification.

## Actions
- `edit`
- `outpaint`
- `restyle`
- `add_remove`

## Request shape (skill-runner → child)
- `args`: object — parameters below.
- `context`: optional object — host may include **`recent_image_paths`**: string array of workspace-relative image paths (generic delivery; the skill may ignore it when `image` is already set).

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | no | string | `edit` | Must be one of supported edit actions. |
| all | `instruction` | yes | string | - | Edit instruction text. |
| all | `image` | conditional | string/object | - | Input image path/url/base64. **Required** unless the skill can recover a path from `context.recent_image_paths` (single candidate, or LLM disambiguation when multiple). |
| all | `images` | no | array | - | If present with paths, treated as “has image” for recovery checks (same as host vision payloads). |
| all | `mask` | no | string/object | - | Optional mask for local edits. |
| all | `output_path` | no | string(path) | auto | Output location for edited asset. |
| all | `_memory` | no | object | - | Optional injected memory blob; **`_memory.context`** is passed into the image-reference resolver prompt when multiple `recent_image_paths` exist. |

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
- Provider/runtime edit failures should return clear error text.

## Request/Response Examples
### Example 1 — Explicit path
Request:
```json
{"request_id":"demo-1","args":{"action":"restyle","instruction":"pixel-art style","image":{"path":"assets/logo.png"}},"context":{}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"image edited: ...","error_text":null}
```

### Example 2 — Recover from context (host provides `recent_image_paths`)
Request:
```json
{"request_id":"demo-2","args":{"instruction":"remove background"},"context":{"recent_image_paths":["image/out-1.png"]}}
```
The skill fills `image.path` internally from the single candidate.
