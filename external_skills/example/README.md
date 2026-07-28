# External Skill Template (example)

这个目录是给外部开发者的提交示例。

## 目录说明

- `INTERFACE.md.template`：接口说明参考（必填）。
- 可执行模板由统一 SDK CLI 生成，不在这里维护第二套 Rust 专用副本。

## 如何使用

1. 运行 `rustclaw-skill init <rust|python|node|go|prebuilt> <skill_name> external_skills/<skill_name>`。
2. 完整填写生成的 `INTERFACE.md` 与 `skill.toml`。
3. 在生成的语言入口中实现业务逻辑，测试保留在独立测试文件。
4. 完整填写 `INTERFACE.md`，尤其是 capability summary、`Config Entry Points`、action、参数表、错误约定、JSON 示例。
5. 执行同步：
   - `python3 scripts/sync_skill_docs.py`
   - 如需校验：`python3 scripts/sync_skill_docs.py --check`
6. 如需真正接入运行时，还需要按当前外部技能接入方式完成对应的导入/注册流程；仅执行 `sync_skill_docs.py` 不代表该技能已经可以被运行时直接调用。

## 注意

- 对 `external_skills/*`，`INTERFACE.md` 是强制门禁，缺失会导致同步失败。
- `prompt_file = "prompts/skills/<skill>.md"` 只作为 registry 逻辑路径保存在配置里。
- 同步脚本会生成/更新实际正文 `prompts/layers/generated/skills/<skill>.md`，不建议手写旧的 `prompts/skills/` 路径。
- `rustclaw-jsonl-v1` 必须兼容这些输入字段：
  - `request_id`
  - `args`
  - `context`
  - `user_id`
  - `chat_id`
- 当前仓库的技能响应除 `request_id`、`status`、`text`、`error_text` 外，也常见可选 `extra` 字段。
