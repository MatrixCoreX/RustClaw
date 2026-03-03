# image_edit

- 脚本：`scripts/skill_calls/call_image_edit.sh`
- 默认参数：`{"action":"edit","instruction":"increase contrast slightly"}`
- 示例：
  - `bash scripts/skill_calls/call_image_edit.sh --args '{"action":"edit","image":"image/upload/demo.png","instruction":"remove background"}'`
  - `bash scripts/skill_calls/call_image_edit.sh --args '{"action":"restyle","image":"image/upload/demo.png","instruction":"watercolor style"}'`
- 常用参数：`action`, `image`, `instruction`, `mask`, `output_path`
