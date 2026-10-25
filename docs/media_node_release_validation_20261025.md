# Media node release validation — 2026-10-25

This release adds application-scoped browser authentication without changing the
retention of ordinary Agent task results or importing their browser sessions.
The Pi companion displays task-owned, short-lived Yuanbao login QR challenges.
The media website evaluates whether submitted content supports a creator
breakdown and returns a localized reason when it does not; download and
transcription remain independent options.

## Source integration

The release includes the pre-existing local Agent execution, evidence,
fallback, and AiAPP request-race changes. These were copied into a separate
integration worktree; the developer worktree and its private state were preserved.

The skill coupling inventory was compared against committed source
`4dd7318631bac4d9860b781366047a8e902d2916`. The integrated source removes fourteen
skill-literal occurrences from failure recovery and adds no skill coupling
relative to that source. The existing inventory had drifted after earlier
committed file moves and host changes; it was synchronized to the actual source
and its self-test and ratchet pass. No skill-specific routing branch was added.

## Validation

- Control console: 609 tests and TypeScript checks pass. The locked body-parser
  dependency was patched from 1.20.8 to 1.20.9; the high-severity audit gate passes.
- Media downloader: 252 Python tests pass.
- Pi companion: 52 tests pass.
- Release packaging: 34 tests pass.
- Product identity: self-test, zero-literal inventory, and both-brand UI build pass.
- Skill manifests: 48 manifests / 9 schemas; on-demand and cross-platform checks pass.
- Website: 92 backend tests and 55 media UI tests pass; production build passes.
- Production API: download, independent transcription and creator analysis,
  cache reuse, owner isolation, cross-tab lock, and ranged media download pass.
- Native MiniMax checks cover suitable teaching content, music-only content,
  visually dependent fragments, and unusable text. A real non-speech ASR fixture
  returns a refusal for creator analysis and a no-speech error for transcription.
- Browser checks cover Chinese/English, light/dark, desktop/mobile and refusal
  presentation without empty analysis cards or retry loops.
- Synthetic Chromium cookies survive fresh task profiles on Linux and Pi;
  expired cookies are excluded. Real official QR images decode at native and
  companion display sizes. This does not imply a personal account was signed in.

The first integrated runtime run completed 4,593 tests with six failures and
three ignored tests. Five failures were missing Playwright in the isolated
worktree; all nine browser session tests pass after supplying that dependency.
The remaining test omitted the authentication schema required by indexed recall
and used English retrieval fixtures for a Chinese photo request. The fixtures
now initialize that schema and the owner-scoped KB namespace, use a matching
KB source reference and Chinese terms, and retain all isolation checks. The test
checks recalled facts and structured KB evidence separately from the 900-character
context budget; versioned JSON evidence is not required to fit a second time in
the bounded text projection.
All nine browser session rechecks and the corrected photo-memory contract test
pass. Combined runtime coverage is 4,599 passed / 3 ignored. A final full run and
the exact platform release/deployment revisions are recorded by the operator in
the installation upgrade evidence after the signed release workflows.

Pi is temporarily admitted only for the requested live test. It must be removed
from the media application allowlist afterwards while retaining its general NNI
membership and updated runtime.
