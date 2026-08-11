# media_discovery Interface

## Capability Summary

Run bounded browser collection for Douyin and Xiaohongshu. The default
`browser_mode=silent` runs without a browser window; `browser_mode=visible`
opens one only when the user's request requires visible or non-silent browsing. The skill screenshots media
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

## Planner Workflow

- A user request to start continuous collection is a multi-capability workflow:
  1. call `media_discovery.enable` with the requested platform(s), bounded
     settings, and `confirm=true` after policy approval;
  2. copy the returned `schedule_spec.args.intent_json` string unchanged into
     `schedule.create_structured`; the runtime validates its nested schedule
     and task types before creating the durable job;
  3. call the no-argument `media_discovery.run_enabled_once` immediately. It
     reads only enabled persisted platform configurations.
- A request to stop one platform first calls `media_discovery.disable` for that
  platform. Then call the required companion `schedule.delete_matching` with
  the exact returned `schedule_cleanup_spec.args`. Matching uses task payload
  fields, never a human-readable job description. A shared multi-platform job
  is retained while it still serves a platform that was not disabled.
- A user request for one bounded batch without continuous collection calls
  `run_once` with explicit platform and source settings. The skill uses an
  ephemeral config and does not enable or schedule the platform. The
  `scheduled_run=true` marker is reserved for the returned `schedule_spec` and
  must never be invented by the planner.
- `disable` also requests a graceful drain of a matching active batch. Report
  the returned `lifecycle_state`, `drain_run_id`, and `stop_mode` rather than
  claiming an immediate process termination.
- Do not ask for a topic or URL when the user clearly selected a platform but
  supplied neither: use `source_mode=home_feed`. Never infer a different
  platform.
- When the user asks to search one or more keywords before collecting, pass
  `source_mode=topics` and place those exact search terms in `topics[]`. Do not
  invent a second keyword parameter or translate the terms unless requested.
- Omit `browser_mode` or pass `silent` by default. Pass `visible` only when the
  user explicitly requests a browser window or non-silent operation. Runtime must consume this enum and must
  not match localized words to select a mode.
- Browsing uses bounded randomized pauses and scroll distances to avoid bursty
  traffic. This is cooperative pacing, not fingerprint spoofing, challenge
  bypass, or a guarantee against platform controls. Login, challenge, and rate
  limit states stop the current batch and remain machine-visible.
- These rules are semantic model guidance. Production runtime and skill code
  must not match fixed Chinese, English, or other-language phrases.

Examples of equivalent intent (documentation examples, not runtime matchers):

- `帮我开始采集抖音` -> enable Douyin home feed, schedule bounded batches, run
  the first batch.
- `停止采集抖音` -> disable Douyin and remove its structured schedule jobs.
- `Start collecting Xiaohongshu posts` -> the same workflow for Xiaohongshu.
- `搜索露营装备并采集小红书内容` -> use `source_mode=topics` and
  `topics=["露营装备"]` for Xiaohongshu.
- `Arrête la collecte de Xiaohongshu` -> disable only Xiaohongshu.

## Parameter Contract

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
| `browser_mode` | no | `silent` (default), or `visible` after an explicit visible/non-silent request. |
| `pacing_min_delay_ms` | no | Lower interaction-delay bound, 200..5000, default 700. |
| `pacing_max_delay_ms` | no | Upper interaction-delay bound, 200..8000, default 1800 and never below the minimum. |
| `confirm` | enable | Must be true after runtime approval. |

`scheduled_run` is an internal scheduler marker emitted only inside
`enable.extra.schedule_spec`; it is not a planner or user parameter.

## Actions

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

## Output Contract

Every response has an empty `text` and structured
`extra.{schema_version,source_skill,status,action}` so the main model can answer
in the user's language. Errors additionally provide
`extra.{error_code,message_key,retryable}`; runtime logic must not parse
`error_text`.

When `run_enabled_once` or a scheduler-marked `run_once` remains active for at
least 15 minutes, zero or more `skill_progress` JSONL records precede the final
response. Their `params.notification_delivery=runtime` marker delegates UI and
channel presentation to the host; it does not change success, retry, routing,
or final-result semantics. The host enforces a minimum 900-second delivery
interval and deduplicates each delivery by task and frame sequence.

`videos.csv` columns:

`sequence,global_sequence,platform,browser_mode,source_mode,search_keyword,discovery_source_url,title,platform_text,recognized_text,cover_screenshot_path,video_page_url,discovered_at`

`images.csv` columns:

`sequence,global_sequence,post_sequence,image_sequence,platform,browser_mode,source_mode,search_keyword,discovery_source_url,title,platform_text,recognized_text,image_url,source_page_url,discovered_at`

CSV files use UTF-8 BOM, RFC 4180 quoting, stable order, and spreadsheet formula
injection protection. The private immutable record ledger remains the recovery
source of truth; CSV files can always be rebuilt.

## Browser and Recognition Rules

- Browser mode defaults to silent. `visible` is accepted only as an explicit
  structured planner argument; when selected, a missing desktop session returns
  `display_unavailable` instead of changing the requested mode.
- The skill uses a private persistent browser profile. It does not read cookies
  from unrelated browser profiles or write them to logs/checkpoints.
- Recognition uses screenshots of browser-rendered media elements. It does not
  issue additional requests for original images and does not present this as a
  mechanism for bypassing anti-automation controls.
- The first stable video frame observed after navigation is retained as a cover
  screenshot. If the page already autoplayed, this is not represented as the
  encoded timeline's exact frame zero. Duplicate items never replace an
  existing persisted cover.
- Tesseract produces the raw text using all installed recognition language
  data without preferring one writing system. In `ocr_reviewed` mode, the
  complete text is split into Unicode-safe bounded chunks and sent through the
  host-scoped internal LLM gateway to restore layout, punctuation, and highly
  certain OCR errors without translation, summary, or invention. Visual soft
  wraps caused only by image width are merged, while real paragraphs, headings,
  lists, tables, code, verse, and other line-oriented structures retain their
  boundaries. Every chunk
  must succeed, and the reviewed result must preserve numeric tokens and a
  bounded amount of source content; otherwise the complete raw OCR text is
  retained. Immutable records keep both `raw_recognized_text` and the selected
  `recognized_text`.
- Login challenges, access denial, and rate limiting produce structured waiting
  or failure states. The skill never bypasses them.

## Error Contract

Errors use `extra.{schema_version,source_skill,status,error_code,message_key,retryable}`.
Stable examples include `display_unavailable`, `browser_missing`,
`login_required`, `challenge_required`, `rate_limited`, `selector_drift`,
`platform_unsupported`, `source_scope_empty`, `run_already_active`,
`collection_already_enabled`, and `storage_lock_timeout`.
`error_text` is a human fallback and must never drive routing or retry logic.

## Request/Response Examples

```json
{"action":"enable","platform":"douyin","source_mode":"home_feed","interval_minutes":60,"confirm":true}
```

```json
{"action":"enable","platform":"xiaohongshu","source_mode":"topics","topics":["AI agent","机器人"],"browser_mode":"visible","confirm":true}
```

```json
{"action":"run_once","platform":"douyin","source_mode":"topics","topics":["AI agent"],"max_items_per_run":5}
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
