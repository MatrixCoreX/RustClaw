# db_basic

- 脚本：`scripts/skill_calls/call_db_basic.sh`
- 默认参数：`{"action":"query","dialect":"sqlite","sql":"select 1 as ok;"}`
- 示例：
  - `bash scripts/skill_calls/call_db_basic.sh`
  - `bash scripts/skill_calls/call_db_basic.sh --args '{"action":"query","dialect":"sqlite","sql":"select count(*) as c from sqlite_master;"}'`
- 常用参数：`action`, `dialect`, `sql`, `dsn`
