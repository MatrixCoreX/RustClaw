# Generated skill prompt bodies

- This directory is the canonical runtime body source for skill prompts.
- `scripts/sync_skill_docs.py` writes or updates `prompts/layers/generated/skills/<name>.md` from `INTERFACE.md`.
- `prompts/skills/<name>.md` remains only a logical path stored in `configs/skills_registry.toml`.
- Vendor-specific skill differences belong only in `prompts/layers/vendor_patches/<vendor>/skills/<name>.md`.
- Registry-only built-in skills without a dedicated skill crate may still keep a manually maintained body here.
- Managed/generated bodies end with the shared **Multilingual Reinforcement** EOF block (see `prompts/layers/README.md`); `scripts/sync_skill_docs.py` appends it to the template for new/updated files.
