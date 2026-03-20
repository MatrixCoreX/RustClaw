# doc_parse Interface Spec

## Capability Summary

`doc_parse` parses local documents into structured output.

Supported formats:
- `md`, `txt`, `html`
- `pdf` (via `pdftotext`/`pdfinfo` when available)
- `docx` (paragraph/title/table extraction from OOXML)

## Action

### `parse_doc`

Parse one local file and return:
- normalized plain text
- `sections` (`id/title/level/content`)
- `tables` (`id/header/rows`)
- `metadata` (optional)

## Input Parameters

- `action` (required, string): `parse_doc`
- `path` (required, string): local file path
- `mode` (optional, string, default `auto`): `auto|text_only`
- `max_chars` (optional, integer, default `12000`): text truncation cap
- `include_metadata` (optional, bool, default `true`)
- `page_range` (optional, string/object): PDF page range, e.g. `"1-5"` or `{ "start": 1, "end": 5 }`
- `table_mode` (optional, string, default `basic`): `basic|strict`

## Output Schema

The skill returns JSON in `text` with:

- `status`: `ok|error`
- `text`: normalized text
- `sections`: array of section objects
- `tables`: array of table objects
- `metadata`: object or `null`
  - `title`, `pages`, `type`, `path`, `encoding`, `truncated`, `truncation_notice`, `page_range_applied`
- `error_code`: nullable string (`NOT_FOUND|DEPENDENCY_MISSING|UNSUPPORTED_FORMAT|PARSE_FAILED|INVALID_ACTION`)
- `error`: nullable string

## Behavior Rules

- Never fabricate content.
- If parser dependency is missing (for PDF), return explicit error.
- For large documents, enforce `max_chars` and set truncation metadata.
- For non-UTF8 text, use lossy fallback decoding.
- `table_mode=strict` drops rows that do not match header width.

## Example Request

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

