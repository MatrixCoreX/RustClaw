# image_vision

- 脚本：`scripts/skill_calls/call_image_vision.sh`
- 默认参数：`{"action":"describe","images":[]}`
- 示例：
  - `bash scripts/skill_calls/call_image_vision.sh --args '{"action":"describe","images":[{"path":"image/upload/demo.png"}]}'`
  - `bash scripts/skill_calls/call_image_vision.sh --args '{"action":"compare","images":[{"path":"a.png"},{"path":"b.png"}]}'`
- 常用参数：`action`, `images`, `language`
