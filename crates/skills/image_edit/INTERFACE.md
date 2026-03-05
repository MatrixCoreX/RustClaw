# image_edit Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the image_edit implementation.

## Capability Summary
- `image_edit` modifies existing images using natural language instructions.
- It supports generic edit/outpaint/restyle/add-remove operations.

## Actions
- `edit`
- `outpaint`
- `restyle`
- `add_remove`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be one of supported edit actions. |
| all | `instruction` | yes | string | - | Edit instruction text. |
| all | `image` | no | string/object | - | Input image path/url/base64 payload. |
| all | `mask` | no | string/object | - | Optional mask for local edits. |
| all | `output_path` | no | string(path) | auto | Output location for edited asset. |

## Error Contract
- Missing `instruction`.
- Unsupported action.
- Missing/invalid source image when operation requires one.
- Provider/runtime edit failures should return clear error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"restyle","instruction":"pixel-art style","image":{"path":"assets/logo.png"}}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"image edited: ...","error_text":null}
```
