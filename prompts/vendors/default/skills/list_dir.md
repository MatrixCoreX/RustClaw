## list_dir — standalone base skill

Independent base skill for listing directory entries. Use `{"type":"call_skill","skill":"list_dir","args":{"path":"..."}}`. Do not use system_basic for listing directories.

## Capability
- Lists direct entries of a directory (includes hidden/dot-prefixed when present).

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | no | string(path) | "." | Directory path to list. |

## Output
- One entry per line; directories may be suffixed with `/`.
