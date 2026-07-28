# extension_manager Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with `crates/skills/extension_manager/src/main.rs`.

## Capability Summary

- `extension_manager` is a guarded developer-facing skill for capability-gap assessment, bounded temporary fixes, and manifest-driven external skill lifecycle management.
- Permanent packages stay unregistered while scaffolded and validated. Registration is allowed only after the selected adapter builds in private staging and the JSONL protocol smoke test produces a trusted install receipt.
- External scaffolds may select Rust/Cargo, Python, Node, Go, or prebuilt explicitly. Source generation supports Cargo, Python, Node, and Go; prebuilt, generic-process, and HTTP packages require developer-supplied artifacts or configuration.
- Registration writes `package_manifest` registry metadata and `skill_switches.<skill>=true` atomically. Only Cargo packages may require Cargo-workspace membership; non-Cargo packages do not edit the workspace.

## Actions

- `assess_gap`: recommend a temporary fix, permanent extension, or manual review.
- `temporary_fix_plan`: produce a bounded one-off script/package plan without registering a skill.
- `temporary_fix_execute`: execute such a plan only after explicit confirmation.
- `permanent_extension_plan`: turn a reusable request into a scaffold-ready skill plan.
- `scaffold_external_skill`: create `external_skills/<skill_name>` in an explicitly selected implementation language.
- `implement_external_skill`: replace untouched scaffold docs and the manifest-selected source entrypoint with a first implementation.
- `validate_external_skill`: synchronize docs, validate the manifest, install with its private adapter in staging, and run the protocol smoke test.
- `register_external_skill`: validate and install the verified package, then atomically add registry/config metadata and enable it.
- `enable_external_skill`: reinstall an already registered external package from its manifest and enable its config switch.

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
|---|---|---:|---|---|---|
| `assess_gap` | `request` | yes | string | - | Missing capability or task. |
| `assess_gap` | `mode_hint` | no | enum | `auto` | `auto`, `temporary_fix`, `permanent_extension`, or `manual_review`; `auto` remains conservative. |
| `temporary_fix_plan` | `request` | yes | string | - | Request for a bounded one-off plan. |
| `temporary_fix_execute` | `confirm` | yes | bool | - | Must be `true`. |
| `temporary_fix_execute` | `plan` | conditional | object | - | Previously generated plan; required unless `request` is supplied. |
| `temporary_fix_execute` | `request` | conditional | string | - | Generate and execute a plan in one call. |
| `temporary_fix_execute` | `allow_package_install` | no | bool | `false` | Separately authorizes temporary language-level package installation. |
| `permanent_extension_plan` | `request` | yes | string | - | Reusable capability request. |
| `scaffold_external_skill` | `skill_name` | yes | snake_case string | - | New external skill name. |
| `scaffold_external_skill` | `capability_summary` | yes | string | - | Short reusable capability summary. |
| `scaffold_external_skill` | `actions` | no | string or string[] | `["todo_action"]` | Initial action names. |
| `scaffold_external_skill` | `implementation_language` | no | enum | `rust` | `rust`, `python`, `node`, `go`, or `prebuilt`; aliases accepted by the SDK parser. |
| `scaffold_external_skill` | `build_adapter` | no | enum | - | Compatibility input for the same language selection when `implementation_language` is absent. |
| `implement_external_skill` | `request` | yes | string | - | Original reusable request used for generation. |
| `implement_external_skill` | `skill_name` | yes | snake_case string | - | Existing scaffold name. |
| `implement_external_skill` | `capability_summary` | yes | string | - | Contract summary. |
| `implement_external_skill` | `actions` | no | string or string[] | `["todo_action"]` | Actions the generated source must support. |
| `validate_external_skill` | `skill_name` | yes | snake_case string | - | Existing scaffold name. |
| `validate_external_skill` | `actions` | no | string or string[] | `["todo_action"]` | Smoke-test request actions. |
| `register_external_skill` | `skill_name` | yes | snake_case string | - | Validated external package. |
| `register_external_skill` | `confirm` | yes | bool | - | Must be `true` before installation and metadata writes. |
| `register_external_skill` | `actions` | no | string or string[] | `["todo_action"]` | Validation smoke actions. |
| `enable_external_skill` | `skill_name` | yes | snake_case string | - | Existing registered external package. |
| `enable_external_skill` | `confirm` | yes | bool | - | Must be `true` before installation/config writes. |

