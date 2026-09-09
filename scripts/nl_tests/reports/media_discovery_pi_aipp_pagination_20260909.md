# Media Discovery Pi and AiAPP Pagination Validation

## Scope

- Deploy UI/runtime artifact revision
  `a317dd11cdbe292edf2139b2a1cbb63c6064e1d3`, followed by registry-only revision
  `6a6b0ee37dd23b7b54658aa58624811d858fb0f1`, to the local UI/runtime policy and
  the existing Linux/aarch64 Pi deployment. No Rust sources changed between
  these revisions, so the registry update does not require another ARM build.
- Preserve device configuration, credentials, channel bindings, private skill
  storage, and existing collected content.
- Update media discovery through the existing admission/receipt mechanism.
- Do not compile Cargo packages on the Pi or publish a new public release.

## Pagination Fix

The collection API paginates records, whereas the UI groups images from one post
into one card. The newest 20 local records were all from Xiaohongshu and became
only five cards after grouping. Other platforms remained in subsequent records.

The host now fills pages with up to 20 complete posts, using bounded cursor
requests and completing a boundary gallery before advancing. Platform changes
reset pagination. Chronological order remains authoritative: selecting all
platforms does not force an older post from each platform onto the first page.

## Completed Verification

- UI focused unit/render suite: 26 passed.
- Playwright collection suite: 14 desktop/mobile, theme, gallery, filtering,
  and pagination scenarios passed.
- Production build and TypeScript checks passed. Existing Vite large-chunk
  advisories remain; they are not compilation errors.
- Product-identity inventory/self-test and two-brand UI builds passed.
- AiAPP decoupling, skill-storage ownership, and MCP contract guards passed.
- Both time orders traversed all 198 local records as 183 post cards without
  omissions or duplicates. The newest first page contains 20 posts/35 records,
  including Xiaohongshu and Kuaishou. Older Douyin content remains accessible.
- Local nginx and direct web entry return the same deployed UI bundle,
  `index-Bp0fHl5X.js`.
- Production UI browser audit used real read-only collection responses with
  isolated test authentication bootstrap. This is not a password-login test.
- Pi production UI empty-state checks passed at widths 1440 and 390, with
  successful catalog/items responses, no page errors, and no horizontal overflow.
- On the Pi, staged media discovery 0.1.59 passed 34 real Chromium/identity/
  concurrency tests and 29 protocol/storage/manifest tests; none were skipped.
- A visible Chromium session also launched and rendered on the Pi's existing
  desktop. Headless screenshot/carousel tests passed separately.
- Pi source preflight checked 281 tracked files. Two OCR files matched an older
  repository revision exactly; reviewed copies were preserved before upgrade.

## Device Constraints

The Pi has approximately 1 GiB physical RAM. The existing detected-memory policy
allows one platform browser at a time; this is intentional and not a failure of
multi-platform support. Node 20.19.2 and Chromium 146 are installed. Existing
local desktop discovery is used for explicit login/verification windows.

Live platform access still depends on platform availability and login. Manual
verification is not bypassed. A completed agent task alone is not evidence that
collection saved a post; inspect the skill's structured counts and run status.

## Deployment Acceptance

- [x] ARM workspace build and proactive receipt projection completed (46 binaries,
  34 receipts, no compiler warnings).
- [x] Pi deployment checksums, backups, startup health, and UI verified.
- [x] Installed media discovery upgraded through admission to 0.1.59; final Pi
  generation 15 and local generation 143 include the corrected resource policy.
- [x] Bounded NL readiness, one-post collection, and stop/status cases evaluated;
  only readiness passed. Evaluation completion is not a passing acceptance gate.
- [x] Final active-run state checked; test traces retained on both machines.
- [ ] Real Pi platform collection with saved content: not passed this round.
- [ ] Combined NL stop/history reply: not passed this round, although stopping
  succeeded and direct structured status/history calls passed afterward.

## Live NL Finding

The initial readiness case (`5da6c40d-94d2-4014-954d-69031df28a29`) exposed a
registry resource error: every media-discovery action inherited a 2048 MiB memory
request, including capabilities/status. Both calls were rejected before dispatch
with `resource_admission_unavailable`, `wait_reason=memory_unavailable`; the Pi
had approximately 411 MiB available. This is not a browser or model-selection
failure. The test was canceled after two completed LLM calls to stop repeated
identical rejected reads. An in-flight third response completed afterward;
three calls are retained. This is not a passing case.

