## write_file — standalone base skill

Independent base skill for writing file contents. Use `{"type":"call_skill","skill":"write_file","args":{"path":"...","content":"..."}}`. Use `append=true` only when appending content to the existing file tail. Do not use system_basic for writing files.

## Capability
- Writes or appends text content to a file; creates parent directories if needed.
- For a new user-deliverable file with no user-named destination, pass a descriptive bare filename. The runtime places it in the configured default output directory. Do not target runtime-owned state, checkpoint, cache, dependency, package-manager, or VCS metadata directories.

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | yes | string(path) | - | Target file path. Use a bare filename when the user did not name a destination. |
| `content` | yes | string | - | Content to write. |
| `append` | no | bool | `false` | Append `content` to the target instead of replacing the file. |

## Output
- Confirmation with path and byte count.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
