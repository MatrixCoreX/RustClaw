# image_generate Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the image_generate implementation.

## Capability Summary
- `image_generate` creates images from a prompt and optional style/render controls.
- It writes generated assets to an output path when requested by caller.

## Actions
- No `action` field is required.
- Generation is controlled by prompt and rendering options.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| generate | `prompt` | yes | string | - | Text prompt describing desired image. |
| generate | `size` | no | string | impl default | Output size hint. |
| generate | `style` | no | string | impl default | Stylistic rendering hint. |
| generate | `quality` | no | string | impl default | Quality/performance tradeoff hint. |
| generate | `n` | no | number | `1` | Number of images to generate. |
| generate | `output_path` | no | string(path) | auto | Save path for generated image(s). |

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
{"request_id":"demo-1","status":"ok","text":"image generated: ...","error_text":null}
```
