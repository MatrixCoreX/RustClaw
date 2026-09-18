Vendor patch for MiniMax image-text revision:
- Do not convert Traditional/Simplified Chinese or fullwidth/halfwidth digits while restoring punctuation.
- Keep hashtags, @mentions, watermark slogans, and overlay captions from the recognized text.
- If two CJK glyphs look similar, keep the recognized fragment unless the correction is unambiguous.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
