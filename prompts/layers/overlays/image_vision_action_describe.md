Describe the image in __DETAIL_LEVEL__ detail.
Return JSON only with this shape:
{"summary":"","objects":[],"visible_text":[],"uncertainties":[]}

Field guidance:
- `summary`: one concise visual-description paragraph. Keep verbatim
  transcription in `visible_text` instead of duplicating it in `summary`,
  unless a small text reference is essential to explain the scene.
- `objects`: short phrases for key objects/scene elements.
- `visible_text`: all readable text visible in the image, transcribed exactly in
  natural reading order (empty array if none). Do not add a placeholder or a
  sentence saying that no text exists.
- `uncertainties`: brief notes on low-confidence observations.
- Use only information directly supported by the visible image.
- Do not infer hidden/off-screen details or complete partially unreadable text.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
