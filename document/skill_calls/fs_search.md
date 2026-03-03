# fs_search

- 脚本：`scripts/skill_calls/call_fs_search.sh`
- 默认参数：`{"action":"find_name","path":".","name":"README","limit":20}`
- 示例：
  - `bash scripts/skill_calls/call_fs_search.sh`
  - `bash scripts/skill_calls/call_fs_search.sh --args '{"action":"grep_text","path":"crates","query":"skill_runner","limit":50}'`
- 常用参数：`action`（`find_name|find_ext|grep_text|find_images`）, `path`, `query`, `name`, `limit`
