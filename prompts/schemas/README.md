# Prompt Schemas (§3.5c)

This directory holds JSON Schemas (Draft 2020-12) describing the **expected
output shape** of LLM prompts whose responses are parsed into Rust structs.

## Why

The `IntentNormalizerOut` / `PlanResult` / `FinalizerOut` / `ScheduleIntentOutput` /
`DeliveryTextClassifierOut` style of LLM-driven
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
- Makes future schema-driven validators / repair passes straightforward; the
  current runtime already uses a small in-tree validator for the authored
  schemas instead of pulling in a full external `jsonschema` dependency.

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
| `boundary_envelope.schema.json` | Target boundary context passed from normalizer/front layer into the planner loop | `crates/clawd/src/intent_router_output_types.rs::BoundaryEnvelope` (migration target; guard: `scripts/check_boundary_envelope_schema.py`) |
| `plan_result.schema.json`       | `prompts/layers/overlays/{single_plan_execution,loop_incremental_plan,plan_repair}_prompt.md` | `crates/clawd/src/agent_engine.rs::SinglePlanEnvelope` + `crates/clawd/src/runtime/types.rs::AgentAction` (drift test: `plan_result_schema_drift`) |
| `finalizer_out.schema.json`     | `prompts/layers/overlays/observed_answer_fallback_prompt.md` | `crates/clawd/src/agent_engine/observed_output.rs::ObservedAnswerFallbackOut` (drift test: `finalizer_out_schema_drift`) |
| `delivery_text_classifier.schema.json` | `prompts/layers/overlays/delivery_text_classifier_prompt.md` | `crates/clawd/src/semantic_judge.rs::DeliveryTextClassifierOut` (drift test: `delivery_text_classifier_schema_drift`) |
| `user_response_contract_validator.schema.json` | `prompts/layers/overlays/user_response_contract_validator_prompt.md` | `crates/clawd/src/fallback.rs::UserResponseContractValidationOut` (drift test: `user_response_contract_validator_schema_drift`) |
| `schedule_intent.schema.json`   | `prompts/layers/overlays/schedule_intent_prompt.md` | `crates/clawd/src/runtime/types.rs::ScheduleIntentOutput` + `crates/clawd/src/schedule_service.rs` (drift test: `schedule_intent_schema_drift`) |
| `long_term_summary.schema.json` | `prompts/layers/overlays/long_term_summary_prompt.md` | `crates/clawd/src/memory/service.rs::LongTermRefreshLlmOut` + memory fact candidate parser (drift test: `long_term_summary_schema_drift`) |
| `context_compaction.schema.json` | Agent-loop-only future model-assisted context compaction output contract | `crates/clawd/src/agent_engine/context_compaction.rs::normalize_model_assisted_compaction_output` |
| `voice_mode_intent.schema.json` | `prompts/layers/overlays/voice_mode_intent_prompt.md` | `crates/claw-core/src/hard_rules/voice_mode.rs::VoiceModeIntentDecision` + `crates/telegramd/src/main.rs` (drift test: `voice_mode_intent_schema_drift`) |
| `run_cmd_suggestion.schema.json` | inline prompt `crates/clawd/src/skills/builtin.rs::build_run_cmd_nl_prompt` | `crates/clawd/src/skills/builtin.rs::RunCmdSuggestionPayload` (drift test: `run_cmd_suggestion_schema_drift`) |
| `image_reference_resolver.schema.json` | `prompts/layers/overlays/image_reference_resolver_prompt.md` | `crates/skills/image_edit/src/main.rs::parse_llm_selected_index` |
| `language_infer.schema.json` | `prompts/layers/overlays/language_infer_prompt.md` | `crates/skills/image_vision/src/main.rs::parse_language_choice_from_llm` |
| `stock_alias_choice.schema.json` | inline JSON contract in `crates/skills/stock/src/main.rs::choose_candidate_via_llm` | `crates/skills/stock/src/main.rs::parse_llm_alias_response` |
| `temporary_fix_plan.schema.json` | `prompts/layers/overlays/extension_manager_temporary_fix_system_prompt.md` | `crates/skills/extension_manager/src/main.rs::parse_temporary_fix_plan_from_text` |
| `permanent_extension_plan.schema.json` | `prompts/layers/overlays/extension_manager_permanent_extension_system_prompt.md` | `crates/skills/extension_manager/src/main.rs::parse_permanent_extension_plan_from_text` |
| `external_skill_implementation.schema.json` | `prompts/layers/overlays/extension_manager_skill_implementation_system_prompt.md` | `crates/skills/extension_manager/src/main.rs::parse_external_skill_implementation_from_text` |
| `image_vision_describe.schema.json` | `prompts/layers/overlays/image_vision_action_describe.md` | `crates/skills/image_vision/src/main.rs::parse_structured_narrative_action_output` (`describe`) |
| `image_vision_compare.schema.json` | `prompts/layers/overlays/image_vision_action_compare.md` | `crates/skills/image_vision/src/main.rs::parse_structured_narrative_action_output` (`compare`) |
| `image_vision_screenshot_summary.schema.json` | `prompts/layers/overlays/image_vision_action_screenshot_summary.md` | `crates/skills/image_vision/src/main.rs::parse_structured_narrative_action_output` (`screenshot_summary`) |

