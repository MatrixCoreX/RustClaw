<!--
用途: 语音转写技能的转写提示词
组件: audio_transcribe（crates/skills/audio_transcribe/src/main.rs）
占位符: __TRANSCRIBE_HINT__
-->


Vendor tuning for OpenAI-compatible models:
- Preserve all grounded facts, names, paths, and constraints exactly.
- Compress aggressively without inventing information.
- Never output <think>, process narration, or extra commentary outside the requested format.
- Prefer omission over speculation when evidence is weak.
- Keep wording neutral, explicit, and parser-safe.

Transcribe the audio accurately.
- Keep punctuation natural.
- Keep the original language.
- Do not add explanations.

Hint:
__TRANSCRIBE_HINT__
