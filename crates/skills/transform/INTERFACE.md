# transform Interface Spec

## Capability Summary

`transform` is a structured JSON-array transformation engine.

Core capabilities:
- nested path access (`a.b.c`)
- type-normalized compare/sort
- filter/sort/dedup/project/group/aggregate ops
- output formats: `json`, `md_table`, `csv`
- stable stats with warnings and skipped-record accounting

## Action

### `transform_data`

## Input Parameters

- `action` (required, string): `transform_data`
- `data` (required, array): input records
- `ops` (optional, array): ordered operations
- `output_format` (optional, string, default `json`): `json|md_table|csv`
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

#### 4. `project`
- fields:
  - `op`: `project`
  - `fields` optional (path list; key defaults to leaf field name)
  - `mappings` optional (explicit rename):
    - item shape: `{ "from": "a.b", "to": "alias_name" }`

#### 5. `group`
- fields:
  - `op`: `group`
  - `by` required (array; or `field` fallback)
  - `aggregations` optional (default count)

#### 6. `aggregate`
- fields:
  - `op`: `aggregate`
  - `aggregations` required

### Aggregations

Aggregation item fields:
- `op` required: `count|sum|avg|min|max`
- `field` optional (`count` can omit)
- `name` optional output alias

## Output Schema

The skill returns JSON in `text` with:

- `status`: `ok|error`
- `error_code`: nullable (`INVALID_ACTION|TRANSFORM_FAILED`)
- `error`: nullable message
- `result`: transformed array
- `formatted`: nullable string (for `md_table`/`csv`)
- `stats`:
  - `input_count`
  - `output_count`
  - `skipped_records`
  - `warnings` (array)

## Behavior Notes

- strict mode default is `true`.
- in non-strict mode, unsupported ops are skipped with warnings.
- nested path resolution returns null when path not found.
- numeric/bool/string comparisons are normalized where possible.
- output column order is stable by first-seen key order.

## Example Request

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
