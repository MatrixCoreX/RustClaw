<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `media_discovery` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `optional_skills/media_discovery/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
Run bounded browser collection for Douyin and Xiaohongshu. The default
`browser_mode=visible` opens a browser window; `browser_mode=silent` is allowed
only when the user explicitly requests no window. The skill screenshots media
elements already rendered in the browser, recognizes visible
text, and exports exactly two user result files: `videos.csv` and `images.csv`.
For each video it also preserves the first stable frame observed in the rendered
browser element under `video_covers/` and records that relative path in the CSV.
It does not download video binaries or original image files.

Keyword discovery uses one canonical structured input:
`source_mode=topics` with non-empty `topics[]`. The skill opens the selected
platform's search result for each keyword in input order, browses bounded
result candidates in one browser session, and records the keyword and search
page URL on every committed result. No localized search phrase is parsed by
runtime or skill code.

A continuous start request is a structured workflow: call `enable`, copy its
returned `schedule_spec.args.intent_json` into `schedule.create_structured` in
the parent loop, then call the no-argument `run_enabled_once` for the initial
batch. A stop request calls
`disable`, then passes the returned `schedule_cleanup_spec.args` to the required
companion capability `schedule.delete_matching`. Runtime does not permit a
terminal reply until that structured cleanup capability succeeds.

Only one collection batch may own the skill lease at a time. A second start or
`run_once` returns structured `run_already_active` instead of opening another
browser. While any continuous platform remains enabled, another `enable`
returns `collection_already_enabled`; this also rejects a duplicate start that
was submitted during a run but reached the skill after that batch finished.
Both rejections are pre-dispatch failures with no side effect. Disabling a
platform atomically prevents future batches and changes a
matching active run to `draining`: the skill finishes the current post, captures
all of its rendered carousel images, commits its records and CSV rows, and then
closes the browser normally. It never kills the browser to implement this stop.

This is an on-demand companion skill with private storage and its own dispatch
queue. It must not block `media_download` manual downloads or OCR work.

Continuous background batches emit one machine-only heartbeat every 15 minutes
while they remain active. The frame uses
`detail_key=media_discovery.background.status` with elapsed time and current
item/video/image/duplicate/failure counts. Runtime persists the frame for UI
task events and projects the same structured snapshot to the originating
communication channel through the unified, idempotent delivery service. The
skill never writes localized notification prose. Explicit one-shot collection
does not enable these periodic notices.

## Config Entry Points (from interface)
- No dedicated config entry points declared.

## Actions (from interface)
- `capabilities`: report GUI, Chromium, capture mode, and supported platforms.
- `preview_enable`: validate settings without changing state.
- `enable`: persist per-platform enabled state and return a structured schedule
  specification. It does not create a hidden child process.
- `disable`: disable selected platforms, gracefully drain any matching active
  batch after its current post, and return the exact required structured
  schedule-cleanup call.
- `run_once`: with explicit platform/source settings, run one ephemeral bounded
  batch without enabling continuous collection; scheduler calls marked with
  `scheduled_run=true` run only enabled, non-paused platforms. A fresh active
  lease rejects either form with `run_already_active`.
- `run_enabled_once`: internal no-argument companion used after `enable`; run
  one batch from enabled, non-paused persisted configurations without asking
  the model to reproduce nested configuration values.
- `status`: return platform state, active run, and result counts.
- `pause` / `resume`: preserve configuration while controlling future batches.
- `stop_current`: request a graceful stop after the current complete post,
  optionally restricted to a platform, without disabling future schedules.
- `list_runs`: return paginated recent batch records.
- `export_results`: deliver rebuilt `videos.csv` and `images.csv` artifacts.
  Persisted browser video-cover screenshots are copied beside them under
  `video_covers/` and returned as image artifacts.

## Parameter Contract (from interface)
| Param | Required | Description |
|---|---:|---|
| `action` | yes | One action listed below. |
| `platform` or `platforms` | enable/preview | `douyin` and/or `xiaohongshu`. |
| `source_mode` | no | `home_feed` (default), `topics`, or `seed_urls`. |
| `topics` | for topics | One or more exact search keywords, browsed in input order. |
| `seed_urls` | for seed_urls | HTTPS URLs on the selected platform only. |
| `max_items_per_run` | no | 1..100, default 20. |
| `max_images_per_post` | no | 1..100, default 100. The adapter follows rendered carousel controls and stops at the actual end or this safety ceiling. |
| `max_run_minutes` | no | 5..180, default 30. |
| `max_scrolls_per_source` | no | 1..100, default 10. |
| `interval_minutes` | no | 10..1440, default 60. |
| `recognition_mode` | no | `ocr_reviewed` (default), `local_ocr`, or `metadata_only`. |
| `browser_mode` | no | `visible` (default), or `silent` only after an explicit user request. |
| `pacing_min_delay_ms` | no | Lower interaction-delay bound, 200..5000, default 700. |
| `pacing_max_delay_ms` | no | Upper interaction-delay bound, 200..8000, default 1800 and never below the minimum. |
| `confirm` | enable | Must be true after runtime approval. |

`scheduled_run` is an internal scheduler marker emitted only inside
`enable.extra.schedule_spec`; it is not a planner or user parameter.

## Error Contract (from interface)
Errors use `extra.{schema_version,source_skill,status,error_code,message_key,retryable}`.
Stable examples include `display_unavailable`, `browser_missing`,
`login_required`, `challenge_required`, `rate_limited`, `selector_drift`,
`platform_unsupported`, `source_scope_empty`, `run_already_active`,
`collection_already_enabled`, and `storage_lock_timeout`.
`error_text` is a human fallback and must never drive routing or retry logic.

## Request/Response Examples (from interface)
```json
{"action":"enable","platform":"douyin","source_mode":"home_feed","interval_minutes":60,"confirm":true}
```

```json
{"action":"enable","platform":"xiaohongshu","source_mode":"topics","topics":["AI agent","机器人"],"browser_mode":"visible","confirm":true}
```

```json
{"action":"run_once","platform":"douyin","source_mode":"topics","topics":["AI agent"],"browser_mode":"silent","max_items_per_run":5}
```

```json
{"action":"disable","platform":"douyin"}
```

```json
{"action":"export_results"}
```

An exported video row may contain:

```json
{"source_mode":"topics","search_keyword":"AI agent","discovery_source_url":"https://www.douyin.com/search/AI%20agent","cover_screenshot_path":"video_covers/douyin_123.png","video_page_url":"https://www.douyin.com/video/123"}
```

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
- For setup or configuration questions about this skill, treat the config entry points section as the grounding source for where changes actually live.

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
- Interpret Chinese colloquial phrasing by capability semantics and requested task shape, not by a fixed phrase list.
- Judge Chinese delivery intent semantically: if the user asks to receive a file/result rather than inline body text, plan toward delivery without depending on fixed wording.
- Preserve Chinese brevity and format constraints as final output contracts when the skill can support them; do not convert those constraints into token-level matching rules.
- Treat Chinese style constraints as audience/tone constraints for the eventual explanation, not as skill-selection shortcuts.
- Resolve Chinese deictic references only from immediate, concrete, type-compatible context; do not guess unsupported targets or invent missing args just to force a skill call.
