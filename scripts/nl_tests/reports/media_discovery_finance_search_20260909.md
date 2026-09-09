# Media Discovery Keyword Search Validation (2026-09-09)

## Scope and Verdict

Test keyword: `财经`. Platforms: Douyin, Xiaohongshu, Kuaishou.
Each NL case requests one batch, up to three posts and five collection minutes,
with platform-default browser mode. Continuous collection is not enabled.
Authentication/challenges require manual completion; no bypass was attempted.

**Live collection acceptance has NOT passed.** Six NL tasks completed their
agent replies, but all six saved zero new records. A task status of `succeeded`
means the agent delivered its reply, not that collection succeeded.

Cases: `scripts/nl_tests/cases/nl_cases_media_discovery_finance_search_20260909.txt`.
Provider/model: `vendor-minimax` / `MiniMax-M3`. No dry run or platform skip.

## Live NL Results

| Round | Platform | Task ID | LLM calls | Actual outcome |
| --- | --- | --- | ---: | --- |
| Initial | Douyin | `179b539f-5d24-4415-9c91-f5c1cf2ca8f6` | 3 | Manual homepage confirmation did not restore search; `manual_verification_not_restored`, zero saved. |
| Initial | Xiaohongshu | `f3f18696-f7d6-4ebf-9a3f-2c07e52d1741` | 4 | Candidate detail redirected to `/404`, incorrectly classified as a challenge; zero saved. |
| Initial | Kuaishou | `a6e73472-5ef0-4bda-8512-6d04e677a8ab` | 3 | Old search URL returned JSON rather than a document; `no_items_collected`, zero saved. |
| 0.1.53 | Kuaishou | `be2d1f79-069e-448c-a66f-4d3057be2b55` | 3 | Correct search path still returned JSON; `selector_drift`, zero saved. |
| 0.1.53 | Xiaohongshu | `0d7cb3bc-0d88-40b8-b100-dd0694778760` | 3 | Navigation timed out; unbounded diagnostic read blocked cleanup. Graceful stop was requested; the owned test browser then required SIGTERM. Zero saved. |
| 0.1.53 | Douyin | `7ffd121c-0031-4a2d-ae7b-4ba0b0faaa85` | 3 | Actual search verification window opened; manual verification was cancelled. Browser closed, `interactive_verification_cancelled`, zero saved. |

Total: 19 LLM calls. Original response fields were replayed with per-case
`LLM#1..N` numbering in the assistant conversation. Full responses are retained
locally in `logs/model_io.log`; do not publish request logs or browser profiles.

Local run roots:

- `scripts/nl_suite_logs/media_discovery_finance_search_20260909/`
- `scripts/nl_suite_logs/media_discovery_finance_search_fixed_20260909/`

The corresponding run directories are `20260909_121103`, `20260909_121219`,
`20260909_121321`, `20260909_121912`, `20260909_121938`, and `20260909_121940`.

## Implemented Skill-Local Corrections

- Manual verification retains the actual blocked search/detail URL and keyword.
  A ready homepage or another keyword cannot satisfy search verification.
- Candidate selection excludes hidden nodes and preserves first-seen DOM order
  across mixed card IDs and links, with canonical URL deduplication.
- HTTP 404/410 and detail `/404` redirects are `source_unavailable`; skip the
  missing post and continue subsequent candidates instead of claiming CAPTCHA.
- Kuaishou uses `/search/<encoded keyword>`, verified by submitting its visible
  website search form. Both direct navigation and the website-opened search tab
  returned `{"result":2,"error_msg":null,...}` during this test; the cause is
  not established and is not assumed to be a selector bug or an IP ban.
- JSON navigation responses are `unexpected_page_response`, not empty results.
- Browser failure diagnostics have a separate five-second read limit so a
  stalled renderer does not indefinitely block cleanup.
- Terminal batch receipts include `browser_session_open=false`. Skill guidance
  distinguishes one-shot completion from continuous-worker resume and forbids
  claiming historical CSV files are empty from a zero-record batch.

No core runtime, channel, UI production, or authentication policy was changed.
No natural-language routing match or fixed agent reply was added.

## Latest Package Smoke

Local installed version: `0.1.54`, generation `137`.
Receipt: `c3022b5aa6c8aa0a82b22fdc3dd98c167464b20351df98088c45d383c34e3ff0`.
Hot update used the normal admission service; old in-flight calls retained their
pinned 0.1.53 version.

Two direct `run_skill` checks used the same keyword (no LLM calls):

| Platform | Task ID | Result |
| --- | --- | --- |
| Kuaishou | `61d3759f-c52a-4b05-9886-57cc33436498` | Correct `unexpected_page_response` from the search URL in about 1.3 seconds; closed browser, zero saved. |
| Xiaohongshu | `8fc188c5-f47c-4133-9b34-926fdb6a87b3` | Navigation still timed out, but diagnostic and browser cleanup finished automatically in about 52 seconds; zero saved. No external process termination needed. |

Two preliminary direct checks accidentally requested a two-minute run, below
the declared five-minute minimum. The verifier rejected both before execution
(`invalid_argument_value`); they are not platform failures or collection runs.

## Regression and Data Verification

Command from `optional_skills/media_discovery`:

```sh
MEDIA_DISCOVERY_BROWSER_TEST=1 MEDIA_DISCOVERY_CHROME_BIN=/usr/bin/google-chrome npm test
```

136 tests passed, zero failed/skipped. These include real Chromium with fully
intercepted fixtures for each platform: search -> first detail 404 -> next three
details in order -> committed records, CSV, screenshots, keyword/source URL,
publication date and caption. Fixtures make no platform requests and are NOT
evidence of current live-platform capture success.

- `git diff --check`: passed.
- NL hard-match check: zero new/unknown findings.
- Skill storage ownership self-test and check: passed.
- Hotplug coupling self-test: passed; inventory still reports eight pre-existing
  findings outside this change in model-provider config, schedule service, KB
  storage, builtin dispatch, AiAPP admission, receipt handling, child-task
  policy and native respond. No baseline was weakened.
- Every touched production/test source file is below 2,000 lines.
- AiAPP items API returned `ok=true`, `matching_total=173`, `active_run=null`.
- Persistent records remain 173: Xiaohongshu 75, Douyin 50, Kuaishou 48.
  No records created by this test; existing material was preserved.
- All test batches and their browser sessions finished. No continuous worker
  was enabled by this suite.

## Remaining Acceptance Gaps

1. After access is available, repeat the same bounded NL cases and require real
   search-source records in first-seen order. Do not substitute recommendations
   or treat a manual challenge as passed without completion on the target page.
2. The 0.1.53 final replies still over-inferred that historical CSV files were
   empty and that `diagnostic.document=null` proved total page unavailability.
   The latter only means diagnostic evidence was unavailable. Report observed
   counts separately from model prose; further evidence/prompt work needs a
   separate regression, not a hardcoded response for this case.
3. Navigation timeout strings remain in nested run errors while the public
   envelope uses `execution_failed`. A future skill-local change should map
   typed browser timeout errors to stable codes without parsing error prose.
