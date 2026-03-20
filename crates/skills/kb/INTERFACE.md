# kb Interface Spec

## Capability Summary

`kb` is a local namespace-based knowledge retrieval layer.

Actions:
- `ingest`: build/update namespace index from local files
- `search`: keyword retrieval with BM25-like scoring and filters

## Action: `ingest`

### Input
- `action` (required): `ingest`
- `namespace` (required, string)
- `paths` (required, string[]): file/dir paths to index
- `chunk_size` (optional, integer, default `1200`)
- `overwrite` (optional, bool, default `false`)
- `file_types` (optional, string[]): extension whitelist, e.g. `["md","txt","json"]`
- `max_file_size` (optional, integer, default `2097152`)

### Behavior
- `overwrite=true`: rebuild namespace from scratch
- `overwrite=false`: incremental update by path + mtime + size
- per-doc metadata: `path`, `file_type`, `mtime_epoch`, `size`, `chunks`
- per-chunk metadata: `chunk_id`, `offset`, `path`, `file_type`, `mtime_epoch`

### Output
- `status`: `ok|error`
- `summary`
- `stats`:
  - `ingested_docs`
  - `removed_docs`
  - `total_docs`
  - `total_chunks`
  - `skipped_files`
  - `warnings[]`

## Action: `search`

### Input
- `action` (required): `search`
- `namespace` (required, string)
- `query` (required, string)
- `top_k` (optional, integer, default `5`)
- `filters` (optional, object):
  - `path_prefix` (optional)
  - `file_type` (optional)
  - `time_from` (optional, epoch seconds)
  - `time_to` (optional, epoch seconds)
- `min_score` (optional, float, default `0`)

### Output
- `status`: `ok|error`
- `hits[]`:
  - `chunk_id`
  - `path`
  - `file_type`
  - `offset`
  - `text`
  - `score`
  - `hit_terms[]`
  - `score_reason`
  - `metadata`
- `summary`
- `stats`

## Notes
- Retrieval score is BM25-style over chunked text.
- Results are fully traceable to source file/chunk metadata.
- If namespace is missing, returns explicit error.
