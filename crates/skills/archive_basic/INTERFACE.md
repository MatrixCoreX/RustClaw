# archive_basic Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with `crates/skills/archive_basic/src/main.rs`.

## Capability Summary
- `archive_basic` provides archive operations for listing archive contents, reading one member from an archive, packing files/folders into an archive, and unpacking archives into a destination directory.
- Supported archive types are `zip` and `tar.gz`/`tgz`.
- `unpack` uses a non-interactive default overwrite strategy (zip: `unzip -o`; tar: `tar --overwrite`) to avoid hanging on interactive replace prompts.
- Relative paths are resolved against `WORKSPACE_ROOT`.
- Explicit absolute paths are accepted when they are already concrete user-provided paths.
- All paths reject `..` traversal.

## Actions
- `list`: list entries in an archive file.
- `read`: output the content of one member inside an archive.
- `pack`: create an archive from a source path.
- `unpack`: extract an archive into a destination directory.

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `list` | `action` | yes | string | - | Must be `list`. |
| `list` | `archive` | yes | string(path) | - | Archive file path (relative to workspace or explicit absolute path). Runtime also accepts `archive_path` or `path` as aliases and normalizes to `archive`. |
| `read` | `action` | yes | string | - | Must be `read`. |
| `read` | `archive` | yes | string(path) | - | Archive file path (relative to workspace or explicit absolute path). Runtime also accepts `archive_path` or `path` as aliases and normalizes to `archive`. |
| `read` | `member` | yes | string(relative path) | - | File path inside the archive. Runtime also accepts `entry`, `file`, or `file_path` as aliases. Must be relative and reject `..`. |
| `pack` | `action` | yes | string | - | Must be `pack`. |
| `pack` | `source` | yes | string(path) | - | Source file or directory to archive. |
| `pack` | `archive` | yes | string(path) | - | Output archive file path. Parent dir is auto-created. Runtime also accepts `archive_path` as an alias and normalizes to `archive`. |
| `pack` | `format` | no | string | `zip` | Supported: `zip`, `tar.gz`, `tgz` (`tgz` handled as `tar.gz`). |
| `unpack` | `action` | yes | string | - | Must be `unpack`. |
| `unpack` | `archive` | yes | string(path) | - | Input archive file path. Runtime also accepts `archive_path` or `path` as aliases and normalizes to `archive`. |
| `unpack` | `dest` | yes | string(path) | - | Extraction destination directory (auto-created; relative to workspace or explicit absolute path). |

## Error Contract
- Input/shape errors:
  - `args must be object`
  - `<key> is required` (for missing required string args, e.g. `archive is required`)
- Action/format errors:
  - `unsupported action; use list|read|pack|unpack`
  - `unsupported format; use zip|tar.gz`
  - `unsupported archive format for list`
  - `unsupported archive format for read`
  - `unsupported archive format for unpack`
- Path safety errors:
  - `path with '..' is not allowed`
  - `archive member must be a relative path`
  - `archive member with '..' is not allowed`
- Runtime/system errors:
  - `mkdir failed: <error>`
  - `run <bin> failed: <error>`
  - `archive command failed: exit=<code>\n<stdout/stderr>`
  - On malformed stdin JSON request: `invalid input: <serde error>`
- Successful `list` returns a single-line JSON object in `text` with `action`, `archive`, `count`, `entries`, `candidates`, and `output`.
- Successful `pack`/`unpack` command execution output is returned in `text` as `exit=<code>\n<stdout/stderr>`.
- Successful `read` returns a single-line JSON object in `text` with `action`, `archive`, `member`, and `content`.
- Non-zero archive command exit codes are returned as `status=error` with `error_text=archive command failed: exit=<code>\n<stdout/stderr>`.
- Successful responses also mirror structured metadata into `extra`, including `action`, relevant paths, and `output`.

## Structured Evidence Contract
- Runtime evidence source: archive results must come from structured `extra`;
  natural-language `text` is an untrusted fallback and must not select routing,
  retry, success, or final-answer shape.
