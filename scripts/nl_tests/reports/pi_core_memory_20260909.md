# Pi Core Memory Verification

## Scope

- Production changes: `7ca0b1ef3`, `f7397cac8`; release-layout NL fixture: `e8687347d`.
- Target: Linux/aarch64, 991 MiB physical RAM, existing release installation.
- Core-only deployment. Do not replace UI, webd, messaging binaries, configuration,
  credentials, database contents, installed skills, grants, or generation pointers.
- The memory policy and diagnostic commands are documented in
  [runtime_memory_profile.md](../../../docs/architecture/runtime_memory_profile.md).

## Local Verification

| Check | Result |
| --- | --- |
| SDK library, lifecycle, receipt and language adapters | 76 passed |
| Runtime-memory selection, pool capacity, persistence, free-page reclamation | 7 passed (includes one existing memory-source test) |
| Skill storage isolation/migration | 14 passed |
| Private SQLite permissions | 2 passed |
| Resource scheduler | 8 passed |
| Skill admission/pinned generation | 4 passed |
| AiAPP projection and assets | 15 passed (one overlaps admission) |
| Task repository, claims, resume and cancellation | 58 passed |
| Product identity self-test/inventory | passed |
| Two-brand UI builds | passed; existing Vite chunk-size advisories |
| Cross-platform static contracts | passed |
| Skill storage ownership | passed |
| NL hard-match inventory | unknown=0, known_legacy=0 |

Apple target `cargo check` was attempted. It is blocked by missing Apple C
cross-compiler/SDK support on the Linux build host: host `cc` rejects `-arch`,
`-mmacosx-version-min`, and `-gfull` in `ring`/`aws-lc-sys`. macOS runtime success
is not claimed. The allocator implementation is gated to Linux/glibc only.

The whole-tree long-file gate reports 12 existing violations in untouched files.
All files changed here remain at or below 2,000 lines, including main at 1,984.
No historical module split is included in this memory fix.

## Baseline

The old running core initially had RSS about 272 MiB plus 92 MiB swapped.
That aged process is **not** the comparison baseline. With no nonterminal tasks,
the old core was restarted, then the probe issued four cycles of authenticated
GET requests to health, AiAPP catalog, and Skill Store (12 requests), followed
by 90 seconds idle settling. All requests succeeded.

- Settled RSS: 195,088 KiB.
- Settled PSS + SwapPss: 217,345 KiB.
- Sampled peak PSS + SwapPss: 248,353 KiB.
- Process high-water RSS: 231,152 KiB.
- Open database descriptors after settling: 14 (8 main, 2 audit, 4 skill-owned).
- Raw evidence: `target/deploy/pi-media-20260909/memory-baseline.jsonl`.

The probe waits for the listener before issuing workload requests. An initial
attempt made before the slow Pi startup finished was discarded and rerun; it is
not counted as a successful probe.

## Deployment and Live Acceptance

The local ARM release build completed in 14m21s with no compiler warnings.
Deployment source: `e8687347d07d8ded3406e6b70ef88a68301998d4`.
Core SHA256: `f40f821db579256a9649e8549483f51fbbab2133d1d6f63c805bef73ad84344f`.
The signed checksum manifest was verified before activation; the previous core
is retained under `.deploy-backups/pi-core-memory-20260909-20260909T192008`.
Configuration/environment and skill-pointer hashes matched before/after.
Startup reports `runtime_allocator_tuning attempted=true applied=true`.

### Matched Read-Only Workload

| Metric | Restarted old core | Optimized core |
| --- | ---: | ---: |
| Settled RSS (KiB) | 195,088 | 112,928 |
| Settled PSS + SwapPss (KiB) | 217,345 | 111,406 |
| Sampled peak PSS + SwapPss (KiB) | 248,353 | 111,470 |
| Process high-water RSS (KiB) | 231,152 | 116,992 |
| Settled swapped memory (KiB) | 23,744 | 0 |
| Database descriptors | 14 | 4 |
| Successful read-only requests | 12/12 | 12/12 |
| Mean health latency (seconds) | 0.083 | 0.092 |
| Mean AiAPP catalog latency (seconds) | 0.253 | 0.249 |
| Mean Skill Store latency (seconds) | 0.235 | 0.175 |

