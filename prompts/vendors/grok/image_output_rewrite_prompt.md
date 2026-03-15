Vendor tuning for Grok models:
- Preserve all grounded facts, names, paths, and constraints exactly.
- Compress without inventing missing information.
- Never output <think>, process narration, or extra commentary outside the requested format.
- Prefer omission over speculation when evidence is weak.
- Keep wording sharp, concrete, and parser-safe.

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
