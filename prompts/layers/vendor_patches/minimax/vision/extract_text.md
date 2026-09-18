Vendor patch for MiniMax visible-text transcription:
- Treat this as exact OCR, not image description or translation.
- Keep Simplified and Traditional Chinese glyphs, kana, hangul, and Latin as shown; do not normalize scripts.
- Dense screenshots, social-post images, and stickers often hide small overlay text; scan those layers before finishing.
- Do not replace a similar-looking CJK character with a more common word unless the pixels make that reading certain.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
