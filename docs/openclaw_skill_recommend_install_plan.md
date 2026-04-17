# RustClaw OpenClaw 技能发现与安装计划

日期：2026-04-11

## 1. 目标

当 RustClaw 当前技能不足以完成用户任务时，系统可以：

1. 判断当前能力缺口是否明显存在。
2. 去 OpenClaw / ClawHub 搜索可能可用的 skill。
3. 把推荐结果明确展示给用户，而不是直接自动安装。
4. 在用户明确确认后，完成安装并接入 RustClaw。
5. 在 UI 中提供同一条能力链路，而不是只停留在聊天侧。

这个功能的核心不是“自动装更多插件”，而是把“能力不足 -> 找候选 -> 用户确认 -> 安装接入”变成一条可控、可解释、可回滚的产品链路。

## 2. 非目标

第一阶段不做这些事：

1. 不做无确认的静默自动安装。
2. 不允许普通聊天用户无门槛安装第三方 skill。
3. 不把 OpenClaw skill 直接当成 RustClaw 原生 runner skill 执行。
4. 不重写现有 planner 主链，只做增量接入。
5. 不依赖“远程贴一个仓库链接”完成多文件 OpenClaw skill 导入。

## 3. 当前仓库现状

### 3.1 RustClaw 已有的相关能力

仓库中已经有三块可复用能力：

1. 外部技能导入
   - [crates/clawd/src/http/ui_routes.rs](../crates/clawd/src/http/ui_routes.rs)
   - 当前已经支持把外部 bundle 导入为 RustClaw external skill，并写入 `skills_registry.toml`。
2. 技能未开启 / 不可用拦截
   - [crates/clawd/src/skills.rs](../crates/clawd/src/skills.rs)
   - 当前 `run_skill_with_runner_outcome()` 在技能不可执行时直接返回错误。
3. 明确确认后继续执行
   - [crates/clawd/src/agent_engine.rs](../crates/clawd/src/agent_engine.rs)
   - [crates/clawd/src/worker/ask_prepare.rs](../crates/clawd/src/worker/ask_prepare.rs)
   - 当前已经有 `resume_context` 机制，可以在用户回复“继续”后恢复剩余动作。

### 3.2 当前 OpenClaw 兼容边界

目前 RustClaw 对 OpenClaw skill 不是“完整兼容”，而是“部分兼容”：

1. 本地文件夹上传相对可靠。
2. 远程链接导入只会拿到一个 `SKILL.md`，不适合多文件 bundle。
3. 执行模型更像 RustClaw external skill wrapper，而不是 OpenClaw 原生运行协议。

因此，这次功能不能建立在“让用户贴一个 OpenClaw GitHub 链接就自动可用”的假设上。

## 4. 推荐总体方案

推荐把整条链路拆成 4 层：

1. `openclaw_market`：负责查询 OpenClaw / ClawHub 候选 skill。
2. `openclaw_installer`：负责把选中的 skill 安装到临时目录。
3. `external_bundle_importer`：负责把安装出来的本地 bundle 导入到 RustClaw registry。
4. `resume_confirm_flow`：负责“先推荐、等用户确认、再执行安装”。

这四层中，RustClaw 已经部分具备第 3 层和第 4 层，需要补的是第 1 层与第 2 层，以及把它们串起来。

## 5. 关键设计决定

### 5.1 搜索与安装分离

不要让 agent 直接在推理过程中拼接 shell 命令去搜索和安装。

应该新增统一服务层，例如：

- `search_openclaw_skills(query) -> candidates`
- `inspect_openclaw_skill(slug) -> metadata`
- `install_openclaw_skill_to_temp(slug, version) -> local_bundle_dir`

这样做的原因：

1. 便于统一权限控制。
2. 便于审计。
3. 便于后续 UI 和聊天共用。
4. 避免 agent 自由拼命令导致行为不可控。

### 5.2 安装到临时目录，再导入 RustClaw

不要直接让 OpenClaw 安装命令写进 RustClaw 最终可执行目录。

推荐流程：

1. 用 OpenClaw 官方安装命令把 skill 安装到临时目录。
2. 定位实际 bundle 目录。
3. 调用 RustClaw 自己的 external bundle 导入逻辑。
4. 写入 `configs/skills_registry.toml`。
5. 必要时写入 `configs/config.toml` 的 `skill_switches`。
6. `reload_skill_views()`。

这样可以把“外部生态安装”和“RustClaw 运行时接入”明确分层。

### 5.3 先推荐，再确认，再安装

不做 silent install。

推荐复用现有 `resume_context` 机制：

1. 搜索到候选后，先给用户一个明确的推荐回复。
2. 把待安装信息写入 `resume_context`。
3. 用户回复“继续 / 安装 / yes”等明确确认词后，恢复执行安装动作。

这样不需要额外发明一套新的 pending-confirm 状态机。

### 5.4 UI 与聊天使用同一套后端能力

不要做成“聊天是一套逻辑，UI 导入又是另一套逻辑”。

