# Skill Develop

这个目录用于集中存放“让 agent 开发 RustClaw 技能”的提示词和说明文档。

## 当前文件
- `skill_authoring_strict.md`
  - 强约束版技能开发提示词。
  - 用于约束 agent 只按当前仓库的热插拔规则开发普通 `runner` 技能。
  - 重点是避免 agent 擅自修改 `clawd`、`skill-runner`、`agent_engine` 主程序。
- `create_skill.py`
  - 仓内 runner 技能脚手架工具。
  - 用于快速生成技能 crate、`INTERFACE.md`，并补 workspace 与 `skills_registry.toml` 基础条目。

## 适用场景
- 新增仓内普通 `runner` 技能（`crates/skills/<skill_name>`）。
- 整理外部提交技能（`external_skills/<skill_name>`）的接入规则。
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

如果是新增仓内普通 runner 技能，建议先运行：

```bash
python3 skill_develop/create_skill.py <skill_name>
```

例如：

```bash
python3 skill_develop/create_skill.py stock --aliases "a_stock,stock_quote" --timeout 15
```

如果是外部提交技能，不使用 `create_skill.py`，走 `extension_manager`：

```text
1. scaffold_external_skill / implement_external_skill 生成或补全 external_skills/<skill_name>
2. validate_external_skill 运行 sync_skill_docs.py、cargo check 和协议 smoke test
3. register_external_skill(confirm=true) 构建 release binary，并自动写入 workspace、skills_registry.toml、configs/config.toml 的 skill_switches.<skill>=true
4. reload skills 或重启 clawd 后再跑 run_skill happy path
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

## 仓内 runner 标准开发步骤
1. 运行 `python3 skill_develop/create_skill.py <skill_name>`
2. 实现单行 JSON stdin/stdout 协议
3. 补全 `INTERFACE.md`
4. 运行 `python3 scripts/sync_skill_docs.py`
5. 在 `prompts/layers/overlays/agent_tool_spec.md` 增加参数契约
6. 如有 vendor 特化，再补 `prompts/layers/vendor_patches/<vendor>/skills/<skill_name>.md`
7. 运行 `cargo check -p clawd -p skill-runner -p <new-skill-package>`

## 外部 skill 标准接入步骤
1. 准备或生成 `external_skills/<skill_name>`，目录内必须有 `Cargo.toml`、`README.md`、`INTERFACE.md`、`src/main.rs`
2. 补全 `INTERFACE.md`，确保 action、参数、错误和请求/响应示例真实可用
3. 运行 `validate_external_skill`，通过 `sync_skill_docs.py`、`cargo check` 和单行 JSON smoke test
4. 验证/编译通过后运行 `register_external_skill` 且 `confirm=true`
5. `register_external_skill` 成功后会构建 release binary，写入根 `Cargo.toml`、`configs/skills_registry.toml`，并把 `configs/config.toml` 的 `skill_switches.<skill_name>` 自动写成 `true`
6. reload skills 或重启 `clawd`，再用 `run_skill` 路径跑一次 happy path

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
- 新增普通仓内 runner 技能时，不应修改主程序代码
- 外部提交技能优先落在 `external_skills/<skill_name>`，通过 `extension_manager` 注册，不应为了新增 skill 修改 `clawd` 主流程
- 外部技能验证/编译通过后，`register_external_skill(confirm=true)` 会自动写 `configs/config.toml` 的 `skill_switches.<skill_name>=true`
- skill prompt 运行时组装方式：
  1. canonical body: `prompts/layers/generated/skills/<skill>.md`
  2. planner/tool 约束叠加：`prompts/layers/overlays/agent_tool_spec.md`
  3. optional vendor patch: `prompts/layers/vendor_patches/<vendor>/skills/<skill>.md`
- `configs/skills_registry.toml` 中的 `prompt_file = "prompts/skills/<skill>.md"` 是逻辑路径；运行时实际主内容由 `scripts/sync_skill_docs.py` 维护的 `prompts/layers/generated/skills/<skill>.md` 提供
- 技能输入协议虽然最少只要能正确处理 `request_id` 和 `args` 就能工作，但新技能脚手架与文档应按当前主协议显式包含：
  - `request_id`
  - `args`
  - `context`
  - `user_id`
  - `chat_id`

## 维护建议
- 若后续技能开发规则调整，优先更新本目录文档，再让 agent 使用新版提示词。
- 若将来有不同风格的开发约束，可继续在本目录增加：
  - `skill_authoring_simple.md`
  - `skill_authoring_builtin.md`
  - `skill_authoring_external.md`
