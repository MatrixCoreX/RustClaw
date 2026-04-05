## write_file — standalone base skill

Independent base skill for writing file contents. Use `{"type":"call_skill","skill":"write_file","args":{"path":"...","content":"..."}}`. Do not use system_basic for writing files.

## Capability
- Writes text content to a file; creates parent directories if needed.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | yes | string(path) | - | Target file path. |
| `content` | yes | string | - | Content to write. |

## Output
- Confirmation with path and byte count.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
