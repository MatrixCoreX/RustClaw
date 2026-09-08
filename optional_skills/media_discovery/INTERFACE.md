# media_discovery Interface

## Capability Summary

Run explicitly requested batch, feed, keyword, or continuous background
browser collection for Douyin, Xiaohongshu, and Kuaishou. A lone copied share payload or
URL whose content should be downloaded and returned now belongs to
`media_download.download`, even when it is used as a `seed_urls` input shape;
this skill does not provide immediate single-post media delivery. The default
`browser_mode=silent` runs without a browser window; `browser_mode=visible`
opens one only when the user's request requires visible or non-silent browsing. The skill screenshots media
elements already rendered in the browser and exports exactly two user result
files: `videos.csv` and `images.csv`. It never runs OCR or model text review;
author-provided captions remain available as `platform_text`.
It also records the engagement counters exposed by platform-owned machine DOM
controls at collection time. Metrics are structured as views, likes, comments,
favorites, and shares; only counters available on the current platform/page are
present. A counter must contain at least one Unicode decimal digit. The
platform-rendered display value is retained without matching localized units,
and an exact numeric value is added only for plain integer forms.
For each video it attempts to preserve the first stable frame from a visible,
unobscured platform video element, then a platform-specific rendered poster.
Successful covers are stored under `video_covers/` and referenced from the CSV.
Platforms do not guarantee either element: login/challenge overlays, selector
changes, and unavailable media may therefore leave a result without a cover.
The skill never substitutes a whole-page or login-dialog screenshot. It does
not download video binaries or original image files.

Keyword discovery uses one canonical structured input:
`source_mode=topics` with non-empty `topics[]`. The skill opens the selected
platform's search result for each keyword in input order, browses bounded
result candidates in one browser session, and records the keyword and search
page URL on every committed result. No localized search phrase is parsed by
runtime or skill code.
Explicit detail `seed_urls` are collected as the exact requested set and do not
expand into unrelated recommendation links from those pages.

Douyin `home_feed` collection opens the first visible recommendation as one
detail video through its HTTPS URL before collecting anything. Card-owned
desktop-app launch handlers are never clicked. It captures the current detail
item, uses the player's next-item control (or scrolls a rendered feed), waits for
the active item identity to change, and repeats one item at a time. It does not scrape a batch of cards
directly from the recommendation landing page. Image-carousel posts encountered
in the detail feed are completed before advancing to the next item.

A continuous start request is a two-step structured workflow: call `enable`,
then call its no-argument companion `run_enabled_once`. The companion is a
runtime-owned durable background job that repeatedly browses enabled sources,
rests for a bounded random period, and continues until disabled or cancelled.
It does not create or depend on a schedule job. A stop request calls `disable`;
the worker observes the persisted control state and exits after the current
complete post is committed.

Only one background worker and one collection batch may own their respective
skill leases at a time. A second start or
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
queue. It must not block `media_download` manual downloads or explicit image
recognition work handled by another skill.

The package declares an `AiPP` companion using the host-owned
`collection_feed_v1` renderer and `media_collection_v1` read contract. When the
skill is installed, enabled, and bound to the current immutable registry
generation, administrators can review its records and rendered video covers on
the AiPP page. The companion is removed from the catalog when the skill is
disabled or uninstalled; retained private data remains governed by this
package's storage policy. The package supplies no browser-executable UI code.

The continuous background worker emits one machine-only heartbeat every 15 minutes
while they remain active. The frame uses
`detail_key=media_discovery.background.status` with elapsed time and current
item/video/image/duplicate/failure counts. Runtime persists the frame for UI
task events and projects the same structured snapshot to the originating
communication channel through the unified, idempotent delivery service. The
skill never writes localized notification prose. Explicit one-shot collection
does not enable these periodic notices.

## Planner Selection Notes

- Select this skill only when the current request explicitly asks for a
  collection workflow: batch browsing, home-feed browsing, keyword discovery,
  continuous collection, collection lifecycle control, or CSV
  export. Do not select `run_once` for one copied share or URL that should be
  downloaded and returned to the user now; select `media_download.download`.
- A user request to start continuous collection is a multi-capability workflow:
  1. call `media_discovery.enable` with the requested platform(s), bounded
     settings, and `confirm=true` after policy approval;
  2. call the no-argument `media_discovery.run_enabled_once` immediately. It
     reads only enabled persisted platform configurations and remains active as
     a durable background job.
- A request to stop one or all platforms calls `media_discovery.disable`. The
  worker finishes the current post before exiting; no schedule cleanup is
  involved.
- A user request for one bounded batch without continuous collection calls
  `run_once` with explicit platform and source settings. The skill uses an
  ephemeral config and does not enable the platform or start a background
  worker.
