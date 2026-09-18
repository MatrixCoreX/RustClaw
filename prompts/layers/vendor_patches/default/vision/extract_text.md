Vendor patch for visible-text transcription:
- Prefer complete glyph-level transcription over a readable paraphrase.
- Reinspect corners, overlays, and stacked captions before omitting a line.
- If a character is only partly visible, keep the readable fragment and record the gap in `uncertainties`.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
