<!--
Purpose: configurable rule snippets for the Intent Router (injected into `intent_router_prompt`)
Component: `clawd` (`crates/clawd/src/main.rs`) constant `INTENT_ROUTER_RULES_TEMPLATE`
Placeholders: none (the whole file is injected as rule text)
-->


Routing rules (important):
- **Skill-match guardrail:** Only choose `act` (or `chat_act`) when an existing skill clearly matches the request. Do not invent skills or force a match. If no skill clearly matches, prefer `chat` (honest limitation) or `ask_clarify` (if key intent/scope is unclear). Do not coerce the request into a superficially similar but unrelated skill.
- Use semantic intent understanding as primary signal; keyword examples are hints, not strict triggers.
- Do not rely on code-side special casing for filesystem requests. The model must route self-contained local inspection requests correctly from semantics alone.
- If the user asks to **count or inventory** the filesystem (how many files, folders, subdirectories, items, photos/images, videos, audio files, PDFs, markdown/txt/docs, or "everything here") under a directory — including phrasing like "current directory / this directory / this folder / here / pwd" — choose `act`. This is executable workspace inspection, not pure chat. Execution must follow normalizer + runtime rules: self-contained present-workspace scope should be represented as `locator_kind=current_workspace`, with no guessed `./image`/`./download`/`./photos`, and standard mappings for files vs folders vs total items vs media types.
- If the user asks to read a local file, inspect a local directory, check whether a local file exists, extract one value from a local file, compare two local files, or read local content and then summarize or explain it, choose `act` or `chat_act` by semantics even if the wording does not match any canned example exactly.
- Requests grounded in the current repo/runtime such as `read the first 30 lines of AGENTS.md`, `show the start of README.md`, `list the docs directory`, `check whether telegramd is running`, or `request http://127.0.0.1:8787/...` are executable local inspection tasks, not pure chat. Route them to `act`/`chat_act`.
- A literal filename or file-entry token in the current message is a concrete locator, not a deictic artifact type, even when the name is common or generic-looking. Requests like `show the start of README`, `read Cargo.toml`, `check AGENTS.md`, or `send LICENSE` stay executable under current-workspace resolution.
- But a deictic artifact reference such as `that README`, `that config file`, or `that log` is **not** self-contained just because the artifact type is recognizable. If recent context does not already bind exactly one concrete target of that type, choose `ask_clarify`.
- For first-turn / fresh deictic requests, do not treat historical memory aliases or stale execution traces as sufficient binding evidence by themselves. Prefer `ask_clarify` unless there is a clear immediate binding: explicit locator in current message, direct answer to the assistant's immediately previous clarification question, or exactly one high-confidence concrete target in immediate recent context.
- Old absolute paths from RECENT_EXECUTION_CONTEXT or memory are weak hints only. They must not override a self-contained current-workspace request or an explicit filename/path in the current message. If history points to an older external path but the user now asks to inspect a local repo file by name, route for current-workspace execution rather than stale-path clarification.
- This deictic safety rule must not interfere with explicit continuation/resume requests (`continue`, `keep going`, `resume from where it failed`, etc.). When continuation intent is clear, keep routing for execution instead of adding a new clarification.
- Generic baseline diagnostic requests such as "run a basic health check" are executable by the existing `health_check` skill and should route to `act`/`chat_act`, not `ask_clarify`, unless the user is explicitly asking for a narrower target that is missing.
- If execution is required to produce the answer and the same turn also asks for a conclusion, explanation, summary, comparison, grouping/categorization, or boolean judgment grounded in that execution result, choose `chat_act`, not `chat`.
- Standalone filesystem statistics requests remain `act` even if RECENT_EXECUTION_CONTEXT shows an unrelated failed file/listing command; do not downgrade to `chat` or force-resume solely because of that failure.
- If user asks to generate/create/draw an image, choose `act`.
- If user asks to edit/retouch/outpaint/restyle/add-remove elements in an image, choose `act`.
- If user asks to analyze/describe/extract/compare images or summarize screenshots, choose `act`.
- If user asks to execute shell/system commands (e.g. "run `ls -la`", "please run `uname -a`"), choose `act`.
- If user asks crypto market data (price/quote/change/candles/indicator/SMA/news/onchain/fees), choose `act`.
- If user asks crypto trading actions (trade preview / trade submit / order lookup / cancel order / holdings), choose `act`.
- For single-symbol price requests, route to `act` and prefer one direct market query flow (avoid multi-step re-query loops).
- For direct trade execution wording like "buy 1U ETH on binance", "sell 0.01 BTC on OKX", or "buy 10u BTC on binance", always choose `act` (do not route to pure chat guidance).
- For portfolio/holdings queries like "check my holdings / positions / assets", always choose `act`.
- If user asks strategy discussion only ("how should I build a strategy / why did it rise or fall / explain the concept") without direct execution intent, choose `chat`.
- If the user says "continue / keep going / resume", first inspect RECENT_EXECUTION_CONTEXT for pending action target; if a concrete tool/skill/command target exists, choose `act`.
- If RECENT_EXECUTION_CONTEXT contains schedule list/create/delete/pause/resume result and user says "delete them all / stop them all / resume them all", choose `act`.
- If user asks only to interpret/explain previous output without new action, choose `chat`.
- If the current message is itself a complete standalone executable request, do not downgrade it to `chat` just because a similar request/result appears in RECENT_EXECUTION_CONTEXT. Repeated execution requests still route to `act`/`chat_act` unless the user is explicitly asking only to discuss the previous result.
- If user asks to send/deliver a file to them (e.g. "send me the file", "send it to me", "send it as a file", "don't paste the content, just send the file"), choose `act` (or `chat_act` if they also ask for explanation). Resolve "which file" from RECENT_EXECUTION_CONTEXT when available.
- If the user already provided one concrete filename or file path, wording like "don't paste the content" strengthens the delivery intent; it does not downgrade the request into pure chat.
- Lightweight local environment identity queries such as current username, hostname, current working directory, or reading one scalar from a known local file should stay in `act`/`chat_act`, not `chat`, when one local execution step can answer them.
- If user explicitly names a file to send (e.g. "send me readme.md", "send me README.md"), still choose `act` even if no prior file-producing step exists yet; the named file itself is the target.
- Apply the named-file delivery rule to any explicit filename or file path the user provides, not only README-like examples. `Cargo.toml`, `LICENSE`, `foo/bar/report.json`, `worker.py`, and similar concrete file targets should be treated the same way.
- If a named file differs only by case from an obvious recent/current entry (e.g. `readme.md` vs `README.md`), prefer treating that as the same executable file-delivery target rather than downgrading to `ask_clarify`.
- If a user explicitly names a file to send and no case-insensitive match is found, still keep it in `act`; execution should return a direct "file not found" style result rather than routing to `ask_clarify`.
- If user asks to make some text result into a file first (e.g. "turn it into markdown and send it", "write a script file for me", "export it as txt", "make the result into a file"), choose `act` because creating and/or delivering the file is an external action, not a pure chat reply.
- If one message contains multiple explicit requests (for example: run a command + tell a joke + query holdings + fetch news), and each item is understandable on its own, choose `act` or `chat_act` for the full turn instead of asking which one to prioritize.
- **Ordinal reply (previous / two-turns-back / three-turns-back assistant reply):** When the user says previous reply / previous response / the reply before that / two replies back / three replies back, bind by **assistant turn index** first. Use __RECENT_ASSISTANT_REPLIES__ when provided (turn_id, relative_index -1/-2/-3, short_preview, has_code_block). previous reply → assistant[-1]; two replies back → assistant[-2]; three replies back → assistant[-3]. The reference target is that assistant turn only; memory/recent_related_events must not override this anchor. Choose `ask_clarify` only when there are not enough assistant turns or binding is ambiguous — do not fall back to picking from memory instead.
- **Follow-up reference and dependency install:** Resolve from recent context before choosing `ask_clarify`. For non-ordinal phrases ("the earlier text", "that code", "install the dependencies"), use RECENT_EXECUTION_CONTEXT (and normalizer output) to anchor. For dependency-install requests without package names, if dependency candidates can be inferred from recent assistant code, choose `act` (or `chat_act`); only choose `ask_clarify` when no candidate or multiple conflicting candidates. Do not route to `ask_clarify` with a generic "Which dependencies should I install?" when context can uniquely determine the target.
- If follow-up target is unclear from recent context (or ordinal reply has no matching assistant turn), choose `ask_clarify`.
- If user request contains both action and conversational request, choose `chat_act`.
- Never choose `chat_act` only because of uncertainty. Use it only when both signals are present.
- Only choose `chat` when no tool/skill/action is needed.
- If the request is likely executable but lacks one key parameter/target/scope, choose `ask_clarify` instead of `chat`.
- **act vs chat vs ask_clarify:** Use `act` only when an existing skill clearly matches and the goal is executable. Use `ask_clarify` when the request might be executable but key target/parameter/scope is unclear. Use `chat` when no skill matches, the request is outside supported capabilities, or the user needs explanation/advice rather than execution. Do not force `act` by inventing or coercing a skill.

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
- "generate a cyberpunk poster for me" -> {"mode":"act"}
- "please turn this image into watercolor style" -> {"mode":"act"}
- "analyze the differences between these two images" -> {"mode":"act"}
- "run `lsb_release -a` and tell me the result" -> {"mode":"chat_act"}
- "please run uname -a and tell me the result" -> {"mode":"chat_act"}
- "generate an image first, then tell me why you designed it that way" -> {"mode":"chat_act"}
- "please explain what this command output means" -> {"mode":"chat"}
- "what is BTCUSDT right now" -> {"mode":"act"}
- "calculate ETHUSDT SMA14" -> {"mode":"act"}
- "confirm execution: buy 0.02 ETHUSDT on binance with a limit price of 1000" -> {"mode":"act"}
- "preview only, do not execute the trade: buy 0.01 BTC" -> {"mode":"act"}
- "help me buy 10u of BTC on binance (preview first)" -> {"mode":"act"}
- "help me buy 1U ETH on binance" -> {"mode":"act"}
- "buy some BTC" -> {"mode":"ask_clarify","reason":"missing amount/risk intent","confidence":0.46}
- "help me deal with this problem" -> {"mode":"ask_clarify","reason":"action target unclear","confidence":0.33}
- "why did Bitcoin rise so much today?" -> {"mode":"chat"}
- "who are you" -> {"mode":"chat"}
- "continue" + recent#1 shows `run_cmd: echo ROUTE_MEMORY_OK` -> {"mode":"act","reason":"follow-up to recent command intent","confidence":0.82,"evidence_refs":["recent#1"]}
- "delete them all" + recent#1 shows schedule list with multiple jobs -> {"mode":"act","reason":"bulk schedule delete from recent list","confidence":0.84,"evidence_refs":["recent#1"]}
- "continue" + no resolvable recent target -> {"mode":"ask_clarify","reason":"missing action target","confidence":0.41,"evidence_refs":["recent#1"]}
- "send me the file" / "send it to me" / "send it as a file" (after a file was produced) -> {"mode":"act","reason":"deliver file to user","confidence":0.85}
- "run `ls -l`, tell a joke, check my DOGE holdings, and fetch the latest news" -> {"mode":"act","reason":"multiple explicit executable requests in one turn; should split and execute in order instead of asking priority","confidence":0.88}
- Ordinal reply: (1) A: provide RSS Python code (2) U: install the dependencies (3) A: which dependencies should I install? (4) U: "save the reply from two turns back as txt and send it to me" -> {"mode":"act","reason":"the reply from two turns back binds to assistant[-2] = step (1) RSS code; file content comes from that turn, not memory or step (3)"}

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Chinese colloquial requests such as `帮我看下`、`你瞄一眼`、`顺手查一下`、`帮我确认下` usually remain executable local inspection requests when the target is otherwise concrete.
- Style requests such as `用人话说`、`通俗点`、`给新手讲`、`别太技术` mainly constrain answer style; they do not by themselves turn an executable request into pure chat.
- Delivery wording such as `发我`、`甩给我`、`直接丢给我`、`别贴正文` strengthens delivery intent and should usually keep routing in `act` / `chat_act`.
- Strict-format wording such as `只回数字`、`只回路径`、`只给结论`、`一句话说完` constrains final output shape but should not prevent execution when execution is needed.
- Fresh deictic Chinese references such as `那个 README`、`那个配置`、`那个日志`、`它` should route to `ask_clarify` unless recent context already binds exactly one high-confidence concrete target.
- Multi-step Chinese requests joined by `再`、`然后`、`顺手`、`最后` usually still indicate one executable compound request and should not trigger a priority clarification when each subtask is already explicit.
