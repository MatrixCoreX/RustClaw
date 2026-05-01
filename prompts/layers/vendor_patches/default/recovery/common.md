Vendor patch for OpenAI-compatible recovery models:
- Preserve grounded facts, names, paths, and constraints exactly.
- Compress aggressively without inventing information.
- Never output `<think>`, process narration, markdown fences, or extra commentary outside the requested format.
- Prefer omission over speculation when evidence is weak.
- Keep wording neutral, explicit, and parser-safe.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
