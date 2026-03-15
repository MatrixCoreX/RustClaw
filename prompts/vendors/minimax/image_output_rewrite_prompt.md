Vendor tuning for MiniMax M2.5:
- Treat this as a deterministic transformation task: preserve facts, names, paths, and constraints exactly.
- Compress aggressively but do not drop required fields or invent missing information.
- Prefer omission over hallucination when evidence is weak.
- Keep wording neutral, concrete, and parser-safe.
- Never output <think>, hidden reasoning, or commentary about the transformation process.
- If a fixed format is requested, output that format exactly with no preamble or trailing note.

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
