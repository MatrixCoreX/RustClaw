# office_workspace Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this contract aligned with the pure Rust OOXML implementation.

## Capability Summary
- Inspect, validate, create, preview changes to, and edit DOCX, XLSX, and PPTX artifacts through structured operations.
- Office files are treated as untrusted ZIP/XML packages. Macros, formulas, field codes, links, embedded objects, and external relationships are never executed.
- Reads are bounded and cursor-paged. Writes are copy-on-write and transactional unless an approved in-place operation explicitly requests otherwise.
- Use `doc_parse` for general PDF/HTML/Markdown/text extraction. Use this skill when Office structure, ranges, creation, editing, package validation, or artifact revisions matter.

## Config Entry Points
- `WORKSPACE_ROOT`: base directory for relative input and output paths.
- `OFFICE_MAX_ZIP_ENTRIES`, `OFFICE_MAX_MEMBER_BYTES`, `OFFICE_MAX_TOTAL_BYTES`, and `OFFICE_MAX_EXPANSION_RATIO`: optional package safety limits.
- Optional rendering/export uses a separately detected adapter. Structural read/write does not require LibreOffice or a platform-native Office application.

## Actions
- Common: `office.inspect`, `office.validate`.
- Word: `word.read`, `word.find`, `word.preview_create`, `word.create`, `word.preview_edit`, `word.edit`.
- Spreadsheet: `spreadsheet.inspect`, `spreadsheet.read_range`, `spreadsheet.preview_create`, `spreadsheet.create`, `spreadsheet.preview_edit`, `spreadsheet.edit`.
- Presentation: `presentation.read`, `presentation.preview_create`, `presentation.create`, `presentation.preview_edit`, `presentation.edit`.
- Rendering: `office.render_status`, `office.render`.

## Parameter Contract
| Action family | Param | Required | Type | Description |
|---|---|---|---|---|
| inspect/read/validate | `path` | yes | string(path) | Existing DOCX/XLSX/PPTX source. |
| bounded read | `cursor` | no | string | Opaque cursor bound to source hash. |
| bounded read | `offset`, `limit` | no | integer | Bounded object page; maximum limit is 1000. |
| `spreadsheet.read_range` | `sheet`, `range` | yes | string | Exact worksheet name and A1 range. |
| preview/create | `output_path` | yes | string(path) | Intended output artifact path. |
| preview/create | `template_path` | no | string(path) | Read-only template with matching Office format. |
| preview/create/edit | `operations` | yes | array(object) | Ordered, structured mutation batch. |
| preview/edit | `source_path`, `source_sha256` | yes | string | Existing artifact and exact precondition hash. |
| create/edit | `overwrite` | no | boolean | Allow replacing an existing output after policy approval. |
| edit | `in_place` | no | boolean | Explicitly request approved in-place replacement with backup. |

Operation objects use an `op` machine token and format-specific fields. Plain
spreadsheet text remains text even when it begins with `=`, `+`, `-`, or `@`;
only `value_type=formula` creates a formula. Object selectors use stable block,
table, cell/range, slide, or shape identifiers from a matching source revision.

## Error Contract
- `invalid_input`, `missing_argument`, `unsupported_action`, `unsupported_operation`.
- `source_unavailable`, `unsupported_format`, `format_mismatch`, `missing_package_part`, `malformed_package`, `malformed_xml`.
- `path_traversal`, `encrypted_package`, `macro_enabled_package`, `package_limit_exceeded`, `package_expansion_rejected`.
- `invalid_cursor`, `cursor_out_of_range`, `invalid_cell_range`, `worksheet_not_found`.
- Mutation errors include `source_conflict`, `ambiguous_selector`, `output_exists`, `transaction_failed`, `validation_failed`, and `renderer_unavailable`.
- Errors return stable `extra.error_code`, `message_key`, `retryable`, and structured `details`. Runtime logic must not parse `error_text`.

## Structured Evidence Contract
- `extra` is a versioned `OfficeArtifactEnvelope`; it is the machine evidence authority.
- `source`: path, SHA-256, byte length, and revision.
- `package`: member counts, bounded sizes, external relationships, macros, and embedded-object observations.
- `document_blocks`, `tables`, `workbook.sheets[].cells`, and `presentation.slides`: bounded format structure with stable object refs.
- `warnings`: untrusted or preservation observations with machine codes.
- `cursor`: returned/total counts and an optional source-bound next cursor.
- `operation_log`, source/output hashes, changed refs, preservation report, validation, and artifacts describe mutation results.
- `text` is only a compact fallback and must not drive routing, retry, success, or final delivery.

## Request/Response Examples
### Example 1
Request:
```json
{"request_id":"office-read-1","args":{"action":"spreadsheet.read_range","path":"reports/budget.xlsx","sheet":"Summary","range":"A1:F40","limit":100}}
```
Response:
```json
{"request_id":"office-read-1","status":"ok","text":"{\"schema_version\":1,\"format\":\"xlsx\"}","error_text":null,"extra":{"schema_version":1,"format":"xlsx","source":{"path":"reports/budget.xlsx","sha256":"...","revision":"sha256:..."},"workbook":{"sheets":[{"name":"Summary","cells":[]}]},"validation":{"valid":true}}}
```

### Example 2
Request:
```json
{"request_id":"office-preview-1","args":{"action":"word.preview_create","output_path":"out/report.docx","operations":[{"op":"add_heading","text":"Quarterly report","level":1},{"op":"add_paragraph","text":"Verified observations follow."}]}}
```
Response:
```json
{"request_id":"office-preview-1","status":"ok","text":"{\"preview\":true}","error_text":null,"extra":{"schema_version":1,"preview":true,"normalized_operations":[{"id":"op_1","op":"add_heading"},{"id":"op_2","op":"add_paragraph"}],"writes_performed":false}}
```

### Example 3
Request:
```json
{"request_id":"office-edit-1","args":{"action":"presentation.edit","source_path":"deck.pptx","source_sha256":"...","output_path":"deck-revised.pptx","operations":[{"op":"replace_slide_text","slide_id":"slide_2","match":"Old title","text":"New title"}]}}
```
Response:
```json
{"request_id":"office-edit-1","status":"ok","text":"{\"format\":\"pptx\"}","error_text":null,"extra":{"schema_version":1,"format":"pptx","operation_log":[{"id":"op_1","operation":"replace_slide_text","object_refs":["slide_2"],"status":"applied"}],"validation":{"valid":true},"artifacts":[{"kind":"office_output","path":"deck-revised.pptx","sha256":"..."}]}}
```

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Interpret Chinese colloquial phrasing by capability semantics and requested task shape, not by a fixed phrase list.
- Preserve user-specified Office text, worksheet names, and output language exactly unless an explicit transformation was requested.
