<!--
用途: 语音转写技能的转写提示词
组件: audio_transcribe（crates/skills/audio_transcribe/src/main.rs）
占位符: __TRANSCRIBE_HINT__
-->


Vendor tuning for Qwen models:
- Treat this as a deterministic transformation task: preserve facts, names, paths, and constraints exactly.
- Compress strongly but do not invent missing facts.
- Prefer omission over hallucination when evidence is weak.
- Never output <think>, process narration, or extra commentary outside the requested format.
- Keep wording concrete, compact, and parser-safe.

Transcribe the audio accurately.
- Keep punctuation natural.
- Keep the original language.
- Do not add explanations.

Hint:
__TRANSCRIBE_HINT__
