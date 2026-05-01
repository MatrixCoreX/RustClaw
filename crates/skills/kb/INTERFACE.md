# kb Interface Spec

## Capability Summary

`kb` is a local namespace-based knowledge retrieval layer for user-managed documents.

Use it when the user wants RustClaw to:
- build a searchable knowledge base from local files or directories
- add/update indexed materials under a named namespace
- search previously indexed documents by natural-language query
- view which namespaces already exist
- inspect basic namespace/library statistics

Current runtime notes:
- `ingest` still keeps the namespace JSON index under `data/kb/` for compatibility.
- `ingest` also tries to sync chunk rows into the unified retrieval index used by `clawd`, so later route/planner/execution recall can reuse the same document chunks.
- document KB is workspace-scoped by default; it is not tied to a single chat.

Natural-language intent mapping:
- Requests that semantically mean "add documents to an indexed knowledge base" should use `ingest` when required args are available; examples are illustrative only.
- Requests that semantically mean "retrieve from an indexed knowledge base" should use `search` when the namespace is known or uniquely bound; examples are illustrative only.
- `kb` is for indexed retrieval over previously ingested local content, not for direct file reading, ad hoc filesystem search, or open-ended chat.

## Actions

- `ingest`: build/update namespace index from local files
  - best for requests like `把 docs/ 建成知识库`、`把这批文档收录到 faq 库`
  - caller should provide an explicit `namespace`
  - if the user names a target folder but not a namespace, prefer a short namespace derived from that folder name only when it is obvious and unambiguous; otherwise ask a concise clarification
- `search`: keyword retrieval with BM25-like scoring and filters
  - best for requests like `去 docs 知识库里搜部署步骤`、`在 faq 库里查退款`
  - caller should provide both `namespace` and `query`
  - if the user asks to search a knowledge base but does not identify which namespace to use, ask a concise clarification unless current context already binds exactly one namespace
- `list_namespaces`: inspect which namespaces are already available
  - best for requests like `看看现在有哪些知识库`、`列出所有资料库`
  - does not require `namespace`
- `stats`: inspect namespace-level or global KB stats
  - best for requests like `看 docs 知识库的统计`、`看看知识库现在一共有多少库`
  - `namespace` is optional; omitted means global KB stats
- Returned `text` is a JSON string payload describing the inner skill result.

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| `ingest` | `action` | yes | string | - | Must be `ingest`. |
| `ingest` | `namespace` | yes | string | - | Namespace to build/update. |
| `ingest` | `paths` | yes | string[] | - | File or directory paths to index. |
| `ingest` | `chunk_size` | no | integer | `1200` | Chunk size for splitting documents. |
| `ingest` | `chunk_overlap` | no | integer | `180` | Overlap between adjacent chunks. |
| `ingest` | `overwrite` | no | bool | `false` | Rebuild namespace from scratch. |
| `ingest` | `file_types` | no | string[] | - | Extension whitelist such as `["md","txt","json"]`. |
| `ingest` | `max_file_size` | no | integer | `2097152` | Skip files larger than this many bytes. |
| `search` | `action` | yes | string | - | Must be `search`. |
| `search` | `namespace` | yes | string | - | Namespace to search. |
| `search` | `query` | yes | string | - | Search query. |
| `search` | `top_k` | no | integer | `5` | Max number of hits. |
| `search` | `filters` | no | object | - | Optional grouped filters object. |
| `search` | `filters.path_prefix` or `path_prefix` | no | string | - | Keep only indexed chunks whose source path starts with this prefix. |
| `search` | `filters.file_type` or `file_type` | no | string | - | Filter by normalized file extension, such as `md` or `json`. |
| `search` | `filters.time_from` or `time_from` | no | integer/string | - | Inclusive lower bound for source file `mtime_epoch`. |
| `search` | `filters.time_to` or `time_to` | no | integer/string | - | Inclusive upper bound for source file `mtime_epoch`. |
| `search` | `min_score` | no | float | `0` | Minimum retrieval score. |
| `list_namespaces` | `action` | yes | string | - | Must be `list_namespaces`. |
| `stats` | `action` | yes | string | - | Must be `stats`. |
| `stats` | `namespace` | no | string | - | Namespace to inspect; omit for global KB stats. |

- `overwrite=true`: rebuild namespace from scratch
- `overwrite=false`: incremental update by path + mtime + size
- per-doc metadata: `path`, `file_type`, `mtime_epoch`, `size`, `chunks`
- per-chunk metadata: `chunk_id`, `offset`, `path`, `file_type`, `mtime_epoch`
- `ingest` prefers Markdown heading / paragraph boundaries when chunking, then falls back to bounded overlapping windows.
- `search` accepts filter fields either nested under `filters` or as top-level aliases; the nested value is checked first.

## Error Contract

- Return explicit error when `namespace`, `paths`, or `query` is missing for the selected action.
- If namespace is missing during `search`, return explicit error rather than empty success.
- Indexing and retrieval failures must be surfaced with readable error text.

## Request/Response Examples

### Example 1

Request:
```json
{
  "request_id": "kb-1",
  "args": {
    "action": "search",
    "namespace": "docs",
    "query": "deployment steps",
    "top_k": 3
  }
}
```

Response:
```json
{
  "request_id": "kb-1",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"hits\":[{\"chunk_id\":\"docs:1\",\"path\":\"README.md\",\"text\":\"...\",\"score\":1.2}],\"summary\":\"1 hit\",\"stats\":{\"top_k\":3}}",
  "error_text": null
}
```

- Retrieval score is BM25-style over chunked text.
- Results are fully traceable to source file/chunk metadata.
- If namespace is missing, returns explicit error.

### Example 2

Request:
```json
{
  "request_id": "kb-2",
  "args": {
    "action": "ingest",
    "namespace": "docs",
    "paths": ["docs", "README.md"],
    "file_types": ["md", "txt"],
    "overwrite": false
  }
}
```

Response:
```json
{
  "request_id": "kb-2",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"namespace\":\"docs\",\"indexed_files\":12,\"updated_files\":3,\"skipped_files\":1,\"summary\":\"docs namespace updated\"}",
  "error_text": null
}
```

### Example 3

Natural-language mapping examples:
- `把 docs/ 建成知识库，命名为 docs` -> `{"action":"ingest","namespace":"docs","paths":["docs/"]}`
- `在 docs 知识库里搜索 telegram 按钮` -> `{"action":"search","namespace":"docs","query":"telegram 按钮"}`
- `列出现在所有知识库` -> `{"action":"list_namespaces"}`
- `看 docs 知识库统计` -> `{"action":"stats","namespace":"docs"}`
- `帮我去知识库里查部署步骤` -> if namespace is not uniquely known, ask a concise clarification for the namespace instead of guessing
