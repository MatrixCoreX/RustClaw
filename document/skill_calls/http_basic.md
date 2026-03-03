# http_basic

- 脚本：`scripts/skill_calls/call_http_basic.sh`
- 默认参数：`{"method":"GET","url":"https://api.github.com","timeout_seconds":15}`
- 示例：
  - `bash scripts/skill_calls/call_http_basic.sh`
  - `bash scripts/skill_calls/call_http_basic.sh --args '{"method":"GET","url":"https://httpbin.org/get"}'`
- 常用参数：`method`, `url`, `headers`, `body`, `timeout_seconds`
