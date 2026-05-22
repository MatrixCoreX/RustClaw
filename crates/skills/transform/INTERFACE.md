# transform Interface Spec

## Capability Summary

`transform` is a structured data transformation engine.

Planner selection guidance:
- Use `transform` when the request supplies or points to structured records and asks to sort, filter, deduplicate, rename keys, project fields, group, aggregate, or render the result as JSON, markdown table, or CSV.
- Inline JSON arrays/objects are valid input; pass them directly as `data` instead of answering from chat when this skill is enabled.
- Inline CSV is valid input; pass it as `csv_text` and set `output_format` for the requested rendering.
- Preserve requested output formats such as markdown table by setting `output_format="md_table"`.

Core capabilities:
- nested path access (`a.b.c`)
- type-normalized compare/sort
- filter/sort/dedup/rename/project/group/aggregate ops
- output formats: `json`, `md_table`, `csv`
- stable stats with warnings and skipped-record accounting

## Actions

- `transform_data`

## Parameter Contract

- `action` (required, string): `transform_data`
- `data` (required unless `csv_text` is used, array or object): input records; an object is treated as one record
- `csv_text` (required unless `data` is used, string): CSV text with a header row
- `ops` (optional, array): ordered operations
- `output_format` (optional, string, default `json`): `json|md_table|csv`
- `result_shape` (optional, string, default `array`; object input defaults to `single_object`): `array|single_object|scalar`
- `strict` (optional, bool, default `true`): strict mode (unsupported/malformed ops fail)
- `null_policy` (optional, string, default `keep`): `keep|drop|zero`

### Supported Ops

#### 1. `filter`
- fields:
  - `op`: `filter`
  - `field` (or `path`) required
  - `cmp` optional: `eq|ne|gt|gte|lt|lte|contains|in|exists` (default `eq`)
  - `value` optional

#### 2. `sort`
- fields:
  - `op`: `sort`
  - `by` (or `field`) required
  - `order` optional: `asc|desc` (default `asc`)
  - `nulls` optional: `first|last` (default `last`)

#### 3. `dedup`
- fields:
  - `op`: `dedup`
  - `field` optional
  - `fields` optional (preferred for multi-key)

#### 4. `rename`
- fields:
  - `op`: `rename`
  - `from` required for one mapping
  - `to` required for one mapping
  - `mappings` optional for multiple mappings:
    - item shape: `{ "from": "old_name", "to": "new_name" }`
- `rename` preserves all other fields.

#### 5. `project`
- fields:
  - `op`: `project`
  - `fields` optional (path list; key defaults to leaf field name)
  - `mappings` optional (explicit rename):
    - item shape: `{ "from": "a.b", "to": "alias_name" }`

#### 6. `group`
- fields:
  - `op`: `group`
  - `by` required (array; or `field` fallback)
  - `aggregations` optional (default count)

#### 7. `aggregate`
- fields:
  - `op`: `aggregate`
  - `aggregations` required

### Aggregations

Aggregation item fields:
- `op` required: `count|sum|avg|min|max`
- `field` optional (`count` can omit)
- `name` optional output alias

## Error Contract

- `INVALID_ACTION`: unsupported `action` value.
- `TRANSFORM_FAILED`: invalid input data or unsupported/malformed operations in strict mode.
- In non-strict mode, unsupported ops should be skipped with warnings instead of hard failure where possible.

## Request/Response Examples

### Example 1

Request:
```json
{
  "request_id": "tf-1",
  "args": {
    "action": "transform_data",
    "strict": true,
    "null_policy": "keep",
    "output_format": "json",
    "data": [
      {"user":{"name":"A"},"score":"10"},
      {"user":{"name":"B"},"score":"20"}
    ],
    "ops": [
      {"op":"filter","field":"score","cmp":"gte","value":15},
      {"op":"project","mappings":[{"from":"user.name","to":"name"},{"from":"score","to":"score"}]}
    ]
  }
}
```

Response:
```json
{
  "request_id": "tf-1",
  "status": "ok",
  "text": "{\"status\":\"ok\",\"result\":[{\"name\":\"B\",\"score\":\"20\"}],\"formatted\":null,\"stats\":{\"input_count\":2,\"output_count\":1,\"skipped_records\":0,\"warnings\":[]},\"error_code\":null,\"error\":null}",
  "error_text": null
}
```

Returned JSON inside `text` contains:

- `status`: `ok|error`
- `error_code`: nullable (`INVALID_ACTION|TRANSFORM_FAILED`)
- `error`: nullable message
- `result`: transformed array
- `formatted`: nullable string (for `md_table`/`csv`)
- `output`: formatted string, result array, single object, or scalar according to `output_format` / `result_shape`
- `stats`:
  - `input_count`
  - `output_count`
  - `skipped_records`
  - `warnings` (array)

- strict mode default is `true`.
- in non-strict mode, unsupported ops are skipped with warnings.
- nested path resolution returns null when path not found.
- numeric/bool/string comparisons are normalized where possible.
- output column order is stable by first-seen key order.
