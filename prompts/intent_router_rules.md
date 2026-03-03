<!--
用途: Intent Router 的可配置规则片段（会注入到 intent_router_prompt）
组件: clawd（crates/clawd/src/main.rs）常量 INTENT_ROUTER_RULES_TEMPLATE
占位符: 无（作为规则文本整体注入）
-->

Routing rules (important):
- Use semantic intent understanding as primary signal; keyword examples are hints, not strict triggers.
- If user asks to generate/create/draw an image, choose `act`.
- If user asks to edit/retouch/outpaint/restyle/add-remove elements in an image, choose `act`.
- If user asks to analyze/describe/extract/compare images or summarize screenshots, choose `act`.
- If user asks to execute shell/system commands (e.g. "你执行 ls -la", "please run uname -a"), choose `act`.
- If user asks crypto market data (price/quote/涨跌/K线/指标/SMA/news/onchain/手续费), choose `act`.
- If user asks crypto trading actions (预览下单/确认下单/查订单/撤单/持仓), choose `act`.
- If user asks strategy discussion only ("怎么做策略/为什么涨跌/解释概念") without direct execution intent, choose `chat`.
- If the user says "continue/继续/接着做", first inspect RECENT_EXECUTION_CONTEXT for pending action target; if a concrete tool/skill/command target exists, choose `act`.
- If RECENT_EXECUTION_CONTEXT contains schedule list/create/delete/pause/resume result and user says "全部删除/全部停止/全部恢复", choose `act`.
- If user asks only to interpret/explain previous output without new action, choose `chat`.
- If follow-up target is unclear from recent context, choose `ask_clarify`.
- If user request contains both action and conversational request, choose `chat_act`.
- Never choose `chat_act` only because of uncertainty. Use it only when both signals are present.
- Only choose `chat` when no tool/skill/action is needed.

Confidence and safety policy:
- High confidence and clear executable intent -> prefer `act`.
- Mixed intent with both execution and explanation/result request -> `chat_act`.
- If follow-up target, parameters, or execution scope is ambiguous -> `ask_clarify` first.
- For potentially irreversible actions, when intent is not explicit enough, route to `ask_clarify` rather than guessing.
- When uncertain between `chat` and `act`, prefer:
  - `chat` for pure explanation/discussion intent,
  - `ask_clarify` for potentially actionable but unclear intent.

Examples:
- "帮我生成一张赛博朋克海报" -> {"mode":"act"}
- "请把这张图改成水彩风格" -> {"mode":"act"}
- "分析这两张图片差异" -> {"mode":"act"}
- "你执行 lsb_release -a 告诉我结果" -> {"mode":"chat_act"}
- "please run uname -a and tell me the result" -> {"mode":"chat_act"}
- "先生成一张图，再告诉我为什么这样设计" -> {"mode":"chat_act"}
- "请解释这段命令输出是什么意思" -> {"mode":"chat"}
- "现在 BTCUSDT 多少钱" -> {"mode":"act"}
- "算下 ETHUSDT 的 SMA14" -> {"mode":"act"}
- "确认执行：paper 模式 ETHUSDT 限价买 0.02，价格 1000" -> {"mode":"act"}
- "只做预览，不要执行交易，BTC 买 0.01" -> {"mode":"act"}
- "帮我 paper 买 10u BTC（先预览）" -> {"mode":"act"}
- "买点 BTC 吧" -> {"mode":"ask_clarify","reason":"missing amount/risk intent","confidence":0.46}
- "帮我处理一下这个问题" -> {"mode":"ask_clarify","reason":"action target unclear","confidence":0.33}
- "为什么比特币今天涨这么多？" -> {"mode":"chat"}
- "你是谁" -> {"mode":"chat"}
- "继续" + recent#1 shows `run_cmd: echo ROUTE_MEMORY_OK` -> {"mode":"act","reason":"follow-up to recent command intent","confidence":0.82,"evidence_refs":["recent#1"]}
- "全部删除" + recent#1 shows schedule list with multiple jobs -> {"mode":"act","reason":"bulk schedule delete from recent list","confidence":0.84,"evidence_refs":["recent#1"]}
- "继续" + no resolvable recent target -> {"mode":"ask_clarify","reason":"missing action target","confidence":0.41,"evidence_refs":["recent#1"]}