推荐：

1. 聊天侧调用统一的搜索 / 安装服务。
2. UI 技能页增加“从 OpenClaw 搜索”入口，直接调用同一套后端接口。
3. UI 只负责展示推荐结果、确认安装和展示安装结果，不重复实现导入分析器。

## 6. 推荐插点

### 6.1 V1 主插点：技能执行不可用时

第一阶段最稳的插点是：

- [crates/clawd/src/skills.rs](../crates/clawd/src/skills.rs)

在这些场景触发推荐：

1. agent 计划调用某技能，但当前技能未开启。
2. agent 计划调用某技能，但当前 agent 不允许该技能。
3. 用户明确说“安装一个 xxx 技能 / 找一个能做这个的 skill”。

这样能先避开全局意图猜测，降低误报。

### 6.2 V2 插点：能力缺口检测

第二阶段再增强为“未明确点名 skill，但当前能力明显不够时自动推荐”。

可以利用：

- [crates/clawd/src/capability_map.rs](../crates/clawd/src/capability_map.rs)
- [crates/clawd/src/runtime/state.rs](../crates/clawd/src/runtime/state.rs)

在 planner 可见技能集不足以覆盖某类任务时，生成 OpenClaw 搜索建议。

## 7. 需要新增的模块

建议新增这些模块：

### A. `crates/clawd/src/openclaw_market.rs`

职责：

1. 封装 OpenClaw 官方 CLI 的搜索能力。
2. 统一解析 JSON 输出。
3. 做候选排序和截断。

建议接口：

1. `search_skills(query, limit) -> Vec<OpenClawSkillCandidate>`
2. `get_skill_info(slug) -> OpenClawSkillInfo`

### B. `crates/clawd/src/openclaw_install.rs`

职责：

1. 调用 OpenClaw 官方安装命令。
2. 安装到临时目录。
3. 返回 bundle 路径和安装元数据。

建议接口：

1. `install_skill_to_temp(slug, version, workspace) -> InstalledOpenClawBundle`

### C. `crates/clawd/src/external_skill_import.rs`

职责：

1. 从 `ui_routes.rs` 中抽离外部 bundle 导入逻辑。
2. 允许 UI 和聊天安装链路共用。

建议把这些现有逻辑迁入此模块：

1. bundle 扫描
2. import plan 分析
3. registry block 生成
4. prompt wrapper 生成
5. 导入后 `reload_skill_views()`

## 8. 推荐的数据结构

### 8.1 搜索结果

```rust
struct OpenClawSkillCandidate {
    slug: String,
    display_name: String,
    summary: String,
    author: Option<String>,
    version: Option<String>,
    homepage: Option<String>,
    tags: Vec<String>,
    install_hint: Option<String>,
    score: f32,
}
```

### 8.2 待确认安装上下文

```json
{
  "resume_context_id": "ctx-...",
  "action": "install_openclaw_skill",
  "query": "read rss and summarize",
  "candidate": {
    "slug": "rss-reader",
    "display_name": "RSS Reader",
    "version": "1.2.0"
  },
  "reason": "当前 RustClaw 没有明显可用的 RSS 获取能力",
  "requires_admin": true,
  "source": "openclaw_search"
}
```

### 8.3 安装结果

```rust
struct OpenClawInstallResult {
    slug: String,
    installed_bundle_dir: PathBuf,
    imported_skill_name: String,
    restart_required: bool,
}
```

## 9. 用户交互设计

### 9.1 聊天侧

推荐回复结构：

1. 先明确说明当前 RustClaw 本地技能不足。
2. 给出 1 到 3 个 OpenClaw 候选 skill。
3. 明确说明将发生什么：
   - 会下载并导入第三方 skill
   - 可能需要额外依赖
   - 需要管理员确认
4. 提示用户回复“继续”或更明确的安装指令。

不建议：

1. 一次性列很多候选。
2. 不解释来源就让用户确认。
3. 用户只说了一个模糊“好”就安装。

### 9.2 UI 侧

第一阶段 UI 只做最小闭环：

1. 技能页新增“从 OpenClaw 搜索”入口。
2. 搜索结果卡片展示：
   - 名称
   - 简介
   - 来源
   - 运行方式
   - 可能依赖
3. 安装按钮点击后弹出明确确认。
4. 安装完成后高亮新 skill，并沿用现有“保存开关 / 自动重启”体验。

## 10. 权限与安全

这是高风险入口，必须单独做门禁。

### 10.1 权限要求

建议：

1. 搜索能力可开放给普通用户。
2. 安装能力仅管理员可用。
3. UI 安装接口沿用 `require_ui_identity()` 的 admin 检查。
4. 聊天侧若不是 admin，最多只返回推荐，不执行安装。

### 10.2 配置开关

建议新增：

```toml
[openclaw_integration]
enabled = true
search_enabled = true
install_enabled = true
admin_only = true
search_limit = 5
install_temp_dir = "/tmp/rustclaw-openclaw"
```

