# Prompt Schemas (§3.5c)

This directory holds JSON Schemas (Draft 2020-12) describing the **expected
output shape** of LLM prompts whose responses are parsed into Rust structs.

## Why

The `IntentNormalizerOut` / `PlanResult` / `FinalizerOut` style of LLM-driven
contract is currently encoded twice:

1. Implicitly, in the prompt body (`prompts/layers/overlays/*.md`).
2. Explicitly, in `serde::Deserialize` derives + `parse_*` enum decoders.

When the prompt and the parser drift — for example a new `target_scope` enum
value gets added to the prompt but the parser still maps it to `Unknown` —
the symptom is a silent fallback that's almost impossible to attribute.

A schema file gives us a **single auditable source of truth** that:

- Documents which fields the prompt is allowed to emit (and which are required).
- Lists every accepted enum value, with the parser's canonical mapping
  cross-referenced via a drift unit test.
- Makes future schema-driven validators / repair passes a one-liner away
  (`jsonschema::validator_for(...)` etc.) — even though we **do not** wire
  one in yet to keep binary size flat.

## Authoring convention

For a prompt at `prompts/layers/overlays/<name>.md`:

1. Add `prompts/schemas/<name>.schema.json` with `$schema`, `$id`, `title`,
   `description`, `type: object`, `required`, and per-field `properties`.
2. Mirror every `#[serde(default)] field: T` in the corresponding Rust
   `struct ...Out` under `properties`. Use `enum: [...]` for tokens that
   the parser dispatches on (case-insensitive whitelist).
3. Add a unit test next to the parser, modelled after
   `intent_normalizer_schema_drift` in
   `crates/clawd/src/intent_router.rs`, that:

   - Loads the schema via `include_str!`.
   - Parses it as `serde_json::Value`.
   - Asserts every parser field appears under `properties`.
   - Asserts every declared `enum` token, when fed to the parser's
     `parse_*` function, returns a non-default variant (i.e. is actually
     recognized rather than swallowed by the catch-all arm).

## Currently authored

| Schema | Backing prompt | Backing parser |
|--------|----------------|----------------|
| `intent_normalizer.schema.json` | `prompts/layers/overlays/intent_normalizer_prompt.md` | `crates/clawd/src/intent_router.rs::IntentNormalizerOut` (drift test: `intent_normalizer_schema_drift`) |

## Future runtime use (not yet wired)

The schemas are currently documentation + drift guards only. A potential
future step (out of §3.5c-小切口 scope) is to add a thin
`prompt_utils::validate_against_schema<T>(raw, schema_id) -> Result<T, _>`
helper that runs schema validation **before** `serde_json::from_str::<T>`,
giving us actionable error messages ("`semantic_kind=oops_status` not in
declared enum") instead of opaque `missing field` failures.
