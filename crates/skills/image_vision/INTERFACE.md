# image_vision Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the `image_vision` implementation.

## Capability Summary
- `image_vision` analyzes one or more images for description, extraction, comparison, and screenshot summaries.
- It returns textual understanding without mutating source images.
- **Output language is owned by this skill end-to-end.** The host (`clawd`) does **not** rewrite `image_vision` result text after the skill returns.

## Actions
- `describe`
- `extract`
- `compare`
- `screenshot_summary`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions. |
| all | `images` | yes | array | - | Image inputs: objects with `path`, `url`, or `base64`, or string shorthand. |
| all | `instruction` / `query` / `text` | no | string | - | Optional user instruction or question to guide the image analysis. |
| all | `response_language` | no | string | - | Preferred language tag or name for the **final** user-visible text (e.g. `zh-CN`, `English`). |
| all | `language` | no | string | - | Used only when `response_language` is absent or empty (not a parallel alias on the same tier). |
| all | `detail_level` | no | string | `normal` | For `describe`, controls verbosity. |
| all | `schema` | no | JSON | - | For `extract`, optional extraction schema hint. |

### Language behavior (skill-side only)
1. **Host does not shape language args:** The host (`clawd`) does **not** inject `response_language` (or similar) into `image_vision` args before execution. Language selection is entirely this skill’s responsibility.
2. **Priority (target language for prompts + optional rewrite):**
   - Non-empty `args.response_language`
   - Else non-empty `args.language`
   - Else non-empty `context.response_language` or `context.language` on the generic runner `context` object (if present)
   - Else `args._memory.lang_hint` when skill memory injection is enabled
   - Else `response_language` / `language` entries inside `args._memory.preferences` (last matching entry wins, same idea as structured preferences)
   - Else optional OpenAI-compatible **`/v1/chat/completions`** inference using `prompts/language_infer_prompt.md` over `args._memory.context` when that block is non-empty and not `<none>`
   - Else default neutral language hints (no forced target language)
3. **Prompt:** The vision request is built with `prompts/image_vision_language_hint_with_target.md` or `image_vision_language_hint_default.md` so the multimodal model is instructed in the chosen language (or default neutral hint when no target is resolved).
4. **Optional same-turn rewrite (narrative actions only):** For `describe`, `compare`, and `screenshot_summary`, when a target language is set, the skill may run an additional OpenAI-compatible **`/v1/chat/completions`** pass using `prompts/image_output_rewrite_prompt.md` to align the final text with `__TARGET_LANGUAGE__`, preserving facts. If that step fails or returns empty output, the skill returns the vision model’s text unchanged.
5. **`extract`:** Relies on the vision prompt + language hints only (no separate rewrite pass), so structured extraction stays stable.

**Note:** Steps that read `args._memory` require `[memory].skill_memory_enabled` (and non-`chat` skills) so the host injects the generic `_memory` blob; when memory injection is off, only explicit args, runner `context`, and defaults apply.

## Error Contract
- Missing/empty `images` input array.
- Unsupported action.
- Invalid image source/path/URL/base64 decode failures.

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