Settled PSS + swapped memory decreased by 48.7%, about 103.5 MiB. Latency samples
are small and include startup activity; do not claim a general speedup. The
read-only probe is not a long-running agent/browser stress test. Database file
descriptors were added to the probe after the baseline run; the baseline count
was obtained from the same still-running baseline process before replacement.

Candidate evidence: `target/deploy/pi-core-memory-20260909/memory-optimized.jsonl`.

### First Live NL Run

Both permanent cases in `scripts/nl_tests/cases/nl_cases_runtime_memory_pi_20260909.txt`
passed terminal-state, actual tool-call and no-side-effect assertions with the
Pi's existing `custom/minimax` relay configuration:

| Case | Task | LLM calls | Seconds |
| --- | --- | ---: | ---: |
| readonly_memory_policy | 5bb552c6-cb32-4006-875b-7e32e7935370 | 4 | 34 |
| installed_media_readiness | cc87060c-3a99-45e0-8f16-a95f66b2c0a7 | 4 | 79 |

The first read the deployed policy file; the second executed the installed
media_download 0.3.29 capability action under generation 16. No media was
downloaded, no browser started, and no configuration mutated.

Evidence is retained locally and on Pi under
`scripts/nl_suite_logs/runtime_memory_pi_20260909/20260909_192320/`.
Raw numbered provider records are in the Pi deployment's `logs/model_io.log`.

**Follow-up found by NL:** after these calls and another 12-request probe, the
core retained 216,864 KiB PSS + SwapPss, including 59,136 KiB swapped. Startup
improvement alone is not a sufficient claim about sustained NL memory use.
The follow-up implementation `f7397cac8` adds serialized free-page reclamation
on the blocking pool. Its live-buffer/database preservation test and explicit
override test passed; storage and task repository regression tests were rerun.
Its deployment and post-NL verification are recorded below.
Intermediate evidence: `target/deploy/pi-core-memory-20260909/memory-post-nl.jsonl`.

Both AiAPP catalog/item endpoints, nginx static UI, and direct webd static UI
returned HTTP 200 after the first deployment. Messaging/webd process IDs were
unchanged. The download history remained visible and discovery correctly
reported an empty Pi collection history.

## Final Candidate: Free-Page Reclamation

The second local ARM build completed in 19m37s without compiler warnings.
Deployed source: `f7397cac8f30953896324a786a1cffbd59815650`.
Core SHA256: `44006b99dbf56878bba67e01a2e755abe939b419680420cfda08323496ee4bf0`.
The signed manifest was verified before activation. The first optimized core
remains available under
`.deploy-backups/pi-core-memory-reclaim-20260909-20260909T194933`.
Only the core executable and committed documentation/test files were replaced;
the Pi did not compile source. Configuration and skill-pointer hashes matched.

### Matched Interface Probe

Four cycles of the same three GET endpoints, then 90 seconds settling:

| Metric | Restarted original core | Final candidate |
| --- | ---: | ---: |
| Settled RSS (KiB) | 195,088 | 155,056 |
| Settled PSS + SwapPss (KiB) | 217,345 | 153,522 |
| Sampled peak PSS + SwapPss (KiB) | 248,353 | 161,506 |
| Settled swapped memory (KiB) | 23,744 | 0 |
| Database descriptors | 14 | 5 |
| Successful read-only requests | 12/12 | 12/12 |
| Mean health latency (seconds) | 0.083 | 0.092 |
| Mean AiAPP catalog latency (seconds) | 0.253 | 0.245 |
| Mean Skill Store latency (seconds) | 0.235 | 0.214 |

The final candidate retained 149.9 MiB versus 212.3 MiB on this probe, a 29.4%
reduction. Use this final measurement rather than the lower first-candidate
startup number as the deployed result. Background initialization, connection
use, allocator timing, and OS page pressure still cause variation; this is not
a controlled long-duration benchmark.