Every action also accepts `action`; it defaults to `assess_gap` only when omitted.

## Error Contract

- Shape/confirmation: `args must be object`, `<key> is required`, `<action> requires confirm=true`, `actions must be a string or string array`.
- Identity: `invalid skill_name: ...`, `skill directory already exists: ...`, `skill scaffold does not exist yet: ...`, `external skill identity mismatch: ...`.
- Source generation: `refusing to overwrite non-scaffold file: ...`; adapters without a source scaffold return `implement_external_skill requires developer-supplied artifacts for adapter=<adapter>`.
- Validation/install: failures include structured phase, code, and detail from manifest validation, adapter build, protocol smoke, or verified receipt resolution. Package builds use private dependency/cache roots and `allow_network=false` by default.
- Registration/config writes roll back the newly installed package or prior metadata when a later atomic step fails.
- Temporary fixes are separate from permanent packaging. Package installation requires both `confirm=true` and `allow_package_install=true`; command failures include runtime, script, exit code, and bounded stderr.
- Malformed stdin returns `status="error"`, readable `error_text`, and structured `extra.error_code`/`message_key`.

## Config Entry Points

- Text generation normally uses the internal LLM gateway and the system `[llm].selected_vendor` / `selected_model`.
- Standalone fallback accepts `OPENAI_BASE_URL`, `OPENAI_API_KEY`, `OPENAI_MODEL`, and optional `EXTENSION_MANAGER_MODEL`.
- Permanent build/run behavior comes only from `external_skills/<skill>/skill.toml`; registry registration stores its `package_manifest` path and does not derive an executor from prose or file extensions.
- Verified packages and receipts live under the runtime skill-package root. Disabling/uninstalling preserves skill-owned data by default.

## Request/Response Examples

### Scaffold a Python package

Request:

```json
{"request_id":"demo-1","context":null,"user_id":1,"chat_id":1,"args":{"action":"scaffold_external_skill","skill_name":"text_probe","capability_summary":"Inspect text locally.","actions":["inspect"],"implementation_language":"python"}}
```

Response:

```json
{"request_id":"demo-1","status":"ok","text":"Scaffolded external skill `text_probe` at external_skills/text_probe. It is not registered or enabled.","extra":{"action":"scaffold_external_skill","skill_name":"text_probe","implementation_language":"python","build_adapter":"python","default_enabled":false},"error_text":null}
```

### Validate the manifest-selected adapter

Request:

```json
{"request_id":"demo-2","context":null,"user_id":1,"chat_id":1,"args":{"action":"validate_external_skill","skill_name":"text_probe","actions":["inspect"]}}
```

Response:

```json
{"request_id":"demo-2","status":"ok","text":"Validated external_skills/text_probe: manifest, adapter build, and protocol smoke test passed.","extra":{"action":"validate_external_skill","skill_name":"text_probe","report":{"synced_docs":true,"manifest_valid":true,"adapter":"python","build_ok":true,"smoke_test_ok":true,"smoke_status":"ok","receipt_digest":"sha256:..."},"default_enabled":false},"error_text":null}
```

### Register after verification

Request:

```json
{"request_id":"demo-3","context":null,"user_id":1,"chat_id":1,"args":{"action":"register_external_skill","skill_name":"text_probe","actions":["inspect"],"confirm":true}}
```

Response:

```json
{"request_id":"demo-3","status":"ok","text":"Registered external skill `text_probe`, installed its verified package, and enabled it in config. Reload skills or restart clawd before using it.","extra":{"action":"register_external_skill","skill_name":"text_probe","default_enabled":true,"install_ok":true,"adapter":"python","receipt_digest":"sha256:...","reload_required":true},"error_text":null}
```
