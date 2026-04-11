<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for MiniMax M2.5:
- Treat each skill description as an operational contract, not loose inspiration.
- Use only the capabilities explicitly described by the skill and keep arguments minimal and standalone.
- Avoid stuffing unrelated prior outputs into skill arguments unless the user explicitly asks for grounding in those outputs.
- Prefer the narrowest skill/tool that can finish the subtask correctly.
- Keep downstream outputs compatible with the existing planner and parser expectations.
- Avoid meta discussion; optimize for clean planner consumption rather than human-facing flourish.
- For filename-only local file requests, prefer one bounded `find_name` resolution under the default workspace root before producing a clarification about full path. Filenames such as `Cargo.toml`, `README.md`, and `AGENTS.md` are examples, not a closed list.
- Do not skip directly to "please provide the full path" when the current request already names a concrete filename and bounded default-root resolution has not been attempted yet.
- For path-scoped lookup requests such as `在 <dir> 找 <token>` where `<token>` is being used like a file or directory name, prefer `find_name` instead of `grep_text`.
- Use `grep_text` only when the user is clearly asking to search inside file contents (for example text/content/包含某行/grep semantics), not when they are trying to locate entries by name.

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
- 中文路径作用域请求里，像 `去 logs 找 act_plan`、`在 document 里找 README`、`去某个目录找 abcd` 这类默认优先理解为按名称定位条目，应使用 `find_name`，不是 `grep_text`。
- 只有当用户明确是在找文件内容中的文本，例如 `搜索包含 xxx 的行`、`grep 一下内容`、`看看哪个文件里出现了 xxx`，才应改用 `grep_text`。
