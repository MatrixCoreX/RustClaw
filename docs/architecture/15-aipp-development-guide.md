# AiAPP Development Guide

<!-- ai-learning-stage: capabilities-artifacts -->
<!-- ai-learning-audience: developer -->

<!-- ai-learning-navigation:start -->
Previous: [AiPP skill companion interfaces](14-aipp-skill-companions.md) |
[Architecture index](README.md)
<!-- ai-learning-navigation:end -->

An AiAPP is an optional visual companion declared and owned by one skill package. It presents
structured skill data or invokes declared capabilities, but it does not create a second execution
path. Installing, updating, disabling, or removing an ordinary AiAPP must not change, rebuild, or
restart `clawd` or the main UI.

## Choose One Integration Mode

Use a reviewed host renderer when an existing contract already represents the data:

| Renderer | Data contract | Intended source |
| --- | --- | --- |
| `collection_feed_v1` | `media_collection_v1` | Bounded records in the declaring skill's private storage |
| `task_activity_v1` | `skill_task_activity_v1` | Runtime tasks selected by structured `tool_finished.payload.skill` events |

Use `sandbox_bundle_v1` with `capability_bridge_v1` when the skill needs a custom interface. The
skill package owns and independently builds the frontend. The main UI only hosts the immutable
static bundle in a sandbox and forwards allowlisted capability requests through the normal runtime
resolver and verifier.

Do not add a skill name, skill-specific route, skill-specific import, or custom response parser to
`clawd` or `UI/src/components/AippPage.tsx`. A new reusable host contract is a separately reviewed
platform change, not part of an ordinary app installation.

## Package Layout

A host-rendered app needs only its normal skill files:

```text
optional_skills/example/
  skill.toml
  INTERFACE.md
  runtime/
```

A custom app keeps source and dependencies inside the skill package and emits static files under
`aipp/`:

```text
optional_skills/example/
  skill.toml
  INTERFACE.md
  runtime/
  aipp-src/          # optional source; built only by this skill's package workflow
  aipp/
    index.html
    app.js
    app.css
```

The normal main-UI build must never enter `aipp-src/`. The package build or release workflow must
produce `aipp/` before admission. Every served bundle asset is included in the immutable package
receipt and verified again before delivery.

## Manifest Examples

Task activity, including Agent UI and communication channels:

```toml
[aipp]
schema_version = 1
renderer = "task_activity_v1"
data_contract = "skill_task_activity_v1"
task_channel_scope = "all"
icon = "download"
default_locale = "en"
titles = { en = "Example Activity", zh = "示例活动" }
descriptions = { en = "Review completed work.", zh = "查看已完成的工作。" }
```

Set `task_channel_scope = "communication"` only when the app deliberately excludes Agent UI
tasks. Selection is based on structured execution events, never request or response prose.

Custom sandbox bundle:

```toml
[aipp]
schema_version = 1
renderer = "sandbox_bundle_v1"
data_contract = "capability_bridge_v1"
asset_root = "aipp"
entrypoint = "aipp/index.html"
bridge_capabilities = ["example.status", "example.list"]
icon = "panels_top_left"
default_locale = "en"
titles = { en = "Example", zh = "示例" }
descriptions = { en = "Review example data.", zh = "查看示例数据。" }
```

Every bridge capability must also be declared under `capability_request.capabilities`. A manifest
request does not grant permission; admission, host policy, enable state, and the pinned registry
generation still control execution.

## Browser Bridge

The bundle runs in an iframe with scripts and downloads enabled but without same-origin access. It
does not receive cookies, API keys, credentials, parent DOM access, unrestricted network access, or
raw filesystem paths. It sends versioned messages:

```json
{"schema_version":1,"type":"aipp.ready"}
{"schema_version":1,"type":"aipp.capability.invoke","request_id":"status-1","capability":"example.status","args":{}}
```

The host validates the source window, message shape, serialized argument bound, concurrent request
bound, and manifest allowlist. Results return through `aipp.host.context` and
`aipp.capability.result`. The app renders localized copy from structured fields and must not parse
model prose to decide state, ownership, success, retry, or permission.

## Data Ownership

- A collection renderer reads only the declaring skill's resolved private storage.
- A task-activity renderer reads only tasks with a structured execution event for the declaring
  skill. `task_channel_scope = "all"` includes UI and external channels.
- Preview and artifact routes return authenticated, bounded, allowlisted files. They never expose
  private storage paths.
- AiAPP removal writes a presentation tombstone only. It does not uninstall or disable the skill
  and does not delete configuration or private data.
- Skill removal follows the ordinary admission lifecycle; in-flight calls finish against their
  pinned version lease.

## Installation Lifecycle

1. Validate `skill.toml`, capability requests, package paths, and static assets.
2. Build the skill and optional custom frontend in the skill-owned build workflow.
3. Run protocol smoke and verify every package artifact digest.
4. Write an immutable package receipt and host policy grant.
5. Commit one overlay generation containing skill and AiAPP state.
6. Enable the skill explicitly. AiAPP availability follows the exact active package binding.
7. Install or remove the AiAPP presentation independently through its overlay tombstone.

No step modifies tracked source, the root Cargo workspace for external skills, the main UI bundle,
or a running `clawd` process.

## Required Verification

Run these checks before delivery:

```bash
python3 scripts/check_aipp_decoupling.py --self-test
python3 scripts/check_aipp_decoupling.py
python3 scripts/sync_skill_docs.py
python3 scripts/check_skill_prompts.py
target/debug/skillctl validate optional_skills/example/skill.toml
cargo test -p agent-skill-sdk aipp_manifest --no-fail-fast
cargo test -p clawd aipp_tests --no-fail-fast
```

For a host-rendered app, also test cursor pagination, filters, authorization, field bounds, secret
redaction, app-only removal/reinstallation, skill disable/re-enable, and zero cross-skill records.
For a sandbox bundle, also test receipt tampering, path traversal, symlinks, unsupported types,
content-security headers, bridge allowlist rejection, argument limits, and concurrent request limits.
Build and test the main UI only when a generic host renderer or bridge itself changed; adding an
ordinary package-owned AiAPP must not require that build.

When a generic manifest or host contract changes, rebuild and deploy every binary that consumes
`agent-skill-sdk`, including `clawd`, `skill-runner`, and `skillctl`, as one release. Verify one
installed-skill protocol invocation after deployment. Optional additive manifest fields need a safe
runtime default for already installed immutable packages, while newly built source manifests remain
subject to the current ratchet. A partial host/runner deployment is invalid even when startup
succeeds.

## Review Checklist

- The app is declared only in its owning `skill.toml`.
- No concrete skill name appears in generic AiAPP host routing or rendering code.
- No skill frontend source is imported by the main UI.
- Install/update/remove does not rebuild or restart the host.
- Runtime decisions use stable machine fields, not natural-language matching.
- Package assets, prompts, grants, and versions are bound to one immutable receipt/generation.
- UI copy is localized inside the package or rendered through existing locale fields.
- The app exposes only the minimum structured data and capabilities required for its task.
