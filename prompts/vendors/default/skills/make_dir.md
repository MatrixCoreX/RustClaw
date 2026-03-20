## make_dir — standalone base skill

Independent base skill for creating directories. Use `{"type":"call_skill","skill":"make_dir","args":{"path":"..."}}`. Do not use system_basic for creating directories.

## Capability
- Creates a directory and any missing parent directories (like mkdir -p).

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | yes | string(path) | - | Directory path to create. |

## Output
- Confirmation with the created path.
