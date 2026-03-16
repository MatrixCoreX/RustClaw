# Skill Develop

这个目录用于集中存放“让 agent 开发 RustClaw 技能”的提示词和说明文档。

## 当前文件
- `skill_authoring_strict.md`
  - 强约束版技能开发提示词。
  - 用于约束 agent 只按当前仓库的热插拔规则开发普通 `runner` 技能。
  - 重点是避免 agent 擅自修改 `clawd`、`skill-runner`、`agent_engine` 主程序。
- `create_skill.py`
  - 新技能脚手架工具。
  - 用于快速生成技能 crate、`INTERFACE.md`，并补 workspace 与 `skills_registry.toml` 基础条目。

## 适用场景
- 新增普通 `runner` 技能。
- 补齐某个技能的 `INTERFACE.md`、registry、prompt、配置。
- 让 agent 按固定流程接入技能，而不是自由发挥式修改仓库。

## 不适用场景
- 开发 `builtin` 技能。
- 修改 prompt 加载机制。
- 修改 skill runtime 协议。
- 变更主程序执行链路。

以上场景如果确实需要，应该先明确说明原因，再单独设计改动方案。

## 推荐使用方式
把 `skill_authoring_strict.md` 的内容直接作为系统提示词或任务前置约束发给 agent，并在任务里补充：
- 要开发的技能名称
- 技能目标
- 输入输出要求
- 是否需要独立配置文件
- 验证方式

如果是新增普通 runner 技能，建议先运行：

```bash
python3 skill_develop/create_skill.py <skill_name>
```

例如：

```bash
python3 skill_develop/create_skill.py stock --aliases "a_stock,stock_quote" --timeout 15
```

## 推荐任务模板
可以给 agent 这样的任务：

```text
请严格按照 `skill_develop/skill_authoring_strict.md` 的约束，为 RustClaw 新增一个 runner 技能 `<skill_name>`。

目标：
- <这里写技能目标>

要求：
- <这里写输入输出要求>
- <这里写是否需要配置文件>
- <这里写验证要求>
```

## 标准开发步骤
1. 运行 `python3 skill_develop/create_skill.py <skill_name>`
2. 实现单行 JSON stdin/stdout 协议
3. 补全 `INTERFACE.md`
4. 运行 `python3 scripts/sync_skill_docs.py`
5. 在 `prompts/agent_tool_spec.md` 增加参数契约
6. 如有 vendor 特化，再补 `prompts/vendors/<vendor>/skills/<skill_name>.md`
7. 运行 `cargo check -p clawd -p skill-runner -p <new-skill-package>`

## `create_skill.py` 支持项
- 创建 `crates/skills/<skill_name>/Cargo.toml`
- 创建 `crates/skills/<skill_name>/src/main.rs`
- 创建 `crates/skills/<skill_name>/INTERFACE.md`
- 自动加入根 `Cargo.toml` 的 workspace member
- 自动追加 `configs/skills_registry.toml` 基础条目
- 支持参数：
  - `--aliases`
  - `--timeout`
  - `--output-kind`
  - `--disabled`
  - `--runner-name`

查看帮助：

```bash
python3 skill_develop/create_skill.py --help
```

## 当前仓库约定
- 普通技能默认做成 `runner`
- 二进制命名约定：`foo_bar -> foo-bar-skill`
- skill prompt 运行时优先读取：
  1. `prompts/vendors/<vendor>/skills/<skill>.md`
  2. `prompts/vendors/default/skills/<skill>.md`
- 新增普通 runner 技能时，不应修改主程序代码

## 维护建议
- 若后续技能开发规则调整，优先更新本目录文档，再让 agent 使用新版提示词。
- 若将来有不同风格的开发约束，可继续在本目录增加：
  - `skill_authoring_simple.md`
  - `skill_authoring_builtin.md`
  - `skill_authoring_external.md`
