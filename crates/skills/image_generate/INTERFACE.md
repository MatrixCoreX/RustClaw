# image_generate Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the image_generate implementation.

## Capability Summary
- `image_generate` creates images from a prompt and optional style/render controls.
- It writes generated assets to an output path and returns file markers in `text`.
- It also supports optional vendor/model routing and response language selection for the human-readable success text.
- Successful responses include machine-readable `extra` metadata such as `provider`, `model`, `model_kind`, and `outputs`.

## Actions
- No `action` field is required.
- Generation is controlled by prompt and rendering options.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| generate | `prompt` | yes | string | - | Text prompt describing desired image. |
| generate | `size` | no | string | `1024x1024` | Output size hint. |
| generate | `style` | no | string | impl default | Stylistic rendering hint. |
| generate | `quality` | no | string | impl default | Quality/performance tradeoff hint. |
| generate | `n` | no | number | `1` | Number of images to generate. |
| generate | `output_path` | no | string(path) | auto | Save path for generated image(s). |
| generate | `response_language` or `language` | no | string | impl/config default | Language for the human-readable success text. |
| generate | `vendor` | no | string | impl default | Backend vendor selector. |
| generate | `model` | no | string | impl default | Backend model selector. |
| generate | `timeout_seconds` | no | integer | impl/config default | Per-request timeout, clamped by implementation. |

## Config Entry Points
- `configs/image.toml` -> `[image_generation]` controls provider routing, model defaults, output limits, and `local_fallback_enabled`.
- `IMAGE_GENERATION_LOCAL_FALLBACK=1|true|on|yes` overrides `local_fallback_enabled` for explicit smoke/NL-test continuity when all configured providers fail.
- Provider credentials use dedicated environment variables such as `IMAGE_GENERATION_MINIMAX_API_KEY`; do not reuse chat/model API key names for image generation.

## Success `extra` (`status=ok`)
- `provider`: resolved backend provider name
- `model`: resolved model name
- `model_kind`: adapter/runtime mode chosen by implementation
- `latency_ms`: reserved latency field
- `outputs`: machine-readable output summary, currently `[{\"type\":\"image_file\",\"path\":\"...\"}]`
- `fallback`: present only when an explicit local fallback produced the file after provider generation failed.

## Error Contract
- Missing or empty `prompt`.
- Invalid option values (`size/style/quality/n`).
- Provider/runtime generation failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"prompt":"Minimal black app icon with claw mark","size":"1024x1024"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"Generated successfully and saved: image/out-1.png\nFILE:image/out-1.png\nEPHEMERAL:IMAGE_SAVED","extra":{"provider":"openai","model":"gpt-image-1","model_kind":"native","latency_ms":0,"outputs":[{"type":"image_file","path":"image/out-1.png"}]},"error_text":null}
```
