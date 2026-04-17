# External Skill Template (example)

这个目录是给外部开发者的提交示例。

## 目录说明

- `skill.Cargo.toml.template`：技能 crate 模板
- `skill.main.rs.template`：技能二进制入口模板
- `INTERFACE.md.template`：接口说明模板（必填）

## 如何使用

1. 复制本目录为你自己的技能目录，例如 `external_skills/weather_query`。
2. 将模板文件重命名为真实文件：
   - `skill.Cargo.toml.template` -> `Cargo.toml`
   - `skill.main.rs.template` -> `src/main.rs`
   - `INTERFACE.md.template` -> `INTERFACE.md`
3. 将模板中的 `your_skill_name` 替换为真实技能名（snake_case）。
4. 完整填写 `INTERFACE.md`，尤其是 capability summary、`Config Entry Points`、action、参数表、错误约定、JSON 示例。
5. 执行同步：
   - `python3 scripts/sync_skill_docs.py`
   - 如需校验：`python3 scripts/sync_skill_docs.py --check`
6. 如需真正接入运行时，还需要按当前外部技能接入方式完成对应的导入/注册流程；仅执行 `sync_skill_docs.py` 不代表该技能已经可以被运行时直接调用。

## 注意

- 对 `external_skills/*`，`INTERFACE.md` 是强制门禁，缺失会导致同步失败。
- `prompt_file = "prompts/skills/<skill>.md"` 只作为 registry 逻辑路径保存在配置里。
- 同步脚本会生成/更新实际正文 `prompts/layers/generated/skills/<skill>.md`，不建议手写旧的 `prompts/skills/` 路径。
- 当前仓库的技能协议建议显式兼容这些输入字段：
  - `request_id`
  - `args`
  - `context`
  - `user_id`
  - `chat_id`
- 当前仓库的技能响应除 `request_id`、`status`、`text`、`error_text` 外，也常见可选 `extra` 字段。
