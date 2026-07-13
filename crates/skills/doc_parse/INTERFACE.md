# doc_parse Interface Spec

## Capability Summary

`doc_parse` parses local documents into structured output.

Planner selection guidance:
- Use `doc_parse` when the request needs semantic evidence from a user/business document: extracting key points, summarizing sections, judging excerpt meaning, collecting paragraphs, reading document structure, parsing tables, or preparing grounded synthesis from a supported document file.
- Prefer `doc_parse` for PDF/docx/html files, markdown or text documents that need key points or section-level synthesis, table/section-aware parsing, long documents, or document-format behavior that `fs_basic.read_text_range` does not model.
- Repository documentation files such as README, release notes, checklists, runbooks, and service notes still belong to `doc_parse` when the request asks to parse, summarize, extract key points, explain sections, or prepare a grounded document synthesis.
- Use `fs_basic.read_text_range` first for source files, prompt markdown, generated skill docs, config-adjacent docs, exact line ranges, raw bounded excerpts, previews, or small text files when the user asks for raw text rather than document understanding; synthesize the user-facing answer from that bounded text.
- Use generic filesystem/text tools when the request is explicitly about raw bytes, exact line ranges, path facts, file listings, or structured JSON/TOML/YAML field extraction.
- `doc_parse` only parses and exposes grounded document evidence. It does not have separate `summarize`, `extract`, `judge`, or rewrite actions; perform those user-facing transformations in the agent response or a later synthesis step using the parsed output.

Supported formats:
- `md`, `txt`, `html`
- `pdf` (via `pdftotext`/`pdfinfo` when available)
- `docx` (paragraph/title/table extraction from OOXML)

## Actions

- `parse_doc`

Backward-compatible action aliases:
- `parse` is accepted by the skill and normalized to `parse_doc`.

Parse one local file and return:
- normalized plain text
- `sections` (`id/title/level/content`)
- `tables` (`id/header/rows`)
- `metadata` (optional)
- structured `extra` evidence fields for runtime verification
- For summary/extraction/judgment requests, call `parse_doc` first, then synthesize the requested answer from the returned `text` / `sections` / `tables`.

## Parameter Contract

- `action` (required, string): `parse_doc`
- `path` (required, string): local file path
- `mode` (optional, string, default `auto`): `auto|text_only`
- `max_chars` (optional, integer, default `12000`): text truncation cap
- `include_metadata` (optional, bool, default `true`)
- `page_range` (optional, string/object): PDF page range, e.g. `"1-5"` or `{ "start": 1, "end": 5 }`
- `table_mode` (optional, string, default `basic`): `basic|strict`

## Error Contract

- `INVALID_ACTION`: unsupported `action`.
- `NOT_FOUND`: target file does not exist.
- `DEPENDENCY_MISSING`: required parser dependency is missing, especially for PDF parsing.
- `UNSUPPORTED_FORMAT`: file type is not supported by the skill.
- `PARSE_FAILED`: parsing failed after format detection and dependency checks.

## Request/Response Examples
### Example 1

Request:
```json
{
  "request_id": "doc-1",
  "args": {
    "action": "parse_doc",
    "path": "/tmp/spec.docx",
    "max_chars": 20000,
    "include_metadata": true,
    "table_mode": "basic"
  }
}
```

Response:
```json
{
  "request_id": "doc-1",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"text\":\"...\",\"sections\":[],\"tables\":[],\"metadata\":{\"type\":\"docx\"},\"error_code\":null,\"error\":null}",
  "error_text": null
}
```

Returned JSON inside `text` contains:

- `status`: `ok|error`
- `text`: normalized text
- `sections`: array of section objects
- `tables`: array of table objects
- `metadata`: object or `null`
  - `title`, `pages`, `type`, `path`, `encoding`, `truncated`, `truncation_notice`, `page_range_applied`
- `error_code`: nullable string (`NOT_FOUND|DEPENDENCY_MISSING|UNSUPPORTED_FORMAT|PARSE_FAILED|INVALID_ACTION`)
- `error`: nullable string

Top-level `extra` contains stable machine-readable evidence:

- `action`: `parse_doc`
- `status`: `ok|error`
- `path`: parsed document path when known, otherwise the requested path
- `requested_path`: requested `args.path`
- `content_excerpt`: bounded excerpt from parsed document text for evidence coverage
- `content_excerpt_truncated`: whether `content_excerpt` was capped
- `text_length_chars`: parsed text length in Unicode scalar values
- `sections_count`: number of parsed sections
- `tables_count`: number of parsed tables
- `metadata`: compact metadata copy when available
- `error_code`: nullable machine error code

- Never fabricate content.
- If parser dependency is missing (for PDF), return explicit error.
- For large documents, enforce `max_chars` and set truncation metadata.
- For non-UTF8 text, use lossy fallback decoding.
- `table_mode=strict` drops rows that do not match header width.
