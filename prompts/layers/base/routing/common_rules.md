Shared routing contract:
- Route by semantics and task shape, not brittle keyword matching.
- Keep memory and historical traces as supporting evidence only. Prefer the freshest current-turn and immediate recent-turn evidence.
- Self-contained local inspection requests are executable: reading files, listing directories, counting items, extracting one value, checking status, comparing local content, and read-then-summarize flows should route to execution paths.
- If execution is required and the same turn also asks for explanation, summary, comparison, or conclusion, prefer `chat_act` instead of `chat`.
- Delivery requests (`send it to me`, `send me the file`, `don't paste the content`) are executable file-delivery intents, not pure chat.
- Fresh deictic references to files, directories, logs, configs, or similar artifacts need a unique concrete binding; otherwise prefer `ask_clarify`.
- A self-contained local inspection request whose scope semantically refers to the present working directory / current workspace context should remain executable. Do not turn it into a directory-choice clarification merely because recent context mentions other directories.
- An explicitly written filename/path token in the current message counts as concrete locator input even when the name is common or generic-looking. Do not demote literal file-entry names such as `README`, `README.md`, `LICENSE`, `Cargo.toml`, `AGENTS.md`, `Makefile`, or similar current-turn basenames into deictic references merely because history also mentions same-type artifacts. These names are examples, not a closed list.
- For filename-only local file requests, treat the filename as sufficient concrete locator input for execution routing. Do not route to clarification just because the directory is omitted; the execution side should first attempt bounded resolution under `default_locator_search_dir`.
- Historical absolute paths, old workspace roots, and stale execution traces are weak hints only. They may inform clarification, but they must not override an explicit current-turn locator or a self-contained current-workspace request.
- Standalone filesystem counting or inventory requests remain new executable tasks even if an older failed task also involved files or paths.
- Ordinal reply references (previous reply / two-turns-back reply) must bind by assistant-turn index first.
- For dependency-install follow-ups without package names, infer candidates from immediate recent assistant code before asking a generic clarification.
- Use `ask_clarify` only when the request is otherwise executable but one key target, scope, or parameter is still missing.

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
- Chinese colloquial action phrases such as `看一眼`、`瞄一眼`、`顺手看看`、`帮我确认一下`、`帮我过一遍` usually still mean normal inspection/execution intent rather than pure chat.
- Scope phrases such as `这里`、`当前目录`、`这个目录`、`这个仓库里`、`手头这个工作区` usually refer to the present workspace scope unless the current message explicitly names another path.
- Deictic Chinese references such as `那个`、`它`、`上面那个`、`刚才那个` only count as executable target binding when immediate context already binds exactly one concrete target of the right type.
- Delivery-style wording such as `发我`、`甩给我`、`直接给我`、`别贴正文` should be interpreted as file/content delivery intent, not as a request to paste content inline.
- Output-style wording such as `只回数字`、`只给结果`、`一句话说完`、`不用展开` constrains the final answer format; it does not by itself change routing away from execution.
