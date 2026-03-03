# log_analyze

- 脚本：`scripts/skill_calls/call_log_analyze.sh`
- 默认参数：`{"action":"summary","path":"logs/clawd.log","limit":200}`
- 示例：
  - `bash scripts/skill_calls/call_log_analyze.sh`
  - `bash scripts/skill_calls/call_log_analyze.sh --args '{"action":"errors","path":"logs/telegramd.log","limit":100}'`
- 常用参数：`action`, `path`, `limit`, `pattern`
