# reference_resolver Interface Spec

## Capability Summary

`reference_resolver` resolves ambiguous references in user requests (for example "上个回复", "那个文件", "那个依赖", "that result").

It is a read-only resolver and does not execute installation, file mutation, or business actions.

## Actions

- `resolve_reference`

Resolve a user reference against recent context candidates (turns, task results, files, dependencies), with scoring and confidence.

## Parameter Contract

- `action` (required, string): `resolve_reference`
- `request_text` (required, string): raw user text containing references
- `recent_turns` (required, array): recent turns (assistant/user/tool) used for candidate recall
- `recent_results` (optional, array): recent task/tool results used for candidate recall
- `target_type` (optional, string, default `generic`): `reply|task|file|dependency|generic`
- `language_hint` (optional, string): e.g. `zh-CN`, `en-US`; used for clarify question language
- `max_candidates` (optional, integer, default `5`, range `1..10`): candidate output cap
- `include_trace` (optional, bool, default `false`): include scoring trace for debugging

## Error Contract

- Return `status=ambiguous` when multiple candidates are close in score; include `top_candidates` and `clarify_question`.
- Return `status=not_found` when no reliable binding is found; include explicit `message`.
- Low confidence must not be promoted to `resolved`.

## Request/Response Examples

### Example 1

Request:
```json
{
  "request_id": "ref-1",
  "args": {
    "action": "resolve_reference",
    "request_text": "用上个回复那个文件",
    "target_type": "file",
    "language_hint": "zh-CN",
    "max_candidates": 5,
    "recent_turns": [
      {"role": "assistant", "turn_id": "a1", "text": "已保存到 /tmp/report.md"}
    ],
    "recent_results": []
  }
}
```

Response:
```json
{
  "request_id": "ref-1",
  "status": "ok",
  "text": "{\"status\":\"resolved\",\"confidence\":0.84,\"resolved_ref\":{\"kind\":\"file\",\"id\":\"file:/tmp/report.md\"},\"top_candidates\":[{\"kind\":\"file\",\"id\":\"file:/tmp/report.md\"}],\"clarify_question\":null,\"message\":null}",
  "error_text": null
}
```

Returned JSON inside `text` contains:

- `status`: `resolved|ambiguous|not_found`
- `confidence`: float in `[0,1]`
- `resolved_ref` (object | null):
  - `kind`: `reply|task|file|dependency|generic`
  - `id`: resolved candidate id
  - `turn_index`: nullable integer
  - `source`: `recent_turns|recent_results`
  - `score`: numeric score
  - `preview`: short preview text
- `top_candidates` (array): each candidate contains:
  - `kind`, `id`, `turn_index`, `source`, `score`, `confidence`, `preview`
- `clarify_question` (string | null): direct question for user when ambiguous
- `message` (string | null): not_found/ambiguous explanation
- `resolution_trace` (object, optional): included when `include_trace=true`

## Scoring Notes

Candidate scoring uses rule-based weighted components:

- time decay (recency)
- kind match vs `target_type`
- keyword hints (reply/task/file/dependency patterns)
- lexical similarity (lightweight semantic proxy)