The registry fix uses a 96 MiB general control profile and a separate 384 MiB
network/browser profile for `run_once` and `run_enabled_once`. Admission still
checks available memory. The existing skill-owned detected-memory concurrency
limit remains in force. The main runtime and unrelated skill profiles are not
changed. Main/container registries have the same policy, with dedicated tests.
Four resource-profile unit tests passed.

The post-fix suite `20260909_160753` made 12 LLM calls using the Pi's existing
`vendor-custom` / `minimax` configuration:

| Case | Task | LLM calls | Result |
| --- | --- | ---: | --- |
| Readiness | `6cfb2ea2-2ea5-4793-b8c4-b64f7c8085e9` | 4 | Succeeded; capabilities and status executed |
| One Douyin post | `fd82027b-b992-407e-ac6c-6d895aa4b9ee` | 1 | Rejected before browser dispatch: 263 MiB available, 384 MiB requested; no collection run created |
| Stop and recent history | `852cc992-8fa9-40ee-b573-22d35ceb3550` | 7 | Stop executed; subsequent planning failed with `plan_parse_failed_no_executable_steps` |

The collection task incorrectly entered mutation reconciliation after its
pre-dispatch admission denial. It was explicitly canceled during cleanup;
the cancellation does not turn the failed case into a pass. Raw numbered model
response fields were replayed in the development conversation. The initial
failed readiness plus the suite account for 15 model calls in total.

Direct `run_skill` checks, without LLM calls, separately confirmed:

- `23377888-1ced-4e55-8331-73399080a573`: capabilities succeeded.
- `e260267b-40b2-450d-8319-0f0e1a0b44c7`: status succeeded; `active_runs={}`,
  `background_worker=null`, images/videos zero, and no latest run.
- `b0de71f6-38c9-45e4-b3a6-ffebc49570e8`: list_runs succeeded; zero runs.

A separate bounded diagnostic opened real ARM Chromium with an isolated
profile and visited the Douyin homepage. HTTP returned 200, but the document
was a verification page, not an accessible feed. Browser/Node process-tree
PSS peaked at approximately 369 MiB, and swap use increased. This evidence does
not justify reducing the 384 MiB browser admission floor. The diagnostic
browser and its temporary profile were closed/removed; existing login profiles
and collected data were not modified. No platform verification was bypassed.

## Follow-up Findings

These are recorded for separate runtime work, not concealed by skill-name
branches, natural-language matching, or fixed replies:

- A denied resource grant before dispatch lacks explicit no-side-effect
  attribution and can incorrectly become mutation reconciliation.
- The stop/history case produced invalid subsequent plans despite successful
  tool execution; model repair did not recover the request.
- Failure composition used response material inconsistently with the provider's
  clean content field. The final reply path needs a generic response-channel
  regression test, not a media-discovery-specific text filter.
- The generic registry-policy checker reports three existing `host_process`
  isolation-profile findings for process capabilities. Comparison with the
  pre-change revision found no new findings; that guard is not reported as green.
- Live collection remains dependent on sufficient free memory and manual
  platform authentication/verification. Browser fixture tests are not evidence
  of successful live collection.

## Deployment Evidence

- Local and Pi nginx/direct web UI return `index-Bp0fHl5X.js`; index SHA256:
  `0c51dac3815fe0fa2f7c76fe598f5b0d6ae017f5d5e4ba2597895b223d67fec5`.
- Pi webd remains bound to `127.0.0.1:8788`, so its direct UI comparison ran
  locally on the Pi. LAN access remains through the existing nginx entry;
  no external API port was opened for deployment or testing.
- Pi clawd SHA256:
  `3cf497ea5e6e3a85051eb351546f72afffb9733049dc2cff2ca7b0341b33a1c9`.
- Pi media-discovery receipt:
  `fe94d4c4a682c6e7289202c6183e002f73780cec2f7ba887d3d9a4507eb9e669`.
- Pi protected backup:
  `.deploy-backups/pi-media-20260909-20260909T155243`, including a verified
  SQLite backup and preserved configuration. The existing release tag remains
  unchanged because this was not a new public release.
- Other optional skill installations, channel/provider settings, TLS setup,
  and compilation caches were preserved. Unrelated WhatsApp configuration
  edits were not included in these commits or deployments.

Case source: `scripts/nl_tests/cases/nl_cases_media_discovery_pi_20260909.txt`.
Local visual evidence: `target/deploy/pi-media-20260909/ui-tests/`.
Raw suite artifacts on both hosts:
`scripts/nl_suite_logs/media_discovery_pi_20260909/`.
Authoritative Pi model trace: `/home/pi/RustClaw/logs/model_io.log`.
