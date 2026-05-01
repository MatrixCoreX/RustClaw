# RustClaw

<img src="./RustClaw.png" width="420" />

这是一个简体中文入口索引页，方便快速找到当前最重要的文档。

完整版中文总览：

- `README.zh-CN.md`

英文版：

- `README.md`

## 我该先看哪个

如果你是第一次接触这个仓库，建议按下面顺序阅读：

1. `README.zh-CN.md`
2. `USAGE.md`
3. `UI/README.md`

## 按场景找文档

### 1. 想快速了解项目是什么

- `README.zh-CN.md`（含与实现对齐的架构与流程图）

### 2. 想安装、构建、启动、排障

- `USAGE.md`

### 3. 想看前端 UI 怎么开发和部署

- `UI/README.md`

### 4. 想新增普通 `runner` 技能

- `docs/skill_integration_guide.md`
- `skill_develop/README.md`
- `skill_develop/skill_authoring_strict.md`

### 5. 想看外部技能示例

- `docs/skill_integration_guide.md`
- `external_skills/example/README.md`

## 当前推荐入口

- 项目总览：`README.zh-CN.md`
- 运行时 / LLM 流程图与说明（与当前 `clawd` 实现一致）：`README.zh-CN.md` 内 **「规划优先架构」** 一节（含 Mermaid）
- 使用手册：`USAGE.md`
- 技能接入统一入口：`docs/skill_integration_guide.md`
- UI 说明：`UI/README.md`

## 架构与运行时流程

完整架构导语、**运行时流程**与 **LLM 请求流程**的两张图及条目说明，均以 `README.zh-CN.md` 为准；请直接打开该文件，从 **「规划优先架构」** 开始阅读。不要在本索引页维护第二套流程图，以免与总览不同步。
