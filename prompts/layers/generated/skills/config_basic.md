## config_basic — planner-facing structured config tool
Use `{"type":"call_tool","tool":"config_basic","args":{...}}` for structured TOML/JSON/YAML config reads, key listing, value-free structure summaries, parse validation, and RustClaw config guard checks. This v1 contract is read-only; it does not expose generic patch/write actions.
## Capability
Read fields, list keys, summarize structure without scalar values, validate syntax, or run the RustClaw config safety guard. Safety/risk scans are guard operations, not broad file reads.
## Actions
- `read_field`
- `read_fields`
- `list_keys`
- `summarize_structure`
- `validate`
- `guard_rustclaw_config`

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `read_field` | `path` | yes | string(path) | - | JSON/TOML/YAML file path. |
| `read_field` | `field_path` | yes | string | - | Dot/bracket path, including array filters such as `skills[?(@.name=='fs_basic')].prompt_file`; rooted shorthand `skills.fs_basic.prompt_file` is accepted for a unique `name`/`id`/`key`. |
| `read_field` | `format` | no | string | auto | `json|toml|yaml`. |
| `read_fields` | `path` | yes | string(path) | - | JSON/TOML/YAML file path. |
| `read_fields` | `field_paths` | yes | non-empty string/string[] | - | One or more exact field paths to extract. |
| `read_fields` | `format` | no | string | auto | `json|toml|yaml`. |
| `list_keys` | `path` | yes | string(path) | - | JSON/TOML/YAML file path. |
| `list_keys` | `field_path` | no | string | root | Optional object/array location. |
| `list_keys` | `max_keys` | no | integer | impl default | Output cap. |
| `summarize_structure` | `path` | yes | string(path) | - | JSON/TOML/YAML file to summarize without returning scalar values. |
| `summarize_structure` | `field_path` | no | string | root | Optional exact subtree to count, for example `skills.skill_switches`. |
| `summarize_structure` | `max_paths` | no | integer | `200` | Per-category machine path preview cap; counts remain exact while the scan is complete. |
| `validate` | `path` | yes | string(path) | - | Structured file to parse. This capability checks syntax only. |
| `validate` | `format` | no | string | auto | `json|toml|yaml`. |
| `validate` | result | - | object | - | Returns `valid=true/false`; do not treat key listing as validation output. |
| `guard_rustclaw_config` | `path` | no | string(path) | discovered config | RustClaw config file to scan. |

## Boundaries
- Use `config_basic` for fields, keys, safe structure summaries, and parse validation instead of broad whole-file reads.
- For empty-field counts, disabled boolean inventories, or similar structural summaries, use `summarize_structure`; scope it with `field_path` when the requested count belongs to one subtree. Do not read raw config ranges and manually count values.
- For RustClaw main-config safety checks, call `config_basic` with `action="guard_rustclaw_config"` directly. Omit `path` unless the user supplied an explicit config file; do not search or list directories first just to find the default config.
- When a complete validation or guard action is available, do not replace it with bounded raw reads; those may gather supplementary evidence only after the validator reports a structured gap.
- `validate` proves parse/schema syntax only. RustClaw semantic safety, risk, or problem checks must use `guard_rustclaw_config` directly; do not approximate them with raw file reads.
- Field paths support dot/bracket selectors. For arrays of objects, both `<item-name>.<field>` and `<array-path>.<item-name>.<field>` may resolve the unique object whose `name`, `id`, or `key` equals `<item-name>` before reading `<field>`.
- `read_fields.field_paths` must contain a non-empty selector. After `list_keys`, pass relevant key tokens to `read_fields`, or inspect a nested object with `list_keys.field_path` first.
- Do not plan `patch_field`, `write`, `set`, or other generic config mutation through `config_basic` in v1.
- Confirmed structured config edits should use `config_edit` when available, followed by validation, guard checks, and read-back. Use broad file or command workflows only when the requested mutation cannot be represented as a config field path and typed value.
- `config_guard` remains the backing RustClaw safety scanner, not a general editor.
- For non-structured files, use `fs_basic`, raw file tools, or `run_cmd`.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- 中文里的“配置项/字段/key/开关/版本号/模型名”要按结构化字段任务理解，优先产出 `field_path` 或 `field_paths`，不要读完整配置后再猜。
- 如果用户要求修改配置，优先让 `config_edit` 处理结构化字段变更；本工具继续负责读取字段、解析校验和安全检查。
