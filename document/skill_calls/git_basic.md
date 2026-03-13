# git_basic

- 脚本：`scripts/skill_calls/call_git_basic.sh`
- 默认参数：`{"action":"status"}`
- 示例：
  - `bash scripts/skill_calls/call_git_basic.sh`
  - `bash scripts/skill_calls/call_git_basic.sh --args '{"action":"log","n":5}'`
  - `bash scripts/skill_calls/call_git_basic.sh --args '{"action":"log","limit":5}'`（limit 为 n 的别名）
- 常用参数：`action`；`log` 用 `n` 或 `limit`（条数）；`show` 用 `target`；`show_file_at_rev` 用 `target`+`path`。
- 只读技能：支持 status / log / diff / diff_cached / branch / current_branch / remote / changed_files / show / show_file_at_rev / rev_parse。
