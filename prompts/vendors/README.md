# Vendor prompt overrides

- **Default skill prompts** live under `default/skills/`. See `default/skills/README.md`.
- **Runtime**: skill prompts are loaded only from vendor layers — `prompts/vendors/<active>/skills/<name>.md`, then `prompts/vendors/default/skills/<name>.md`. No fallback to `prompts/skills/`.
- **Naming**: same as registry — one file per skill name. `scripts/sync_skill_docs.py` updates `default/skills/*.md` from crates' `INTERFACE.md`.
