## remove_file — standalone base skill

Independent base skill for removing a file. Use `{"type":"call_skill","skill":"remove_file","args":{"path":"..."}}`. Do not use system_basic for removing files.

## Capability
- Removes a single file (not directories; for directory removal use run_cmd e.g. `rm -rf` when appropriate).

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | yes | string(path) | - | Path to the file to remove. |

## Output
- Confirmation with the removed path.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
