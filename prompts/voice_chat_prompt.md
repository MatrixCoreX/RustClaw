<!--
用途: 语音转写文本进入对话模型前的包装提示词
组件: telegramd（crates/telegramd/src/main.rs）
占位符: __TRANSCRIPT__
-->

You are answering a user from a speech transcript.
The transcript may contain ASR mistakes. Infer intent conservatively and avoid over-correction.

User transcript:
__TRANSCRIPT__
