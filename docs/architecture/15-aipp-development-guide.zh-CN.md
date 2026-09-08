# AiAPP 开发手册

<!-- ai-learning-stage: capabilities-artifacts -->
<!-- ai-learning-audience: developer -->

<!-- ai-learning-navigation:start -->
上一页：[AiPP 技能配套界面](14-aipp-skill-companions.zh-CN.md) |
[架构索引](README.md)
<!-- ai-learning-navigation:end -->

AiAPP 是由单个技能包声明并持有的可选可视化配套界面。它用于展示结构化技能数据，或调用
已经声明的 capability，但不会形成第二条执行链路。普通 AiAPP 的安装、更新、禁用和卸载
不得修改、重新编译或重启 `clawd`，也不得重新编译主 UI。

## 选择接入模式

现有合同足以表达数据时，使用已经审核的宿主 renderer：

| Renderer | 数据合同 | 数据来源 |
| --- | --- | --- |
| `collection_feed_v1` | `media_collection_v1` | 当前技能私有存储中的有界记录 |
| `task_activity_v1` | `skill_task_activity_v1` | 由结构化 `tool_finished.payload.skill` 事件筛选的运行时任务 |

需要自定义界面时，使用 `sandbox_bundle_v1 + capability_bridge_v1`。前端由技能包独立持有和
构建；主 UI 只在沙箱中托管不可变静态产物，并把白名单 capability 请求交给正常的 resolver
和 verifier。

不得在 `clawd` 或 `UI/src/components/AippPage.tsx` 中增加具体技能名、技能专属路由、技能
专属 import 或自定义回复解析器。确实需要新增可复用宿主合同时，应作为单独的平台变更进行
schema 与安全审核，不能混入普通 App 安装。

## 技能包目录

宿主 renderer 类型只需要正常技能文件：

```text
optional_skills/example/
  skill.toml
  INTERFACE.md
  runtime/
```

自定义 App 的源码、依赖和静态产物都归技能包所有：

```text
optional_skills/example/
  skill.toml
  INTERFACE.md
  runtime/
  aipp-src/          # 可选源码，只由当前技能的打包流程构建
  aipp/
    index.html
    app.js
    app.css
```

普通主 UI 构建不得进入 `aipp-src/`。技能包构建或发行流程必须在准入前生成 `aipp/`。所有
可访问的 bundle 文件都要写入不可变 package receipt，并在提供给浏览器前再次验证。

## Manifest 示例

展示 Agent UI 和全部通信端任务：

```toml
[aipp]
schema_version = 1
renderer = "task_activity_v1"
data_contract = "skill_task_activity_v1"
task_channel_scope = "all"
icon = "download"
default_locale = "en"
titles = { en = "Example Activity", zh = "示例活动" }
descriptions = { en = "Review completed work.", zh = "查看已完成的工作。" }
```

只有明确需要排除 Agent UI 任务时才使用 `task_channel_scope = "communication"`。任务归属
必须依据结构化执行事件，不得匹配请求或回复的自然语言。

自定义沙箱 App：

```toml
[aipp]
schema_version = 1
renderer = "sandbox_bundle_v1"
data_contract = "capability_bridge_v1"
asset_root = "aipp"
entrypoint = "aipp/index.html"
bridge_capabilities = ["example.status", "example.list"]
icon = "panels_top_left"
default_locale = "en"
titles = { en = "Example", zh = "示例" }
descriptions = { en = "Review example data.", zh = "查看示例数据。" }
```

每个 bridge capability 还必须出现在 `capability_request.capabilities`。manifest 只能申请，
不能自行授权；最终执行仍由准入、宿主 policy、启用状态和固定 registry generation 决定。

## 浏览器 Bridge

bundle 在允许脚本和下载、但没有同源权限的 iframe 中运行。它不能获得 cookie、API key、
凭据、父页面 DOM、任意网络访问或本地原始路径。调用使用版本化消息：

```json
{"schema_version":1,"type":"aipp.ready"}
{"schema_version":1,"type":"aipp.capability.invoke","request_id":"status-1","capability":"example.status","args":{}}
```

宿主校验消息来源窗口、schema、参数序列化大小、并发数量和 manifest 白名单，再通过
`aipp.host.context` 与 `aipp.capability.result` 返回结果。App 从结构化字段生成本地化展示；
不得通过模型自然语言判断状态、归属、成功、重试或权限。

## 数据归属

- Collection renderer 只能读取声明它的技能经 `SkillStorageResolver` 解析出的私有存储。
- Task activity renderer 只能读取存在当前技能结构化执行事件的任务；`task_channel_scope =
  "all"` 同时包含 UI 与外部通信端。
- 预览和工件路由必须认证，只返回有界且在允许列表内的文件，不能暴露私有存储路径。
- 单独卸载 AiAPP 只写展示 tombstone，不卸载或禁用技能，也不删除配置和私有数据。
- 卸载技能走正常准入生命周期；已运行调用使用固定版本 lease 收尾。

## 安装生命周期

1. 校验 `skill.toml`、capability request、包路径和静态文件。
2. 在技能自己的构建流程中构建技能和可选自定义前端。
3. 运行协议冒烟，并验证每个 package artifact digest。
4. 写入不可变 package receipt 与宿主 policy grant。
5. 以一个事务提交包含技能和 AiAPP 状态的 overlay generation。
6. 显式启用技能；AiAPP 只跟随当前精确包绑定生效。
7. 通过 overlay tombstone 独立安装或删除 AiAPP 展示。

整个流程都不能修改 tracked source、外部技能不能修改根 Cargo workspace，也不能修改主 UI
bundle 或重启正在运行的 `clawd`。

## 必须执行的验证

交付前运行：

```bash
python3 scripts/check_aipp_decoupling.py --self-test
python3 scripts/check_aipp_decoupling.py
python3 scripts/sync_skill_docs.py
python3 scripts/check_skill_prompts.py
target/debug/skillctl validate optional_skills/example/skill.toml
cargo test -p agent-skill-sdk aipp_manifest --no-fail-fast
cargo test -p clawd aipp_tests --no-fail-fast
```

宿主 renderer App 还要验证游标分页、筛选、权限、字段边界、secret 脱敏、App 独立卸载与重装、
技能禁用与恢复，以及不同技能之间零串数据。沙箱 bundle 还要验证 receipt 篡改、路径穿越、
符号链接、不支持的文件类型、安全响应头、bridge 白名单拒绝、参数上限和并发上限。只有修改了
通用宿主 renderer 或 bridge 才构建、测试主 UI；新增普通技能包 AiAPP 不得要求主 UI 构建。

修改通用 manifest 或宿主合同时，必须以同一发行版本重新构建并部署所有使用
`agent-skill-sdk` 的二进制，包括 `clawd`、`skill-runner` 和 `skillctl`，部署后至少对一个已安装
技能执行真实协议调用。新增的可选 manifest 字段要为已经安装的不可变旧包提供安全默认值，
新源码 manifest 仍受当前门禁约束。即使服务能够启动，只更新宿主或 runner 中的一部分也属于
无效部署。

## Review 清单

- App 只由所属技能的 `skill.toml` 声明。
- 通用 AiAPP 宿主路由和 renderer 中不存在具体技能名。
- 主 UI 不 import 技能前端源码。
- 安装、更新和删除不会重新构建或重启宿主。
- Runtime 决策读取稳定机器字段，不匹配自然语言。
- package asset、prompt、grant 和版本绑定到同一个不可变 receipt/generation。
- UI 文案由技能包或现有 locale 字段本地化。
- App 只暴露完成任务所需的最小结构化数据和 capability。
