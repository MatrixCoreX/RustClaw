# Prompt Layers

This directory defines RustClaw's layered prompt sources.

The shared runtime prompt-layer parser/helper lives in `crates/claw-core/src/prompt_layers.rs`. `clawd`, `telegramd`, and any skill process that adopts this helper will render prompts according to the layering rules defined here.

## Structure

- `base/`
  - Stores system truth and cross-model shared rules
- `overlays/`
  - Stores the main body for task-type or concrete prompts
- `vendor_patches/<vendor>/`
  - Stores only thin vendor/model-specific adaptations
- `manifest.toml`
  - Declares which logical prompts use layered rendering, and how each is composed from base / overlay / vendor_patch

## Overlay Sources

The first migration batch has already moved the overlay bodies declared in the manifest into `prompts/layers/overlays/*.md`:

- `overlay` entries in the manifest now point primarily to `prompts/layers/overlays/*.md`
- These overlay files came from a controlled migration of the former default-vendor body copies, with the old `Vendor tuning ...` sections removed during migration
- As a result, the old full non-skill vendor copies are no longer kept; for these prompts, the only main prompt body chain is now under `layers/`
- The old `prompts/vendors/` tree has been removed and no longer participates in runtime prompt resolution for migrated prompts

## Skill Prompt

- The main source is `prompts/layers/generated/skills/<name>.md`
- This file is generated or updated from `INTERFACE.md` by `scripts/sync_skill_docs.py`
- If a model truly needs skill-level specialization, only `vendor_patches/<vendor>/skills/<name>.md` may be added
- `prompts/skills/<name>.md` is only the logical path used in registry; it does not map to a real prompt-body directory under `prompts/`

## Persona Prompt

- Persona prompts still participate in runtime resolution through the logical path `prompts/personas/<profile>.md`
- The actual prompt body lives at `prompts/layers/overlays/personas/<profile>.md`
- The physical `prompts/personas/` directory has been removed and no longer stores real files

## Debug / Preview

Use the following scripts to inspect the final prompt a given vendor receives:

```bash
python3 scripts/render_prompt_layers.py --list
python3 scripts/render_prompt_layers.py --vendor openai --prompt prompts/agent_tool_spec.md
python3 scripts/render_prompt_layers.py --vendor claude --prompt prompts/clarify_question_prompt.md
```

## Maintenance Rules

- For new rules, first decide whether they should be encoded in code; if they can be encoded in code, prefer code
- Shared behavior belongs in `base`
- Task-specific prompt body belongs in `overlay`
- Only model-specific adaptation belongs in `vendor_patch`
- Avoid maintaining new full vendor prompt copies unless absolutely necessary

## EOF multilingual reinforcement block

- **Scope:** Every real prompt markdown file under `prompts/` (layers, overlays, vendor patches, generated skills, repo-root prompt stubs such as `skill_authoring_strict.md`) must end with the same fixed section: an H2 titled “Multilingual Reinforcement” followed by the HTML comment template used everywhere in this repo.
- **Not for README docs:** Do **not** append this block to documentation-only files such as `prompts/layers/README.md` or `prompts/layers/generated/skills/README.md`; those files only document the convention.
- **Purpose:** Reserve the EOF area for language-specific nuance (e.g. `### zh-CN`, `### en`). Keep universal rules in the main body above; do not duplicate general policy in this block.
- **Edits:** If the section already exists but differs, normalize it to the canonical text (single block at EOF, no duplicates). New or regenerated skill prompts from `scripts/sync_skill_docs.py` include this block automatically.
