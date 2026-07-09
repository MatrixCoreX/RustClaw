## make_dir — standalone base skill

Independent base skill for creating directories. Use `{"type":"call_skill","skill":"make_dir","args":{"path":"..."}}`. Do not use system_basic for creating directories.

## Capability
- Creates a directory and any missing parent directories (like mkdir -p).

## Parameter contract
| Param | Required | Type | Default | Description |
|-------|----------|------|---------|-------------|
| `path` | yes | string(path) | - | Directory path to create. |
| `parents` | no | bool | `true` | Whether to create missing parent directories. |
| `recursive` | no | bool | `true` | Alias for `parents` when planning mkdir-style operations. |

## Output
- Confirmation with the created path.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
