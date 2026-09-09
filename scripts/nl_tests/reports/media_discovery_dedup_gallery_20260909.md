# Media Discovery Dedup and Gallery Verification

Date: 2026-09-09. Scope: media discovery package, generic AiAPP collection renderer,
focused tests and current-architecture documentation. No clawd or channel changes.

## Findings and Changes

- Initial ledger: 197 records, including 95 Xiaohongshu records. No duplicate explicit
  dedup key, exact image URL or screenshot hash was found. Live cursor pagination also
  returned 95 unique Xiaohongshu record IDs.
- Three posts were rendered as 18 separate cards with repeated captions: records
  179-184 (6 images), 185-189 (5 images), and 191-197 (7 images).
- The generic renderer now groups by platform and positive `post_sequence`. It shows
  the caption once, orders retained images by `image_sequence`, and supports image
  switching, enlargement and authenticated download of the selected image.
- Bounded cursor lookahead completes the contiguous gallery at a page boundary,
  without consuming rows belonging to the next post. Refresh replaces page state;
  stale fetches cannot repopulate an app after navigation away.
- The skill deduplicates candidate URLs by post ID. Image dedup uses the post plus
  resource identity; reviewed Xiaohongshu CDN object IDs ignore signed delivery
  variants. Other image sources keep full-URL identity. Titles and captions are
  never dedup keys, and distinct images/posts are not merged by visual similarity.
- Stable image resource filenames avoid reusing a previous carousel position's file.
  Existing records are recognized without rewriting their IDs, paths or CSV rows;
  partial recapture reuses the existing post group.

## Verification

- Skill unit/protocol/Chromium suite: 162 passed, 0 failed, 0 skipped.
- Focused UI unit/render tests: 22 passed. TypeScript check passed.
- Collection browser suite: eight responsive/theme checks and four gallery checks
  passed. Tests cover a gallery spanning API pages, ordered switching, refresh,
  next/previous pages and exact selected-image download. Unit tests include 100-image
  galleries in both sort directions, concurrent persistence and signed URL variants.
- Shared viewer browser suite passed for collection images, video covers and task
  activity, both themes and mobile/desktop, including failure/retry and keyboard focus.
- Live retained-data pagination: all 198 final records appear as 183 cards over ten
  pages; Xiaohongshu's 96 records appear as 81 cards over five pages. No duplicate
  record or gallery IDs across pages. The original 197 numbered records remain.
- Deployed nginx production bundle was exercised in Chromium with real collection
  and preview API data. Read-only test authentication bootstrap was mocked, so this
  is presentation verification, not a fresh password-login test. The seven-image post
  appears once and switches to `2 / 7`; no page errors. Initial harness attempts lacked
  valid bootstrap state and timed out before the app; no production auth was changed.
- Screenshots: `scripts/nl_suite_logs/media_discovery_dedup_20260909/`, including
  `aipp-deployed-desktop.png`, `aipp-deployed-mobile.png`, `ui-final/` and `viewer/`.
- AiAPP decoupling, prompt, NL hard-match and storage ownership checks passed.
  Hotplug inventory self-test passed. `git diff --check` passed.
- Product-identity inventory/self-test and the two-identity UI build test passed.

## Live NL

Case: `scripts/nl_tests/cases/nl_cases_media_discovery_dedup_20260909.txt`.
Run directory: `scripts/nl_suite_logs/media_discovery_dedup_20260909/20260909_143215/`.
Task: `de91417e-890f-4b2c-8609-76449d723abc`.
Skill run: `run_586d8c53-a3a5-43d8-8749-8c73919aa261`.
Provider/model: `vendor-minimax / MiniMax-M3`. Three LLM calls, 117 seconds.
Raw request/return records: `logs/model_io.log`, starting byte 134234045; replayed
as LLM#1 capability loading, LLM#2 run_once, LLM#3 respond in the conversation.

The batch completed one post, saved one new image/caption/cover, and skipped zero
duplicates. Two earlier candidate failures were recorded; the final model answer
omitted them. Therefore this run proves live capture and preservation, not that a
duplicate was encountered on the live platform. Repeat suppression is proved by
the controlled browser/persistence tests, including changed CDN URLs and reordered
captures. No continuous run remains active. All test material was retained.

## Local Deployment

- Skill version `0.1.59`, enabled, registry generation `142`.
- Update operation: `d662ee36-ef60-4845-ab54-027ee3280760`.
- Receipt: `f61b3ba06f9419d896198ed9177e9dea641811612600b6c1c611e9837be960a7`.
- UI built and copied to the existing nginx root `/var/www/html/agent-runtime`.
  Both port 80 and 8788 returned HTTP 200 and the exact built index document;
  deployed bundle: `index-C0lkgaft.js`. Existing nginx settings were retained.
- No remote deployment or core restart. Build caches, credentials and login profiles
  were retained. Vite's existing large-chunk warnings were not hidden.

## Limits

- Identity matching does not perform OCR, fuzzy image matching, cross-post merging,
  CAPTCHA solving or browser camouflage.
- Lookahead is bounded to twenty requests. Non-contiguous fragments of an older post
  may appear on separate pages; a full post-index API is a separate contract change.
  None of the retained live galleries had this layout, and every current gallery was
  verified on one card without deleting any image.
- Provider failure-summary completeness and the two skipped live candidates remain
  separate follow-ups; no hardcoded multilingual reply was added to disguise them.
- Linux/Chrome was tested; macOS and Pi were not exercised this round.
