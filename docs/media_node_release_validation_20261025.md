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

## Dependency audit before publication

The remote audit initially reported 31 vulnerability records in the existing
lockfile. The Rust audit job also lacked permission to publish its check; that
job now requests `contents: read` and `checks: write` only for its own results.

The release updates the affected upstream dependencies: k256 0.14.0, teloxide
0.14.1, quick-xml 0.41.0, zip 9.1.0, rustls 0.23.46, rustls-webpki 0.103.16,
quinn-proto 0.11.19, and the 1.0.11 webpki roots/certificates. Cargo's locked audit
reports zero vulnerability records without an advisory ignore list. Existing
upstream maintenance, unsoundness, and yanked-version warnings remain separate
from that exit-status gate; this is not a claim that every dependency is risk-free.

Compatibility coverage includes the fixed Core signature vector and rejection of
DER, high-S and tampered signatures; Telegram polling/webhook configuration,
message IDs, captions, media extraction and error classification; XML text and
entity boundaries in DOCX/XLSX/PPTX; and both ZIP extraction paths rejecting
links and refusing to restore special permission bits. XML parsing does not
resolve external or custom DTD entities. Source package admission and receipt
pinning keep the existing machine contracts.

The pre-upgrade full runtime suite passes 4,599 tests with three ignored tests.
The upgraded dependency set is validated again before publication. When running
tests with a target directory outside the checkout, set
`APP_PRODUCT_IDENTITY_CONFIG` to the absolute path of the checkout's selected
`configs/product_identity.toml`; the runtime intentionally fails when its sole
identity configuration cannot be found.

The upgraded runtime suite passes 4,599 tests with three ignored tests. SDK
passes 95/95; Telegram passes 23/23; Office library and process tests pass 57/57;
document parsing passes 11/11; RSS passes 58 with three pre-existing ignored
network cases. ZIP fixture assertions inspect raw external permission bits as
well as the sanitized ZIP API and extracted filesystem mode.

Telegram tests cover both explicit MIME and explicit null MIME for voice/video.
An upstream limitation shared by the old and new Telegram SDK remains: completely
omitting the optional MIME field can cause the SDK's untagged message decoder to
choose an empty variant. The regression samples include realistic MIME-bearing
messages and null cases; they do not claim to fix that pre-existing decoder gap.

The configuration-template regression removes a duplicate Docker permission
baseline and uses the existing reviewed baseline for both distributions. The
actual registry permissions remain as declared; the duplicate test list had
omitted already-declared health and service-control entries.

Final configuration-template tests pass 22/22, and the CLI harness passes 3/3.
