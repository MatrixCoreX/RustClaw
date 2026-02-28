<!--
用途: Intent Router 的可配置规则片段（会注入到 intent_router_prompt）
组件: clawd（crates/clawd/src/main.rs）常量 INTENT_ROUTER_RULES_TEMPLATE
占位符: 无（作为规则文本整体注入）
-->

Routing rules (important):
- If user asks to generate/create/draw an image, choose `act`.
- If user asks to edit/retouch/outpaint/restyle/add-remove elements in an image, choose `act`.
- If user asks to analyze/describe/extract/compare images or summarize screenshots, choose `act`.
- If user request contains both action and conversational request, choose `chat_act`.
- Only choose `chat` when no tool/skill/action is needed.

Examples:
- "帮我生成一张赛博朋克海报" -> {"mode":"act"}
- "请把这张图改成水彩风格" -> {"mode":"act"}
- "分析这两张图片差异" -> {"mode":"act"}
- "先生成一张图，再告诉我为什么这样设计" -> {"mode":"chat_act"}
- "你是谁" -> {"mode":"chat"}
