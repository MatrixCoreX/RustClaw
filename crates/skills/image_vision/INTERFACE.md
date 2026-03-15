# image_vision Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the image_vision implementation.

## Capability Summary
- `image_vision` analyzes one or more images for description, extraction, comparison, and screenshot summaries.
- It returns textual understanding without mutating source images.

## Actions
- `describe`
- `extract`
- `compare`
- `screenshot_summary`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported actions. |
| all | `images` | yes | array | - | Image inputs as `{path|url|base64}` items. |
| all | `instruction` / `query` | no | string | - | Optional user instruction or question to guide the image analysis. |
| optional | language/format hints | no | string | impl default | Optional output shaping hints. |

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
