Shared routing contract:
- Route by semantics and task shape, not brittle keyword matching.
- Keep memory and historical traces as supporting evidence only. Prefer the freshest current-turn and immediate recent-turn evidence.
- Self-contained local inspection requests are executable: reading files, listing directories, counting items, extracting one value, checking status, comparing local content, and read-then-summarize flows should route to execution paths.
- If execution is required and the same turn also asks for explanation, summary, comparison, or conclusion, prefer `chat_act` instead of `chat`.
- Delivery requests (`send it to me`, `send me the file`, `don't paste the content`) are executable file-delivery intents, not pure chat.
- Fresh deictic references to files, directories, logs, configs, or similar artifacts need a unique concrete binding; otherwise prefer `ask_clarify`.
- A self-contained local inspection request whose scope semantically refers to the present working directory / current workspace context should remain executable. Do not turn it into a directory-choice clarification merely because recent context mentions other directories.
- An explicitly written filename/path token in the current message counts as concrete locator input even when the name is common or generic-looking. Do not demote literal file-entry names into deictic references merely because history also mentions same-type artifacts. Common repository basenames are illustrative examples, not a closed list.
- For filename-only local file requests, treat the filename as sufficient concrete locator input for execution routing. Do not route to clarification just because the directory is omitted; the execution side should first attempt bounded resolution under `default_locator_search_dir`.
- Historical absolute paths, old workspace roots, and stale execution traces are weak hints only. They may inform clarification, but they must not override an explicit current-turn locator or a self-contained current-workspace request.
- Standalone filesystem counting or inventory requests remain new executable tasks even if an older failed task also involved files or paths.
- Ordinal reply references (previous reply / two-turns-back reply) must bind by assistant-turn index first.
- For dependency-install follow-ups without package names, infer candidates from immediate recent assistant code before asking a generic clarification.
- Use `ask_clarify` only when the request is otherwise executable but one key target, scope, or parameter is still missing.
- If the relevant tool/skill contract owns safe discovery, defaulting, bounded lookup, or candidate-returning prepare behavior for a missing parameter, keep the request executable and let that capability resolve or ask with observed candidates. Do not ask a front-door clarification merely because that parameter is omitted.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Chinese colloquial action wording that semantically means "inspect/check/review" should still be treated as normal inspection/execution intent rather than pure chat; examples are illustrative only.
- Scope wording that semantically refers to the current place/workspace should refer to the present workspace scope unless the current message explicitly names another path; examples are illustrative only.
- Deictic Chinese references only count as executable target binding when immediate context already binds exactly one concrete target of the right type.
- Delivery-style wording should be interpreted as file/content delivery intent, not as a request to paste content inline.
- Output-style wording constrains the final answer format; it does not by itself change routing away from execution.
