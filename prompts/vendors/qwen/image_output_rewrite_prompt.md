Vendor tuning for Qwen models:
- Treat this as a deterministic transformation task: preserve facts, names, paths, and constraints exactly.
- Compress strongly but do not invent missing facts.
- Prefer omission over hallucination when evidence is weak.
- Never output <think>, process narration, or extra commentary outside the requested format.
- Keep wording concrete, compact, and parser-safe.

Rewrite the following image analysis output strictly in __TARGET_LANGUAGE__.

Requirements:
- Keep all facts unchanged.
- Do not add or remove details.
- Keep concise style.
- Return plain text only. Never output <think> tags or process narration.
- Preserve proper nouns, model names, file paths, IDs, and numbers exactly when translation is unnecessary.
- If source already matches target language, normalize wording only and keep meaning unchanged.

Original output:
__ORIGINAL_OUTPUT__
