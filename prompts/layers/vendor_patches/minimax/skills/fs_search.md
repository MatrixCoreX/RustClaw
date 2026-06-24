<!-- AUTO-GENERATED: sync_skill_docs.py -->

MiniMax-specific `fs_search` tuning:
- For filename-only local file requests, prefer one bounded `find_name` resolution under the default workspace root before producing a clarification about full path. Common repository filenames are illustrative examples, not a closed list.
- Do not skip directly to "please provide the full path" when the current request already names a concrete filename and bounded default-root resolution has not been attempted yet.
- For path-scoped lookup requests where the searched token is being used like a file or directory name, prefer `find_name` instead of `grep_text`.
- Use `grep_text` only when the user is clearly asking to search inside file contents, not when they are trying to locate entries by name.

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
- 中文路径作用域请求如果语义是在某个目录或范围内按名称定位条目，默认优先使用 `find_name`，不是 `grep_text`。
- 只有当用户明确是在找文件内容中的文本，而不是定位文件或目录名时，才应改用 `grep_text`。
