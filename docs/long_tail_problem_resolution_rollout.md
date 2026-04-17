# 长尾问题解决闭环实施文案

## 目标

把 RustClaw 从“主要依赖单次指令/单次 skill 调用”升级成“面向长尾问题的通用闭环执行器”。

这套闭环不只覆盖运维，还覆盖：

- 运维/服务问题
- 配置变更问题
- 代码开发/修复问题
- skill 开发问题

共同目标不是“调用某个特定 skill”，而是：

- 根据用户当前问题先收集证据
- 制定可执行方案
- 实施变更
- 验证是否真的解决
- 失败时继续修复
- 在预算耗尽前尽量跑通

## 设计原则

1. 复用一套闭环骨架，不为每个工具单独长 skill
2. skill 开发单独建 profile，但和其他长尾任务共用同一套闭环引擎
3. 先做结构分层，再逐步强化验证、回滚和调度
4. 尽量优先结构化证据，少依赖字符串启发式

## 问题模型

长尾任务先分两层：

### 1. 任务 profile

- `ops_service`
  - 目标：安装、启动、修复、验证服务或系统环境
- `config_change`
  - 目标：修改配置并确认生效
- `code_change`
  - 目标：按用户当前需求修改代码、脚本或项目，直到问题解决
- `skill_authoring`
  - 目标：开发新的 skill 或扩展能力

### 2. 目标范围 scope

- `system`
  - 面向系统、服务、环境、网络
- `current_repo`
  - 面向当前仓库或当前项目
- `external_workspace`
  - 面向当前仓库之外的其他目录/项目
- `greenfield`
  - 从零创建新脚本、新项目、新工具

## 通用闭环

统一走：

`inspect -> plan -> apply -> validate -> repair -> stop`

说明：

- `inspect`
  - 收集当前状态、文件、日志、错误、上下文
- `plan`
  - 产出可执行动作序列
- `apply`
  - 修改文件、执行命令、调整服务、改代码
- `validate`
  - 用机器可验证信号确认结果
- `repair`
  - 验证失败时进入下一轮修复
- `stop`
  - 成功结束，或预算耗尽后明确失败

## 第一阶段落地策略

### 已决定

- 继续保留 `ExecutionRecipeKind::OpsClosedLoop` 作为运行时主 kind
- 新增 `profile + target_scope`，先做分类，不立即大改 loop 主逻辑
- 这样可以先把“运维/配置/代码/skill 开发”在结构上分开，同时复用当前已经稳定的闭环实现

### 第一阶段改动

1. 在 `execution_recipe` 中新增：
   - `ExecutionRecipeProfile`
   - `ExecutionRecipeTargetScope`

2. `detect_execution_recipe(...)` 输出：
   - `kind=ops_closed_loop`
   - `profile`
   - `target_scope`

3. prompt / trace / goal overlay / journal 中透出：
   - 当前 profile
   - 当前 target scope

4. loop policy 暂时仍复用 `ops_closed_loop` 的预算配置

## 第二阶段计划

1. router 显式输出 recipe profile
   - 不再主要依赖启发式检测

2. validation 结构化
   - `verified`
   - `reason`
   - `status_code`
   - `matched_marker`
   - `post_state`

3. 为 profile 提供更稳定的模板
   - `ops_service`
   - `config_change`
   - `code_change`
   - `skill_authoring`

4. 增加备份 / 回滚点
   - 尤其是配置和代码类变更

## 第三阶段计划

1. 按 profile 细化预算和调度
2. 支持更明确的暂停 / 恢复 / 后台继续执行
3. 强化普通用户隔离和 task 级权限边界

## 当前代码实施边界

第一阶段不做的：

- 不直接新增第二套 loop 系统
- 不把所有策略改成 profile 专属
- 不把 skill 开发自动化路由默认打开
- 不一次性引入完整回滚系统

## 验证方式

第一阶段至少保证：

- `execution_recipe` 单测覆盖 profile/scope 检测
- 现有 deterministic 闭环测试不回退
- focused NL 回归入口能继续工作

## 当前阶段的成功标准

- 长尾任务已不再只区分“是不是 ops_closed_loop”
- 运行时已经知道它处理的是：
  - 运维
  - 配置
  - 代码
  - skill 开发
- 同时知道目标范围是：
  - 系统
  - 当前仓库
  - 外部工作区
  - 从零创建

这为后续按 profile 做验证、回滚、调度和权限分层打基础。