- Treat `state=completed_batch` and `run.counts` as the completed batch receipt.
  Do not start another batch to verify success or after exporting results.
  Use `status` or `list_runs` for verification. Call `export_results` only when
  the user requests exported files; saving content already makes it available
  in AiAPP. An async job must be polled through its returned runtime handle,
  not started again with altered pacing or other arguments.
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
- Browsing uses bounded randomized pauses, scroll distances, and inter-batch
  rests to avoid bursty
  traffic. This is cooperative pacing, not fingerprint spoofing, challenge
  bypass, or a guarantee against platform controls. Login, challenge, and rate
  limit states stop the current batch and remain machine-visible.
- These rules are semantic model guidance. Production runtime and skill code
  must not match fixed Chinese, English, or other-language phrases.

Examples of equivalent intent (documentation examples, not runtime matchers):

- `帮我开始采集抖音` -> enable Douyin home feed and start its durable
  background worker.
- `停止采集抖音` -> disable Douyin; the worker drains the current post and exits.
- `Start collecting Xiaohongshu posts` -> the same workflow for Xiaohongshu.
- `Collect a small Kuaishou recommendation batch` -> run one bounded Kuaishou
  `home_feed` batch without enabling background collection.
- `搜索露营装备并采集小红书内容` -> use `source_mode=topics` and
  `topics=["露营装备"]` for Xiaohongshu.
- `Arrête la collecte de Xiaohongshu` -> disable only Xiaohongshu.

## Parameter Contract

| Param | Required | Description |
|---|---:|---|
| `action` | yes | One action listed below. |
| `platform` or `platforms` | enable/preview | `douyin`, `xiaohongshu`, and/or `kuaishou`. |
| `source_mode` | no | `home_feed` (default), `topics`, or `seed_urls`. |
| `topics` | for topics | One or more exact search keywords, browsed in input order. |
| `seed_urls` | for seed_urls | HTTPS URLs on the selected platform only. |
| `max_items_per_run` | no | 1..100, default 5. |
| `max_images_per_post` | no | 1..100, default 100. The adapter follows rendered carousel controls and stops at the actual end or this safety ceiling. |
| `max_run_minutes` | no | 5..180, default 30. |
| `max_scrolls_per_source` | no | 1..100, default 10. |
| `rest_min_seconds` | no | Minimum random rest between continuous batches, 5..3600, default 180. |
| `rest_max_seconds` | no | Maximum random rest between continuous batches, 5..7200, default 420 and never below the minimum. |
| `browser_mode` | no | `silent` (default), or `visible` after an explicit visible/non-silent request. Browser visibility is a user-selected execution constraint: every planner action that accepts this field must emit `visible` when visibility was requested, while omission is valid only when the user expressed no browser-mode preference. |
| `pacing_min_delay_ms` | no | Lower interaction-delay bound, 200..5000, default 1000. |
| `pacing_max_delay_ms` | no | Upper interaction-delay bound, 200..8000, default 2800 and never below the minimum. |
| `confirm` | enable/clear_results | Must be true after runtime approval. |

## Actions

- `capabilities`: report GUI, Chromium, capture mode, and supported platforms.
- `preview_enable`: validate settings without changing state.
- `enable`: persist per-platform enabled state and return the exact durable
  background companion capability.
- `disable`: disable selected platforms, gracefully drain any matching active
  batch after its current post, and make the background worker exit before its
  next batch.
- `run_once`: with explicit platform/source settings, run one ephemeral bounded
  batch without enabling continuous collection. A fresh active lease rejects
  it with `run_already_active`.
- `run_enabled_once`: no-argument durable companion used after `enable`; keep
  running bounded collection batches with randomized rests until `disable` or
  task cancellation.
- `status`: return platform state, live background worker, live active batch,
  `latest_run`, and result counts. Expired heartbeat records appear only in
  `expired_leases` with `lifecycle_state=heartbeat_expired`; they are not proof
  of a running worker. This read does not erase or rewrite stored history.
- `pause` / `resume`: preserve configuration while pausing/resuming the worker.
- `stop_current`: request a graceful stop after the current complete post,
  optionally restricted to a platform, without disabling the background worker.
- `list_runs`: return paginated recent batch records.
- `export_results`: deliver rebuilt `videos.csv` and `images.csv` artifacts.
  Persisted browser video-cover screenshots are copied beside them under
  `video_covers/` and returned as image artifacts.
- `clear_results`: after explicit confirmation, delete collected records,
  exports, diagnostics, stale temporary files, and run history. It refuses to
  run during a live batch and preserves platform configuration plus the private
  browser profile/login state.

## Output Contract

Every response has an empty `text` and structured
`extra.{schema_version,source_skill,status,action}` so the main model can answer
in the user's language. Errors additionally provide
`extra.{error_code,message_key,retryable}`; runtime logic must not parse
`error_text`.

Browser failures also retain `failure_diagnostic` in the run and response:
the machine stage, sanitized page origin/path, document readiness, and element
counts. The same JSON is kept under private `diagnostics/<run_id>/` with the
configured expiry. It contains no cookies, URL query values, or page text.