Evidence: `target/deploy/pi-core-memory-reclaim-20260909/memory-reclaimed.jsonl`.

### Repeated Live NL and Post-Task Probe

The same two read-only cases passed again using the existing model configuration:

| Case | Task | LLM calls | Seconds |
| --- | --- | ---: | ---: |
| readonly_memory_policy | 4682a2e0-38f5-4f00-9cde-bc298b01f115 | 4 | 39 |
| installed_media_readiness | 39e1e656-beb9-42dc-bffb-119f386a1cb4 | 4 | 52 |

Each case made one successful real capability call. Journal assertions reported
zero completed side effects and zero mutation steps. The response verifier
passed both answers. No media download, browser launch, configuration write,
or external publishing API action was requested or executed.

After NL, another identical 12-request probe and 90-second settling period gave:

| Metric | First candidate after NL | Final candidate after NL |
| --- | ---: | ---: |
| Settled RSS (KiB) | 159,264 | 165,680 |
| Settled PSS + SwapPss (KiB) | 216,864 | 178,251 |
| Settled swapped memory (KiB) | 59,136 | 14,096 |
| Sampled peak PSS + SwapPss during post-NL probe (KiB) | 226,984 | 201,579 |
| Successful read-only requests | 12/12 | 12/12 |
| Mean health latency (seconds) | 1.596 | 0.068 |
| Mean AiAPP catalog latency (seconds) | 0.268 | 0.287 |
| Mean Skill Store latency (seconds) | 0.192 | 0.176 |

Post-NL retained memory decreased from 211.8 to 174.1 MiB (17.8%) versus the
first candidate. RSS alone increased slightly while swapped memory decreased;
the combined metric avoids misreporting that tradeoff. The first candidate's
health mean included a 6.215-second initial request under swap pressure, so the
latency difference is not a general throughput claim. Model outputs and timing
vary between runs. The original unoptimized binary was not run through these
NL cases; do not present this as an original-versus-final NL A/B comparison.

The independent 5-second memory timeline contains 45 samples spanning NL and
post-task activity (225 seconds). It is useful diagnostic evidence, not a
complete task peak trace or a long-running leak test. Active database connection
counts also vary with background work; the post-NL final probe had nine open
database descriptors, not a guaranteed fixed idle count.

Evidence:

- `scripts/nl_suite_logs/runtime_memory_reclaim_pi_20260909/20260909_195228/`
  is retained on the development host and Pi, including assertion summaries.
- `target/deploy/pi-core-memory-reclaim-20260909/llm-public-responses.jsonl`
  retains each numbered model-visible response and provider metadata; private
  reasoning text is not reproduced in the report or chat.
- `target/deploy/pi-core-memory-reclaim-20260909/memory-post-nl.jsonl`
- `target/deploy/pi-core-memory-reclaim-20260909/memory-nl-timeline.jsonl`
- The Pi's `logs/model_io.log` remains the original provider trace source.

### Final Functional Checks

- Health: HTTP 200, running=0, queued=0; no nonterminal tasks in the database.
- AiAPP catalog: two installed apps. Discovery items: zero, matching the prior
  empty history. Download history: three items returned by the limited query.
- Nginx on port 80 and direct webd on port 8788 both return HTTP 200 and match
  the existing deployed `UI/dist/index.html` exactly.
- webd, Telegram, and WeChat process IDs are unchanged throughout core updates.
  This check does not claim new end-to-end channel delivery acceptance; existing
  Telegram upstream connectivity was outside the scope of this memory change.
- Protected configuration/environment and skill version/generation pointers
  remain unchanged. The deployed core hash matches the signed payload.

This optimization changes memory retention, not model routing, skill permissions,
task budgets, database schemas, user data, or the installed skill set. It does
not impose a strict memory ceiling. Browser and speech-model child processes
have separate footprints and are not included in the core-only measurements.
