# log_analyze Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the log_analyze implementation.

## Capability Summary
- `log_analyze` scans logs for notable errors/events and summarizes key findings.
- It can target a specific log file, or a directory path whose newest log-like file will be analyzed automatically.
- It can narrow results with keyword filters.
- When a task asks for recent/tail log lines together with an anomaly or health judgment, it can return a bounded tail excerpt through `tail_lines` / `tail` / `n`.
- Even without explicit `keywords`, it returns structured severity evidence (`level_counts`, `recent_notable_lines`) and recovery evidence (`recovery_counts`, `recent_recovery_lines`) so warning/error and retry/recovery lines remain observable.
- Planner selection: prefer `log_analyze` over generic file reading when the task asks for log health, notable anomalies, errors, warnings, failures, timeouts, retries, or recovery signals in a log file or log directory.

## Actions
- No action field is required for baseline analysis.
- Optional behavior is controlled by filter parameters.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| analyze | `path` | no | string(path) | impl default | Log file path, or a directory path whose newest log-like file will be analyzed. |
| analyze | `keywords` | no | array/string | - | Keyword filters for matching lines. |
| analyze | `max_matches` | no | number | impl default | Cap for returned evidence rows. |
| analyze | `tail_lines` | no | number | 0 | Return the last N log lines as bounded `tail_lines` / `tail_excerpt` evidence. |
| analyze | `tail` | no | number | 0 | Alias for `tail_lines`. |
| analyze | `n` | no | number | 0 | Alias for `tail_lines` when planner has a generic count. |

## Output Fields
- `requested_path`: path requested by the caller.
- `path`: resolved log file path.
- `total_lines`: number of scanned lines.
- `keyword_counts`: counts for configured or default operational keywords.
- `recent_matches`: recent keyword-matching lines.
- `level_counts`: counts by detected log level (`warn`, `error`, `fatal`, `panic`, etc.).
- `recent_notable_lines`: recent `warn` or higher severity lines, independent of keyword filters.
- `recovery_counts`: counts for operational recovery tokens such as `retry`, `recover`, `recovered`, `succeeded`, and `success`.
- `recent_recovery_lines`: recent lines containing retry/recovery tokens, independent of severity level.
- `tail_lines_requested`: requested bounded tail line count.
- `tail_lines`: last N log lines with line numbers when `tail_lines` / `tail` / `n` is provided.
- `tail_excerpt`: newline-joined form of `tail_lines`.

## Error Contract
- Invalid/missing log path when path is provided.
- Directory path with no readable files should return a clear error.
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

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"path":"logs","keywords":["panic","timeout"]}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"{\"requested_path\":\"logs\",\"path\":\"logs/clawd.log\",...}","error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"path":"logs/act_plan.log","tail_lines":3}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"{\"path\":\"logs/act_plan.log\",\"tail_lines\":[\"10: ...\"],\"tail_excerpt\":\"10: ...\"}","error_text":null}
```
