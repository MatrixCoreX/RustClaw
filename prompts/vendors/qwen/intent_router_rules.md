<!--
用途: Intent Router 的可配置规则片段（会注入到 intent_router_prompt）
组件: clawd（crates/clawd/src/main.rs）常量 INTENT_ROUTER_RULES_TEMPLATE
占位符: 无（作为规则文本整体注入）
-->


Vendor tuning for OpenAI-compatible models:
- Make one decisive classification and commit to it.
- Output exactly the required JSON or label and nothing else.
- Never output <think>, explanations, markdown fences, or prose before/after the required JSON or label.
- Prefer ask_clarify when a missing key field would make execution unsafe or materially incomplete.
- Resolve follow-up intent from recent execution context first, then memory; keep memory non-authoritative.
- Keep reasons compact, explicit, and tightly grounded in observable evidence.

Routing rules (important):
- Use semantic intent understanding as primary signal; keyword examples are hints, not strict triggers.
- If the user asks to **count or inventory** the filesystem (how many files, folders, subdirectories, items, photos/images, videos, audio files, PDFs, markdown/txt/docs, or "everything here") under a directory — including "current directory / this directory / this folder / here / pwd / 当前目录" phrasing — choose `act`. This is executable workspace inspection, not pure chat. Execution must follow normalizer + runtime rules: **current-directory phrases → `.`**, no guessed `./image`/`./download`/`./photos`, and standard mappings for 文件 vs 文件夹 vs 东西 vs media types.
- Standalone filesystem statistics requests remain `act` even if RECENT_EXECUTION_CONTEXT shows an unrelated failed file/listing command; do not downgrade to `chat` or force-resume solely because of that failure.
- If user asks to generate/create/draw an image, choose `act`.
- If user asks to edit/retouch/outpaint/restyle/add-remove elements in an image, choose `act`.
- If user asks to analyze/describe/extract/compare images or summarize screenshots, choose `act`.
- If user asks to execute shell/system commands (e.g. "你执行 ls -la", "please run uname -a"), choose `act`.
- If user asks crypto market data (price/quote/涨跌/K线/指标/SMA/news/onchain/手续费), choose `act`.
- If user asks crypto trading actions (预览下单/确认下单/查订单/撤单/持仓), choose `act`.
- For single-symbol price requests, route to `act` and prefer one direct market query flow (avoid multi-step re-query loops).
- For direct trade execution wording like "帮我在币安买 1U ETH", "在 OKX 卖出 0.01 BTC", "buy 10u BTC on binance", always choose `act` (do not route to pure chat guidance).
- For portfolio/holdings queries like "查持仓/看仓位/资产情况", always choose `act`.
- If user asks strategy discussion only ("怎么做策略/为什么涨跌/解释概念") without direct execution intent, choose `chat`.
- If the user says "continue/继续/接着做", first inspect RECENT_EXECUTION_CONTEXT for pending action target; if a concrete tool/skill/command target exists, choose `act`.
- If RECENT_EXECUTION_CONTEXT contains schedule list/create/delete/pause/resume result and user says "全部删除/全部停止/全部恢复", choose `act`.
- If user asks only to interpret/explain previous output without new action, choose `chat`.
- If the current message is itself a complete standalone executable request, do not downgrade it to `chat` just because a similar request/result appears in RECENT_EXECUTION_CONTEXT. Repeated execution requests still route to `act`/`chat_act` unless the user is explicitly asking only to discuss the previous result.
- If user asks to send/deliver a file to them (e.g. "把文件发给我", "发给我", "发一下文件", "send me the file", "发过来", "以文件形式发给我", "不要贴内容直接发文件", "send it as a file"), choose `act` (or `chat_act` if they also ask for explanation). Resolve "which file" from RECENT_EXECUTION_CONTEXT when available.
- If user explicitly names a file to send (e.g. "把 readme.md 发给我", "send me README.md"), still choose `act` even if no prior file-producing step exists yet; the named file itself is the target.
- Apply the named-file delivery rule to any explicit filename or file path the user provides, not only README-like examples. `Cargo.toml`, `LICENSE`, `foo/bar/report.json`, `worker.py`, and similar concrete file targets should be treated the same way.
- If a named file differs only by case from an obvious recent/current entry (e.g. `readme.md` vs `README.md`), prefer treating that as the same executable file-delivery target rather than downgrading to `ask_clarify`.
- If a user explicitly names a file to send and no case-insensitive match is found, still keep it in `act`; execution should return a direct "file not found" style result rather than routing to `ask_clarify`.
- If user asks to make some text result into a file first (e.g. "整理成 md 发我", "写个脚本文件给我", "导出成 txt 给我", "把结果做成文件"), choose `act` because creating and/or delivering the file is an external action, not a pure chat reply.
- If one message contains multiple explicit requests (for example: run a command + tell a joke + query holdings + fetch news), and each item is understandable on its own, choose `act` or `chat_act` for the full turn instead of asking which one to prioritize.
- If follow-up target is unclear from recent context, choose `ask_clarify`.
- If user request contains both action and conversational request, choose `chat_act`.
- Never choose `chat_act` only because of uncertainty. Use it only when both signals are present.
- Only choose `chat` when no tool/skill/action is needed.
- If the request is likely executable but lacks one key parameter/target/scope, choose `ask_clarify` instead of `chat`.

Confidence and safety policy:
- High confidence and clear executable intent -> prefer `act`.
- Mixed intent with both execution and explanation/result request -> `chat_act`.
- If follow-up target, parameters, or execution scope is ambiguous -> `ask_clarify` first.
- Do not use `ask_clarify` only because there are multiple clear tasks in the same user turn.
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
- "确认执行：binance 模式 ETHUSDT 限价买 0.02，价格 1000" -> {"mode":"act"}
- "只做预览，不要执行交易，BTC 买 0.01" -> {"mode":"act"}
- "帮我 binance 买 10u BTC（先预览）" -> {"mode":"act"}
- "帮我在币安买 1U ETH" -> {"mode":"act"}
- "买点 BTC 吧" -> {"mode":"ask_clarify","reason":"missing amount/risk intent","confidence":0.46}
- "帮我处理一下这个问题" -> {"mode":"ask_clarify","reason":"action target unclear","confidence":0.33}
- "为什么比特币今天涨这么多？" -> {"mode":"chat"}
- "你是谁" -> {"mode":"chat"}
- "继续" + recent#1 shows `run_cmd: echo ROUTE_MEMORY_OK` -> {"mode":"act","reason":"follow-up to recent command intent","confidence":0.82,"evidence_refs":["recent#1"]}
- "全部删除" + recent#1 shows schedule list with multiple jobs -> {"mode":"act","reason":"bulk schedule delete from recent list","confidence":0.84,"evidence_refs":["recent#1"]}
- "继续" + no resolvable recent target -> {"mode":"ask_clarify","reason":"missing action target","confidence":0.41,"evidence_refs":["recent#1"]}
- "把文件发给我" / "发给我" / "send me the file" / "以文件形式发给我" (after a file was produced) -> {"mode":"act","reason":"deliver file to user","confidence":0.85}
- "执行 ls -l，讲个笑话，查询我 doge 持仓，查询最新新闻" -> {"mode":"act","reason":"multiple explicit executable requests in one turn; should split and execute in order instead of asking priority","confidence":0.88}
