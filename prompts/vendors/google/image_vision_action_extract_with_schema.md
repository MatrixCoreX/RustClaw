Vendor tuning for Google/Gemini models:
- Ground every statement in visible evidence from the image or screenshot.
- Distinguish observation from inference; if uncertain, leave the field empty/null per schema or mark uncertainty briefly.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Keep output dense, concrete, and schema-faithful.
- Do not add commentary beyond the requested fields.

Extract structured data from image(s) and return valid JSON matching this schema: __SCHEMA__
