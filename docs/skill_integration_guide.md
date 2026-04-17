# RustClaw 技能接入指南

这份文档是当前仓库的统一技能接入入口，用来回答 3 个问题：

1. 现在新增技能应该走哪条路径
2. 哪些文件必须改
3. 哪些文档和脚手架才是当前有效入口

## 先判断你要接入哪一类

### 1. 普通内建 `runner` 技能

适用场景：

- 新增 `crates/skills/<skill_name>` 下的技能
- 通过 `skill-runner` 按约定拉起
- 不希望改 `clawd`、`skill-runner` 主链路

首选入口：

- `skill_develop/README.md`
- `skill_develop/skill_authoring_strict.md`
- `skill_develop/create_skill.py`

推荐命令：

```bash
python3 skill_develop/create_skill.py <skill_name>
python3 scripts/sync_skill_docs.py
cargo check -p clawd -p skill-runner -p <new-skill-package>
```

### 2. 外部技能示例 / 外部提交技能

适用场景：

- 技能目录位于 `external_skills/<skill_name>`
- 先提供接口说明和模板实现
- 重点是让同步脚本识别并生成 prompt

入口：

- `external_skills/example/README.md`
- `external_skills/example/skill.Cargo.toml.template`
- `external_skills/example/skill.main.rs.template`
- `external_skills/example/INTERFACE.md.template`

注意：

- `python3 scripts/sync_skill_docs.py` 只负责 prompt 生成和门禁校验
- 这不等于外部技能已经完成运行时导入或可直接调用

### 3. 不属于普通 `runner` 的情况

如果你要做的是以下场景，不要直接套普通脚手架：

- `builtin` 技能
- 修改 prompt 加载机制
- 修改 skill runtime 协议
- 修改 `clawd` / `skill-runner` 主程序执行链路

这类改动应先单独设计方案，再实施。

## 当前仓库的最小接入要求

以普通 `runner` 技能为例，至少要完成这些项：

1. 新建 `crates/skills/<skill_name>/Cargo.toml`
2. 新建 `crates/skills/<skill_name>/src/main.rs`
3. 新建 `crates/skills/<skill_name>/INTERFACE.md`
4. 加入根 `Cargo.toml` 的 `[workspace].members`
5. 在 `configs/skills_registry.toml` 中新增 `[[skills]]`
6. 运行 `python3 scripts/sync_skill_docs.py`
7. 如有需要，在 `prompts/layers/overlays/agent_tool_spec.md` 增加技能契约

## 当前 prompt 路径约定

当前仓库的技能 prompt 不是旧版单层目录结构，而是分层组装：

1. canonical body：`prompts/layers/generated/skills/<skill>.md`
2. planner/tool overlay：`prompts/layers/overlays/agent_tool_spec.md`
3. vendor patch：`prompts/layers/vendor_patches/<vendor>/skills/<skill>.md`

补充说明：

- `configs/skills_registry.toml` 中的 `prompt_file = "prompts/skills/<skill>.md"` 目前是逻辑路径
- 运行时真正依赖的 canonical 主体内容，由 `scripts/sync_skill_docs.py` 维护在 `prompts/layers/generated/skills/<skill>.md`

## 当前技能协议建议

技能进程遵循：

- 单行 JSON stdin
- 单行 JSON stdout

当前建议显式兼容的输入字段：

- `request_id`
- `args`
- `context`
- `user_id`
- `chat_id`

当前常见输出字段：

- `request_id`
- `status`
- `text`
- `error_text`
- `extra`（可选）
- `buttons`（按需）

## 推荐阅读顺序

如果你要新增普通 `runner` 技能：

1. 先看 `AGENTS.md`
2. 再看 `skill_develop/README.md`
3. 再看 `skill_develop/skill_authoring_strict.md`
4. 然后运行 `python3 skill_develop/create_skill.py <skill_name>`

如果你要准备外部技能示例或外部提交：

1. 先看 `AGENTS.md`
2. 再看 `external_skills/example/README.md`
3. 按模板补齐 `INTERFACE.md`
4. 运行 `python3 scripts/sync_skill_docs.py`

## 一句话原则

普通 `runner` 技能优先走“crate + registry + INTERFACE + sync_skill_docs”这条路径；不要为了新增技能先去改主程序。
