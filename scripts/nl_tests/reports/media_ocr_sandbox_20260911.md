# Media OCR Sandbox Regression

## Incident

Task `ea3045e0-da3b-44bc-a295-583e399ae88a` downloaded its media and first frame successfully. Image vision rejected two malformed provider responses. The subsequent local OCR action was denied with `sandbox_workspace_credential_denied`. The final reply incorrectly attributed that later denial to the already completed download.

## Changes

- The admitted `media_download.ocr` action does not require provider credentials. Optional review uses the host-granted internal LLM gateway.
- Runner action projection keeps an explicitly granted LLM capability when network access permits it, independently of raw credential access. It does not add undeclared capabilities.
- An action with `credential_access=false` does not select or inject a provider connection/key, secret broker values, or credential fallback. Read-only/network-disabled actions retain their existing restrictions.
- The verifier response contract includes bounded action identities, status evidence, and the current block reason, without copying tool arguments, outputs, or secrets. Composer and validator prompts distinguish completed steps from the blocked step.
- Existing skill-owned review still preserves raw OCR and the TXT artifact on missing gateway, provider failure, empty output, or integrity-check failure.
- Numeric integrity separates Unicode decimal runs from other numeric symbols. Spacing changes around numeric units no longer reject a valid review; changed digits/units and merged or split decimal runs remain rejected. No language-specific phrase matching is added.
- Runner helpers moved into the existing support module. Production runner: 1,997 lines. Tests remain separate.

## Offline Verification

- 110 media-download Python tests passed, including OCR review/fallback, numeric integrity, text and TXT delivery, multilingual preservation, and platform behavior.
- 101 focused Rust tests passed: runner 33; prepare-round 11; verifier permissions 10; policy tests 28; credential fallback 5; bounded verifier recovery 13; admin YOLO host-policy test 1.
- Product identity self-test/inventory and two-brand UI builds passed.
- Skill storage ownership self-test/inventory passed.
- Natural-language hard-match scan: no unknown or legacy findings. Runtime hard-reply diff scan: no candidates.
- Policy token, on-demand build and skill-doc checks passed; `git diff --check` passed.
- Repository-wide long-file check still reports 11 pre-existing violations outside this change. This change removes the runner violation and does not grow the other oversized files.
- The identity UI build retains existing bundle-size notices. No Rust warnings were emitted by the focused test build.

## Local Acceptance

Case source: `scripts/nl_tests/cases/media_ocr_sandbox_regression_20260911.txt`.

- [x] Media package 0.3.31 installed through the normal Skill Store admission path, operation `3254dbaa-f178-4e32-97cc-77fed7119070`, registry generation 146. Receipt digest: `639e1fdc0cc5311bc60d9fbd6f0338d81cbbfe73fa08edc641f19600957d093d`.
- [x] Release core build passed without Rust warnings; local restart completed with the binary built at 2026-09-11 22:14:10 +08:00.
- [x] English local-OCR request on the retained incident image, including model review and TXT delivery.
- [x] Chinese original-link image-text request, with local OCR fallback available and no sandbox changes.

### English Local OCR

- Final run: `20260911_222716`; task: `81a75e3f-44bb-429c-9d69-766e60f2bfcb`; status `succeeded`; duration 109 seconds; 5 logged LLM responses using MiniMax-M3, including the skill's internal-gateway review.
- The first model turn supplied an invalid filesystem-stat argument. The loop continued to load the skill and execute OCR; the initial error is retained, not counted as a successful tool call.
- Structured pre/post-tool observations report `media_download.ocr` allowed and completed successfully at global step 3. The skill performed no new download or audio transcription.
- Reviewed TXT: 694 bytes; SHA-256 `74e4eec94b138c052aa544e14eee45b885ba02147ec281b1366c539b4f6fe727`. Raw OCR backup: 803 bytes. Authenticated task-artifact download returned HTTP 200 and matched the recorded size and hash.
- Artifact directory: `.agent-runtime/artifacts/skill-invocations/81a75e3f-44bb-429c-9d69-766e60f2bfcb/media_download/c9de9aaf-9f75-4c1b-8b47-578d95ad7031/`.
- Run log: `scripts/nl_suite_logs/media_ocr_sandbox_20260911/20260911_222716/run.log`. Raw numbered LLM records remain in `logs/model_io.log` under the task ID.

### Chinese Original Link

- Run: `20260911_221517`; task: `4e9bbf95-d473-492f-9942-df5aec27b5f9`; status `succeeded`; duration 205 seconds. Four host-native LLM turns are recorded in `model_io.log`; the direct vision-provider request is outside this host-log count.
- Download and first-frame image recognition succeeded. Vision succeeded on this run, so it does not prove a live vision-provider failure followed by OCR; local OCR and review-failure fallback were exercised separately.
- TXT: `image_text_ai.txt`, 726 bytes; SHA-256 `42834e4ef2bc1f4691db049f3fbadb70c935fe4f6fbb5dfe9997973fc77ed0b9`. Authenticated download returned HTTP 200 with matching bytes/hash. The final task contained inline text and the TXT artifact.

### Retests And Limits

- Initial task `c332a752-0315-4e0e-b3ee-2bc05e47e6cf` was canceled because its fixture pointed into another task's private artifact directory. That isolation remains intact. The test operator copied the retained image into the test-input directory and corrected the case path; no sandbox exception was added. The mixed initial run exits nonzero and is not presented as an all-pass run.
- Intermediate task `a8185baf-bba9-403d-adb1-3299f9107c12` succeeded with 6 logged LLM responses and raw-text fallback. Its review exposed the numeric-spacing false positive fixed in 0.3.31. The final run above verifies successful review.
- This acceptance checks execution, review, numeric integrity, and downloadable artifacts, not perfect OCR accuracy. The source's uncertain trailing OCR fragment remains. The final model also speculated that it was a watermark; that interpretation is unsupported and is not an acceptance claim.
- Task-summary action identities showed a separate projection inconsistency: requested action/capability fields can attach to a different global step after multiple rounds. Execution observations and artifacts were cross-checked independently. This and direct-provider trace completeness are recorded in `plan/queued/media_ocr_observability_followup_20260911.md`, not silently marked fixed.

Only local Linux and the common task API/artifact path are exercised in this incident run; no new WeChat platform delivery, remote deployment, or macOS/Raspberry Pi execution is claimed. Test artifacts and build caches are retained.
