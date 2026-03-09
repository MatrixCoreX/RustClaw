# db_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the db_basic implementation.

## Capability Summary
- `db_basic` provides basic SQLite query/execute capabilities.
- Read operations and mutating operations are separated by action and confirmation rules.

## Actions
- `sqlite_query`
- `sqlite_execute`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | yes | string | - | Must be `sqlite_query` or `sqlite_execute`. |
| all | `sql` | yes | string | - | SQL statement text. |
| all | `db_path` | no | string(path) | impl default | SQLite database file path. |
| `sqlite_query` | `limit` | no | number | impl default | Row cap for query results. |
| `sqlite_execute` | `confirm` | yes | boolean | - | Must be `true` for mutating execute. |

## Error Contract
- Missing action/sql/confirm fields as required.
- `sqlite_query` with non-read-only SQL should be rejected.
- SQL/runtime errors should return explicit database error text.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"sqlite_query","db_path":"data/app.db","sql":"SELECT * FROM users LIMIT 5"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"rows=5 ...","error_text":null}
```
