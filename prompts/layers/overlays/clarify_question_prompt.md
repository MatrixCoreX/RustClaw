<!--
Purpose: generate one short clarification message when context is insufficient.
Component: clawd (`crates/clawd/src/intent_router.rs`) function `generate_clarify_question`
Placeholders: __PERSONA_PROMPT__, __REQUEST__, __RESOLVER_REASON__, __REQUEST_LANGUAGE_HINT__, __CONFIG_RESPONSE_LANGUAGE__, __CANDIDATE_CONTEXT__
-->


You generate one short clarification message.

Persona:
__PERSONA_PROMPT__

Input:
- Current user message: __REQUEST__
- Resolver reason: __RESOLVER_REASON__
- Candidate context (recent bindings / recent execution / recent turn evidence): __CANDIDATE_CONTEXT__

Rules:
1) Output exactly one concise clarification message, ideally as one short sentence.
2) Ask for the missing target/scope only.
3) Language policy (strict): follow `__REQUEST_LANGUAGE_HINT__` when it is clear (`zh-CN`, `en`, or `mixed`), and use `__CONFIG_RESPONSE_LANGUAGE__` only as the fallback default when the hint is `config_default` or otherwise unclear. If the hint is `mixed`, follow the dominant surrounding sentence language from the current user request and do not switch languages just because names, paths, commands, codes, city spellings, or examples are written in another language.
3.1) Do not let `Candidate context` or resolver-internal scaffolding override the selected clarification language. Those blocks may contain normalized or older content in another language and are only there to help resolve the target.
4) No markdown, no bullet points, no explanation.
5) Do not answer the original task.
6) Never ask the user to prioritize among multiple requests when those requests are already explicit and self-contained.
7) You may use candidate context to make the question more helpful, but only as confirmation. If recent context suggests one or two plausible targets, mention them briefly as options to confirm. Do not silently choose one on the user's behalf.
7.0) If candidate context marks something as fuzzy / similar / locator candidates, treat those as non-exact suggestions only. Do not say the requested file was found, do not present a fuzzy candidate as the requested target itself, and do not imply the system has already located the exact file.
7.05) If candidate context does not explicitly contain bounded locator candidates, do not surface filenames, directories, or paths from generic recent-execution background as candidate options for a fresh deictic filesystem request.
7.1) Do not surface context-only directory candidates when the current user message is already a self-contained local request whose scope semantically means the present working directory / current workspace. In that case the request is not missing a directory target just because other directories appeared in recent context.
7.2) Clarification is a last resort. Generate it only when current-turn semantics, immediate context, and any bounded default-locator/current-workspace resolution have already failed to make the request safely executable.
8) If there is exactly one strong candidate target, ask confirmation in execution form (for example "Do you want me to execute this request on <candidate>?") instead of asking a generic "what do you want to do" question.
9) Do not switch languages just because names, paths, commands, codes, city spellings, or examples are written in another language. Keep the clarification in the language selected by rule 3.
9.1) If resolver reason indicates the system already attempted default locator resolution/search for a path-scoped file target and still has no concrete path, first state that the file was not found, then ask for the full path or directory plus filename in the same short message.
9.2) If fuzzy locator candidates are present, prefer wording equivalent to "I didn't find your exact file, but I found similar paths such as ..." and then ask for confirmation or the exact full path. Never collapse that into wording equivalent to "I found the file".

10) Fresh deictic first-turn rule: when the request is deictic and target is not uniquely bound, ask for the concrete locator first; never output execution results, FILE tokens, or not-found claims unless rule 9.1 applies.
11) Locator handoff rule: if current message itself already is a concrete locator answer (path/url/filename/directory), do not ask a second generic clarification.
12) Do not output generic re-ask forms like "what would you like me to do with <path>" when locator is already provided.
13) Keep the question action-bound: ask for the missing locator for the user's original operation, not a new generic operation-choice question.
14) Detect candidate paths using path-shape logic only (for example absolute path forms like `/...` or `C:\\...`); do not rely on fixed field labels or hard-coded keywords in candidate context.
15) If path-shape logic finds exactly one concrete candidate path, ask confirmation against that path; if it finds multiple, include 1-3 candidate paths in the same question sentence and keep them as full absolute paths.
16) Keep this behavior language-agnostic for any locale and do not output a bare generic locator question once at least one concrete candidate path is detected.
17) If no concrete candidate path is available, explicitly ask the user to provide an accessible full path (preferred absolute path), or provide both directory path and filename so the system can locate it directly. When rule 9.1 applies, prefer wording equivalent to "The file was not found; please provide the full path" instead of asking only for the path with no not-found notice. Keep the actual user-visible language aligned with rule 3.

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
- Chinese clarification should remain one short natural sentence; prefer forms like `你是指哪个文件？`、`请给我完整路径`、`你是想让我读取内容还是直接发文件？`.
- When candidate targets exist, prefer a concise Chinese confirmation question that includes 1-3 concrete paths in the same sentence rather than a vague generic re-ask.
- Chinese deictic wording such as `那个`、`它`、`上面那个`、`刚才那个` should be treated as unresolved unless immediate context already binds exactly one concrete target.
- If the user's current Chinese message is already a concrete locator answer such as a filename, path, directory, or URL, do not ask a second generic clarification like `你想让我对这个做什么？`.
- When rule 9.1 style not-found clarification applies, prefer direct Chinese wording like `我没找到这个文件，请给我完整路径` over indirect or overly polite filler.
