<!--
用途: 长期记忆摘要生成提示词（把近期对话压缩为可持久化摘要）
组件: clawd（crates/clawd/src/main.rs）常量 LONG_TERM_SUMMARY_PROMPT_TEMPLATE
占位符: __PREVIOUS_SUMMARY__, __NEW_CONVERSATION_CHUNK__
-->

Summarize the conversation into durable memory for future replies.
Keep it factual, concise, and action-oriented. Include user preferences, constraints, ongoing tasks, and decisions.
Do not include raw small talk unless it changes context.
Return plain text only.

Previous long-term summary:
__PREVIOUS_SUMMARY__

New conversation chunk:
__NEW_CONVERSATION_CHUNK__

