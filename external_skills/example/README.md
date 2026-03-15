# External Skill Template (exampe)

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
4. 完整填写 `INTERFACE.md`，尤其是 action、参数表、错误约定、JSON 示例。
5. 执行同步：
   - `python3 scripts/sync_skill_docs.py`
   - 如需校验：`python3 scripts/sync_skill_docs.py --check`

## 注意

- 对 `external_skills/*`，`INTERFACE.md` 是强制门禁，缺失会导致同步失败。
- `prompts/skills/<skill>.md` 由同步脚本自动生成/维护，不建议手写。