## Runtime use

The schemas are no longer documentation-only. `clawd` now uses
`prompt_utils::validate_against_schema<T>(raw, schema_id)` on the authored
hot-path JSON prompts before deserializing into Rust structs, giving more
actionable failures such as "`mode=oops_status` not in declared enum" instead
of opaque parser errors. The validator intentionally covers only the subset of
JSON Schema features used by these in-repo prompt schemas. Parser-backed paths
outside `clawd` can still consume the same schemas directly; for example
`claw_core::hard_rules::voice_mode` now validates `voice_mode_intent` JSON
against its authored schema before Telegram mode-switch routing accepts it.

## Skill-side parsers

Not every schema-backed parser lives inside `clawd`.

- `image_reference_resolver.schema.json` is consumed directly by
  `crates/skills/image_edit/src/main.rs::parse_llm_selected_index`.
- `language_infer.schema.json` is consumed directly by
  `crates/skills/image_vision/src/main.rs::parse_language_choice_from_llm`.
- `stock_alias_choice.schema.json` is consumed directly by
  `crates/skills/stock/src/main.rs::parse_llm_alias_response`.
- `temporary_fix_plan.schema.json`,
  `permanent_extension_plan.schema.json`, and
  `external_skill_implementation.schema.json` are consumed directly by
  `crates/skills/extension_manager/src/main.rs::{parse_temporary_fix_plan_from_text,parse_permanent_extension_plan_from_text,parse_external_skill_implementation_from_text}`.
- `image_vision_describe.schema.json`,
  `image_vision_compare.schema.json`, and
  `image_vision_screenshot_summary.schema.json` are consumed directly by
  `crates/skills/image_vision/src/main.rs::parse_structured_narrative_action_output`
  for the live `describe` / `compare` / `screenshot_summary` actions.

These skill-side parsers currently use small local schema-aware validators
instead of `clawd::prompt_utils`, to avoid introducing unnecessary cross-crate
runtime coupling for standalone skill binaries.

## Backlog status

As of `2026-04-20`, the **fixed-shape, real online, strict JSON-only**
prompt/parser backlog tracked by `§3.5c` is considered covered.

What remains intentionally out of that fixed-schema backlog:

- Dynamic user-schema prompts such as
  `image_vision_action_extract_with_schema.md`, whose output contract is
  supplied at runtime by the caller rather than authored as one repo-owned
  static schema file.
- Mixed dispatcher prompts such as `image_vision_prompt.md`, where only some
  action branches map to repo-owned fixed schemas and those fixed branches are
  already covered by dedicated child schemas.
- Legacy/dead overlays such as `image_vision_action_fallback.md`, which are no
  longer authoritative online JSON contracts and therefore should not be
  counted as active schema debt.

To keep this inventory honest, `crates/clawd/tests/config_templates.rs` now
contains a guard test that scans overlay prompts for strict JSON markers and
requires each match to be explicitly classified as either schema-backed or
intentionally excluded.
