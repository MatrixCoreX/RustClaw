## code_index - incremental repository intelligence

Use registry leaf capabilities through
`{"type":"call_capability","capability":"<leaf>","args":{...}}`.
`code_index` returns parser-derived machine evidence and bounded source range
handles; it does not infer repository intent from user prose.

## Capability

- Incrementally index repository source files.
- Search parser-derived Rust symbols.
- Find exact definitions and references.
- List tests and estimate changed-file impact.
- Retrieve compact code context by structured symbol/path relevance.

## Actions

- `refresh`
- `search_symbols`
- `find_definitions`
- `find_references`
- `list_tests`
- `changed_impact`
- `retrieve_context`

## Parameter Contract

| Action | Required | Optional |
| --- | --- | --- |
| `refresh` | - | `max_files` |
| `search_symbols` | `query` | `mode`, `cursor`, `max_results`, `max_files` |
| `find_definitions` | `symbol` | `cursor`, `max_results`, `max_files` |
| `find_references` | `symbol` | `cursor`, `max_results`, `max_files` |
| `list_tests` | - | `path`, `symbol`, `cursor`, `max_results`, `max_files` |
| `changed_impact` | - | `paths`, `cursor`, `max_results`, `max_files` |
| `retrieve_context` | `symbols` or `paths` | `mode`, `context_lines`, `cursor`, `max_results`, `max_files` |

`mode` is a machine enum: `exact|prefix|contains`. `symbols` and `paths`
accept a string or string array. Paths must be workspace-relative.
Use `page.next_cursor` for exact continuation; do not invent a cursor.

## Planning Rules

- For coding tasks, prefer `code.find_definitions`,
  `code.find_references`, and `code.retrieve_context` over repeatedly reading
  broad file ranges.
- Extract explicit symbol/path candidates in the model plan and pass them as
  structured arguments. Never put the whole natural-language task into
  `query`, `symbol`, or `path`.
- Use `code.changed_impact` before selecting focused tests after a patch.
- Treat results as conservative parser evidence. Rust symbols use `syn`;
  other recognized source languages currently provide file inventory only.
- Check `summary.scan_complete`, `summary.parse_status_counts`, and
  `provenance` before treating an empty page as exhaustive.
- A missing definition/reference is not proof that a symbol does not exist in
  an unsupported language, generated file, dynamic macro expansion, or
  excluded directory.
- Use returned `range_handle` values for narrow follow-up reads. Preserve exact
  path and line fields.
- Ordinary explanations must be synthesized from the structured result;
  runtime does not own user-language rendering.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
