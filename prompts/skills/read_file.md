## read_file — standalone base skill

Independent base skill for reading file contents. Use `{"type":"call_skill","skill":"read_file","args":{"path":"..."}}`. Do not use system_basic for reading files.

## Capability
- Reads a file from the workspace and returns its text content.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | yes | string(path) | - | Path to the file (relative to workspace or absolute). |

## Output
- File content as text. Large files may be truncated.
