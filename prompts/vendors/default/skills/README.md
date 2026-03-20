# Default skill prompts (vendor fallback baseline)

- Runtime loads skill prompts only from vendor layers: first `prompts/vendors/<active>/skills/<name>.md`, then this directory `prompts/vendors/default/skills/<name>.md`.
- The main directory `prompts/skills/` is no longer used at runtime.
- This directory is the fallback baseline; keep it in sync with the skill set in `configs/skills_registry.toml`. Use `python3 scripts/sync_skill_docs.py` to regenerate from crates' `INTERFACE.md`.