- Ordinary list/read/pack/unpack responses use `result_kind="none"` and model
  synthesis from the capability result. Exact output uses generic selectors
  such as `members`, `count`, `content_excerpt`, `archive`, or `dest`;
  artifact delivery uses the structured `artifacts` array.
- `list` success `extra` fields:
  - `action`: string, always `list`; evidence role `status`.
  - `archive`: string absolute/resolved archive path; evidence role `path`.
  - `count`: integer member count; evidence role `count`.
  - `entries`: array of objects with `name` and `kind`; evidence role `candidates`.
  - `candidates`: array of archive member names; evidence role `candidates`.
  - `output`: string command observation; fallback evidence only.
- `read` success `extra` fields:
  - `action`: string, always `read`; evidence role `status`.
  - `archive`: string absolute/resolved archive path; evidence role `path`.
  - `member`: string archive member path; evidence role `path`.
  - `content`: string member content; evidence role `field_value`.
- `pack` success `extra` fields:
  - `action`: string, always `pack`; evidence role `status`.
  - `format`: string archive format; evidence role `field_value`.
  - `source`: string resolved source path; evidence role `path`.
  - `archive`: string output archive path; evidence role `artifact_path`.
  - `output`: string command observation; fallback evidence only.
  - `field_value`: object containing `archive`, `format`, and `source`.
  - `artifacts`: array containing the created archive path and machine metadata.
- `unpack` success `extra` fields:
  - `action`: string, always `unpack`; evidence role `status`.
  - `archive`: string input archive path; evidence role `path`.
  - `dest`: string extraction directory; evidence role `path`.
  - `output`: string command observation; fallback evidence only.
  - `field_value`: object containing `dest`.
- Sensitive fields: archive member `content` may include user data. Provider-facing traces should prefer excerpt/hash/length metadata unless the user explicitly requested the content.
- Error responses include top-level `error_kind`; `extra.error_kind` is present when the error has path/action context.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"demo-1","args":{"action":"list","archive":"tmp/sample.zip"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"{\"action\":\"list\",\"archive\":\"/workspace/tmp/sample.zip\",\"count\":2,\"entries\":[{\"name\":\"notes.txt\",\"kind\":\"file\"},{\"name\":\"nested/config.ini\",\"kind\":\"file\"}],\"candidates\":[\"notes.txt\",\"nested/config.ini\"],\"output\":\"exit=0\\nnotes.txt\\nnested/config.ini\"}","extra":{"action":"list","archive":"/workspace/tmp/sample.zip","count":2,"entries":[{"name":"notes.txt","kind":"file"},{"name":"nested/config.ini","kind":"file"}],"candidates":["notes.txt","nested/config.ini"],"output":"exit=0\nnotes.txt\nnested/config.ini"},"error_text":null}
```

### Example 2
Request:
```json
{"request_id":"demo-2","args":{"action":"read","archive":"tmp/sample.zip","member":"notes.txt"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"{\"action\":\"read\",\"archive\":\"/workspace/tmp/sample.zip\",\"member\":\"notes.txt\",\"content\":\"fixture archive notes\"}","extra":{"action":"read","archive":"/workspace/tmp/sample.zip","member":"notes.txt","content":"fixture archive notes"},"error_text":null}
```

### Example 3
Request:
```json
{"request_id":"demo-3","args":{"action":"pack","source":"tmp/data","archive":"tmp/data.zip","format":"zip"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"exit=0\n  adding: ...","extra":{"action":"pack","format":"zip","source":"/workspace/tmp/data","archive":"/workspace/tmp/data.zip","output":"exit=0\n  adding: ..."},"error_text":null}
```

### Example 4
Request:
```json
{"request_id":"demo-4","args":{"action":"unpack","archive":"tmp/data.tgz","dest":"tmp/out"}}
```
Response:
```json
{"request_id":"demo-4","status":"ok","text":"exit=0\n...","extra":{"action":"unpack","archive":"/workspace/tmp/data.tgz","dest":"/workspace/tmp/out","output":"exit=0\n..."},"error_text":null}
```