When `run_enabled_once` remains active for at
least 15 minutes, zero or more `skill_progress` JSONL records precede the final
response. Their `params.notification_delivery=runtime` marker delegates UI and
channel presentation to the host; it does not change success, retry, routing,
or final-result semantics. The host enforces a minimum 900-second delivery
interval and deduplicates each delivery by task and frame sequence.

`videos.csv` columns:

`sequence,global_sequence,platform,browser_mode,source_mode,search_keyword,discovery_source_url,title,platform_text,cover_screenshot_path,cover_capture_source,video_page_url,discovered_at,engagement_captured_at,views,likes,comments,favorites,shares`

`images.csv` columns:

`sequence,global_sequence,post_sequence,image_sequence,platform,browser_mode,source_mode,search_keyword,discovery_source_url,title,platform_text,image_url,image_screenshot_path,source_page_url,discovered_at,engagement_captured_at,views,likes,comments,favorites,shares`

CSV files use UTF-8 BOM, RFC 4180 quoting, stable order, and spreadsheet formula
injection protection. The private immutable record ledger remains the recovery
source of truth; CSV files can always be rebuilt.

`platform_text` is the author-provided post caption extracted from reviewed
platform DOM markers. A video record and every image belonging to one carousel
retain that caption. Media screenshots are never OCR inputs, and AiAPP exposes
no visual-text field. Collected image screenshots are retained under the
skill-owned export directory and referenced by `image_screenshot_path` so AiAPP
can provide authenticated same-origin downloads without proxying arbitrary
remote URLs.

## Browser and Capture Rules

- Browser mode defaults to silent. `visible` is accepted only as an explicit
  structured planner argument; when selected, a missing desktop session returns
  `display_unavailable` instead of changing the requested mode.
- A silent background run opens the skill-owned persistent browser profile only
  when the platform reports the structured `login_required` state and a desktop
  is available. This temporary window exists only for interactive sign-in. It
  closes after authentication is present, then retries collection in the
  originally requested silent mode. `challenge_required`, selector drift, rate
  limits, and ordinary collection failures never promote a silent run to a
  visible browser. A challenge enters an error-specific exponential cooldown
  instead of repeatedly reopening the site. An explicitly visible run keeps its
  window open while a user completes the platform challenge, then continues in
  that same run. Verification and feed-readiness waits observe graceful stop.
  The same private profile is reused after login.
- The skill uses one private persistent browser profile per platform. Later
  runs reuse that profile's cookies, local storage, and browser cache; clearing
  collected results preserves this login/session state. The skill does not read
  cookies from unrelated browser profiles or write them to logs/checkpoints.
- Capture uses screenshots of browser-rendered media elements. It does not
  issue additional requests for original images and does not present this as a
  mechanism for bypassing anti-automation controls.
- A visible, unobscured platform video frame is the preferred cover; a
  platform-specific rendered poster is the fallback. If the page already
  autoplayed, the captured frame is not represented as the encoded timeline's
  exact frame zero. If neither trusted element is available, the result has no
  preview rather than a whole-page fallback. Duplicate items never replace an
  existing persisted cover. Covers and image screenshots are stored for preview
  only and never sent through OCR or model text review. Text comes from the
  platform's rendered title and author-caption fields.
- Login challenges, access denial, and rate limiting produce structured waiting
  or failure states. The skill never bypasses them.

## Error Contract

Errors use `extra.{schema_version,source_skill,status,error_code,message_key,retryable}`.
Stable examples include `display_unavailable`, `browser_missing`,
`login_required`, `challenge_required`, `rate_limited`, `selector_drift`,
`no_items_collected`,
`platform_unsupported`, `source_scope_empty`, `run_already_active`,
`collection_already_enabled`, and `storage_lock_timeout`.
`error_text` is a human fallback and must never drive routing or retry logic.

## Request/Response Examples

```json
{"action":"enable","platform":"douyin","source_mode":"home_feed","rest_min_seconds":180,"rest_max_seconds":420,"confirm":true}
```

```json
{"action":"enable","platform":"xiaohongshu","source_mode":"topics","topics":["AI agent","机器人"],"browser_mode":"visible","confirm":true}
```

```json
{"action":"run_once","platform":"douyin","source_mode":"topics","topics":["AI agent"],"max_items_per_run":5,"browser_mode":"visible"}
```

```json
{"action":"disable","platform":"douyin"}
```

```json
{"action":"export_results"}
```

```json
{"action":"clear_results","confirm":true}
```

An exported video row may contain:

```json
{"source_mode":"topics","search_keyword":"AI agent","discovery_source_url":"https://www.douyin.com/search/AI%20agent","cover_screenshot_path":"video_covers/douyin_123.png","video_page_url":"https://www.douyin.com/video/123"}
```
