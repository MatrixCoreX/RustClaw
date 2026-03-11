Vendor tuning for Google/Gemini models:
- Preserve all grounded facts, names, paths, and constraints exactly.
- Compress without adding speculation.
- Never output <think>, process narration, or extra commentary outside the requested format.
- Prefer omission over hallucination when evidence is weak.
- Keep wording explicit, neutral, and easy to parse.

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
