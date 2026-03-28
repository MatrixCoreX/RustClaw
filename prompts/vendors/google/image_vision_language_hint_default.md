Vendor tuning for Google/Gemini models:
- Ground every statement in visible evidence from the image or screenshot.
- Distinguish observation from inference; if uncertain, leave the field empty/null per schema or mark uncertainty briefly.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Keep output dense, concrete, and schema-faithful.
- Do not add commentary beyond the requested fields.

Use remembered response language from context preferences first (response_language or language).
If preference is unknown, use config.toml default language. Do not infer language from task instruction/user request text.