### 10.3 审计与提示

必须记录：

1. 谁发起了搜索。
2. 谁确认了安装。
3. 安装了哪个 slug、哪个版本。
4. 导入成了哪个 RustClaw skill 名称。

必须提示：

1. 这是第三方 skill。
2. 需要哪些本地依赖。
3. 是否需要重启或重新加载。

## 11. 实施阶段

### Phase 1. 抽离 bundle 导入服务

目标：

1. 让 UI 和未来自动安装链路共用同一套 external skill 导入逻辑。

工作项：

1. 从 `ui_routes.rs` 抽离 import / detect / finalize 逻辑。
2. 提供“从本地 bundle 目录导入”的可复用服务接口。

完成标准：

1. UI 当前导入功能行为不变。
2. 聊天侧和后台任务也能调用相同导入逻辑。

### Phase 2. 接入 OpenClaw 搜索服务

目标：

1. 能在 RustClaw 内部以结构化方式检索 OpenClaw skill。

工作项：

1. 新增 `openclaw_market.rs`。
2. 封装官方 CLI 搜索命令。
3. 做 JSON 解析、候选排序、错误处理。

完成标准：

1. 给定 query 能返回稳定结构化候选列表。
2. 搜索失败时返回清晰错误，不影响主链稳定性。

### Phase 3. 接入“先推荐，再确认”

目标：

1. 当本地能力不足时，系统能提出安装建议，但不会自动执行。

工作项：

1. 在 `skills.rs` 的不可用技能分支接入推荐逻辑。
2. 生成用户可见提示和 `resume_context`。
3. 复用现有 `resume_continue_execute` 链路。

完成标准：

1. 用户能收到明确推荐。
2. 用户明确确认后，系统能恢复执行安装动作。

### Phase 4. 接入实际安装与导入

目标：

1. 用户确认后，RustClaw 能完成完整安装接入。

工作项：

1. 新增 `openclaw_install.rs`。
2. 安装到临时目录。
3. 调用 external bundle importer。
4. `reload_skill_views()`。

完成标准：

1. 安装成功后，新 skill 能在 RustClaw 中可见。
2. 安装失败时用户能看到明确失败原因。

### Phase 5. UI 搜索与安装入口

目标：

1. 在控制台中提供可视化搜索与安装体验。

工作项：

1. 技能页新增 OpenClaw 搜索框与结果卡片。
2. 新增安装确认弹窗。
3. 安装后高亮 skill，并接现有保存开关 / 重启提示。

完成标准：

1. 普通用户能看懂“这是什么、会发生什么、下一步做什么”。
2. UI 与聊天侧使用同一套后端逻辑。

### Phase 6. 能力缺口自动发现

目标：

1. 不依赖“点名 skill”，而是在本地能力明显不足时自动推荐。

工作项：

1. 基于 capability map 增加轻量缺口检测。
2. 把推荐范围控制在明确能力域。
3. 避免在可由现有技能完成时误触发。

完成标准：

1. 自动推荐误报率可接受。
2. 不明显增加无意义搜索和额外轮次。

## 12. 风险点

### 12.1 远程链接导入不可靠

当前 RustClaw 的远程导入链路不适合多文件 OpenClaw bundle，因此安装必须优先走“本地临时目录 -> 本地 bundle 导入”。

### 12.2 第三方 skill 安全风险

第三方 AI skill / 插件市场天然带来安全风险，因此必须保留：

1. 管理员权限
2. 显式确认
3. 审计日志
4. 可关闭配置开关

### 12.3 Planner 误判

如果过早做“全局能力不足自动发现”，容易把本地已有能力误判成缺口，导致不必要的搜索和安装建议。

因此建议先做“不可执行 skill 时再推荐”的保守版本。

## 13. 最小验证

每个阶段至少验证：

1. `cargo check -p clawd`
2. 新增对应模块的定向测试
3. 至少一条 happy path：
   - 搜索成功
   - 用户确认
   - 安装成功
   - 导入成功
   - 新 skill 出现在技能页
4. 至少一条失败路径：
   - 无搜索结果
   - 非 admin 触发安装
   - OpenClaw 安装失败
   - 导入失败

如果改到 UI，再补：

1. `cd UI && npm run lint`
2. `cd UI && npm run build`

## 14. 推荐实施顺序

建议按这个顺序推进：

1. 抽离 external bundle importer
2. 接入 OpenClaw 搜索服务
3. 接入推荐 + 明确确认链路
4. 接入实际安装
5. 补 UI 搜索入口
6. 最后再做能力缺口自动发现

## 15. 参考资料

1. OpenClaw CLI Skills 文档：https://docs.openclaw.ai/cli/skills
2. OpenClaw ClawHub 文档：https://docs.openclaw.ai/tools/clawhub
3. 第三方技能市场安全背景参考：https://www.theverge.com/news/874011/openclaw-ai-skill-clawhub-extensions-security-nightmare
