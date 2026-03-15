# log_analyze Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the log_analyze implementation.

## Capability Summary
- `log_analyze` scans logs for notable errors/events and summarizes key findings.
- It can target a specific path and narrow results with keyword filters.

## Actions
- No action field is required for baseline analysis.
- Optional behavior is controlled by filter parameters.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| analyze | `path` | no | string(path) | impl default | Log file or directory path. |
| analyze | `keywords` | no | array/string | - | Keyword filters for matching lines. |
| analyze | `max_matches` | no | number | impl default | Cap for returned evidence rows. |

## Error Contract
- Invalid/missing log path when path is provided.
- Read/parse errors should return clear filesystem/runtime details.
- Oversized/unbounded scans should be summarized safely.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"path":"logs/app.log","keywords":["error","timeout"],"max_matches":50}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"Top findings: ...","error_text":null}
```
