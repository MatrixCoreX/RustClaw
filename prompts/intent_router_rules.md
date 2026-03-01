<!--
用途: Intent Router 的可配置规则片段（会注入到 intent_router_prompt）
组件: clawd（crates/clawd/src/main.rs）常量 INTENT_ROUTER_RULES_TEMPLATE
占位符: 无（作为规则文本整体注入）
-->

Routing rules (important):
- If user asks to generate/create/draw an image, choose `act`.
- If user asks to edit/retouch/outpaint/restyle/add-remove elements in an image, choose `act`.
- If user asks to analyze/describe/extract/compare images or summarize screenshots, choose `act`.
- If user asks to execute shell/system commands (e.g. "你执行 ls -la", "please run uname -a"), choose `act`.
- If the user says "continue/继续/接着做", first inspect RECENT_EXECUTION_CONTEXT for pending action target; if a concrete tool/skill/command target exists, choose `act`.
- If RECENT_EXECUTION_CONTEXT contains schedule list/create/delete/pause/resume result and user says "全部删除/全部停止/全部恢复", choose `act`.
- If user asks only to interpret/explain previous output without new action, choose `chat`.
- If follow-up target is unclear from recent context, choose `ask_clarify`.
- If user request contains both action and conversational request, choose `chat_act`.
- Never choose `chat_act` only because of uncertainty. Use it only when both signals are present.
- Only choose `chat` when no tool/skill/action is needed.

Examples:
- "帮我生成一张赛博朋克海报" -> {"mode":"act"}
- "请把这张图改成水彩风格" -> {"mode":"act"}
- "分析这两张图片差异" -> {"mode":"act"}
- "你执行 lsb_release -a 告诉我结果" -> {"mode":"chat_act"}
- "please run uname -a and tell me the result" -> {"mode":"chat_act"}
- "先生成一张图，再告诉我为什么这样设计" -> {"mode":"chat_act"}
- "请解释这段命令输出是什么意思" -> {"mode":"chat"}
- "你是谁" -> {"mode":"chat"}
- "继续" + recent#1 shows `run_cmd: echo ROUTE_MEMORY_OK` -> {"mode":"act","reason":"follow-up to recent command intent","confidence":0.82,"evidence_refs":["recent#1"]}
- "全部删除" + recent#1 shows schedule list with multiple jobs -> {"mode":"act","reason":"bulk schedule delete from recent list","confidence":0.84,"evidence_refs":["recent#1"]}
- "继续" + no resolvable recent target -> {"mode":"ask_clarify","reason":"missing action target","confidence":0.41,"evidence_refs":["recent#1"]}
