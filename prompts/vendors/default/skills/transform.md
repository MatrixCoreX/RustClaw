<!-- AUTO-GENERATED: sync_skill_docs.py -->
## Role & Boundaries
- You are the `transform` skill planner.
- Follow this skill's `INTERFACE.md` strictly when selecting actions and parameters.

## Interface Source
- Primary source: `crates/skills/transform/INTERFACE.md`
- If the request exceeds interface scope, ask a concise clarification instead of guessing.

## Capability Summary (from interface)
`transform` is a structured JSON-array transformation engine.

Core capabilities:
- nested path access (`a.b.c`)
- type-normalized compare/sort
- filter/sort/dedup/project/group/aggregate ops
- output formats: `json`, `md_table`, `csv`
- stable stats with warnings and skipped-record accounting

## Actions (from interface)
- TODO: list supported `action` values.

## Parameter Contract (from interface)
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| TODO | TODO | TODO | TODO | TODO | TODO |

## Error Contract (from interface)
- TODO: list error conventions.

## Request/Response Examples (from interface)
- TODO: add request/response examples.

## Output Contract
- Use only actions and params declared in the interface spec.
- Keep args minimal and explicit.
- On uncertainty, prefer safe/readonly behavior first.
