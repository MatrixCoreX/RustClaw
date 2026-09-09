# Platform Search and Capture Validation

Date: 2026-09-09. Scope: `media_discovery` only. Live model: `vendor-minimax / MiniMax-M3`.
No core routing, channel delivery, UI production, or unrelated configuration changes.

## Delivered Behavior

- Keyword collection opens the homepage, fills the actual visible field, clicks search, validates the query and waits for rendered results.
- Xiaohongshu supports textarea/search-icon submission, encoded AI-search URLs, visible detail links, refreshed parameters, scoped note capture and clipped carousel images.
- Kuaishou supports both observed search layouts. New cards open in-page players; visible covers are uniquely associated with public loaded post IDs, without private API calls or clipboard access.
- QR-only login dialogs are recognized. Known controls belonging to the current video player are not confused with unrelated screenshot obstructions.
- Hidden tooltip text is excluded from engagement counters. Xiaohongshu publication time is taken from the current note detail, excluding generated JSON-LD navigation dates, telemetry and comment dates.
- Login/CAPTCHA remains manual. No automation camouflage, challenge solving or access-control bypass was added.

## Local Deployment

Node skill hot-update, version `0.1.58`, enabled, registry generation `141`.
Operation: `0de5d0da-11fc-4eee-a109-c4fece2014a7`.
Receipt: `ce48a3b9fa6c72a2d8b641653b661854daab16686cb06efdf0f27e0e44fd5abd`.
No core recompilation/restart or remote-device deployment was required.

## Live NL Evidence

Case source: `scripts/nl_tests/cases/nl_cases_media_discovery_platform_search_20260909.txt`.
Raw model records: `logs/model_io.log`, keyed by task IDs below.
Run artifacts: `scripts/nl_suite_logs/media_discovery_platform_search_20260909/<timestamp>/`.
The harness's task-success status alone is not collection acceptance: an accurate failure report also completes an ask task.

| Run directory | Task ID | LLM calls | Actual result |
| --- | --- | ---: | --- |
| 20260909_124355 | f14f2ae8-c7ba-4e7c-922f-7d87c07e5173 | 4 | Xiaohongshu: 0 saved, 9 failures; detail/background screenshot scope defect. |
| 20260909_124657 | ac1a9762-908b-4672-b321-1a758394b7cb | 4 | Kuaishou silent: 0 saved, JSON rather than HTML search response. |
| 20260909_125710 | cb6dc72c-e370-48ef-ace8-7fc9dbbbd6ac | 3 | Xiaohongshu: 0 saved; clipped carousel and refreshed detail-link defects. |
| 20260909_131446 | aa9d8999-1833-4e60-8b83-0002e0aca58b | 4 | Kuaishou visible: 3 handled, 1 new video, 2 duplicates, 0 failures; caption/cover/comments/likes saved. |
| 20260909_131645 | e1c0166d-4c8a-4e95-b172-72668b5f4239 | 4 | Xiaohongshu: 3 posts, 12 images, 0 duplicates/failures; captions, likes and favorites saved. Publication review exposed generated JSON-LD dates. |
| 20260909_132235 | 263c86b6-4f4e-44cf-8414-5e2cf90fb098 | 3 | Final 0.1.58: Xiaohongshu 1 post, 7 images, 0 duplicates/failures. Persisted publication date 2026-09-06T02:16:46.000Z, source post_state:noteDetailMap.note.time. |

Six live NL invocations, 22 model calls total. The final three captured material successfully;
the first three were diagnostic failures despite the ask harness completing successfully.
Final skill run: `run_8caefbfa-1f1e-49aa-b326-2ccd336a7fb5`.
Its model incorrectly claimed publication was unavailable; the persisted date above is authoritative.

Additional real-browser checks, not claimed as NL tests:

- `run_020ca9e3-2d99-45a0-8c5b-12ef1bb5886f`: Kuaishou visible, 3 new videos, captions/covers each 3, failures 0.
- `run_ab7f8eee-69d8-44e0-90a5-1901cae70b34`: Xiaohongshu, 1 new image/caption/screenshot, failures 0.
- `run_697bf26b-d041-4d7d-993a-6d81aa0238eb`: final counter fix, Kuaishou visible, 1 duplicate, failures 0. Actual comment display `1.1万`, without hidden tooltip text.
- Final publication extractor returned `2026-09-06T02:16:46.000Z` and `2026-07-08T08:40:31.000Z` from current Xiaohongshu note details, not collection time.
- Kuaishou silent search reached valid results once, but a subsequent attempt returned JSON. This is not reliable silent-mode acceptance.

## Data and UI Verification

- All pre-existing 173 records were retained. Final ledger: 197 records (142 videos, 55 images), with 4 new Kuaishou videos and 20 new Xiaohongshu images. No active batch or background coordinator remains.
- This round's generated-date records were corrected only when a matching source timestamp was verified; unverified dates were cleared, not guessed. Audit: `scripts/nl_suite_logs/media_discovery_platform_search_20260909/publication_corrections.json`. No media/captions/order were removed.
- AiAPP item endpoint returned HTTP 200 with the saved captions, keyword provenance, publication fields and `preview_available=true`.
- Authenticated preview endpoints for Kuaishou sequences 176-178 and Xiaohongshu sequences 182-184 returned HTTP 200, `image/png`, non-empty payloads. A Kuaishou saved cover was visually inspected.
- Persistent user files: `data/skills/media_discovery/exports/videos.csv`, `images.csv`, `video_covers/`, `images/`.
- Sandboxed `/tmp/agent-runtime-writable/0/exports` paths in an agent reply are not host download paths; use AiAPP or explicit `export_results` delivery.

## Automated Verification

- `MEDIA_DISCOVERY_BROWSER_TEST=1 node --test --test-concurrency=2 --test-reporter=spec`: 156 passed, 0 failed, 0 skipped.
- Coverage includes real Chromium fixtures for three-platform form submission, delayed/invalid results, popup navigation, ordered detail capture, 404 handling, login barriers, cancellation, carousel completeness, counter visibility, timestamp ownership, CSV and preview files.
- Prompt check passed; NL hard-match scan: unknown 0, known legacy 0. Skill storage ownership passed. Hotplug inventory self-test passed. `git diff --check` passed.
- Full repository hotplug inventory has 8 existing findings outside these edits; long-file gate has 12 existing Rust violations outside these edits. Neither baseline was weakened. Changed production browser file remains below 2,000 lines.
- Only Linux/Chrome was available for these live tests. macOS/Pi live execution was not tested in this round.

## Remaining Limits

- Douyin is still awaiting manual resolution of its search-page verification; no new live capture success is claimed for it.
- Kuaishou login cookies persisted, but a sidebar sign-in offer/limited-results prompt can remain. Cookie existence is not proof of unrestricted server-side login.
- Kuaishou silent-mode search is intermittent; visible-mode success must not be described as full silent-mode success.
- One model made an unnecessary filesystem lookup before selecting the skill. Another needed one response-contract repair. Neither caused a repeated collection; these are agent-quality follow-ups, not reasons to add NL hard matching to the core.
- Model replies occasionally overstated empty CSVs, browser state or publication availability. Acceptance uses persisted structured evidence, not those statements.
- Follow-up: include publication availability/counts in the skill's bounded capture receipt so the model need not infer per-record fields from aggregate item counts; do not add a core language-specific reply branch.
