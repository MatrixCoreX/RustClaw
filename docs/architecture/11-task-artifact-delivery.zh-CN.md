# 任务产物交付

<!-- ai-learning-stage: architecture-guide -->
<!-- ai-learning-audience: operator -->

<!-- ai-learning-navigation:start -->
上一页：[Web 入口与核心隔离](10-web-entry-security.zh-CN.md) |
[架构索引](README.md)

<!-- ai-learning-navigation:end -->

RustClaw 会把成功任务输出的文件转换为经过鉴权、可持久恢复的任务产物。浏览器读取
机器可解析的产物清单，根据类型展示预览或下载控件，不解析助手回复中的自然语言。

## 交付流程

```mermaid
flowchart LR
    A[Agent Loop]
    X[工具或技能执行]
    R[结构化任务结果]
    M[产物物化器]
    D[受控交付目录]
    J[任务结果产物清单]
    C[通信端适配器]
    W[webd 会话代理]
    N[可选 nginx]
    U[浏览器 UI]

    A --> X --> R
    R --> M --> D
    M --> J
    R --> C
    J --> W
    D -->|鉴权与分段传输| W
    W -->|直接部署| U
    W --> N -->|域名或 TLS 部署| U
```

任务成功时，`clawd` 收集结构化的本地输出引用，确认来源位于工作区内，将允许交付的
文件复制到 `.rustclaw/artifacts/delivery/<task_id>/<artifact_id>/`，并在持久化任务
结果中增加 `artifacts` 数组。每条清单包含稳定 ID、文件名、媒体类别、MIME 类型、
字节数、SHA-256 摘要，以及同源下载和预览路径。

dry-run 输出、工作区外路径、目录、缺失文件和超过交付上限的文件都不会暴露。产物
物化失败不会把原本成功的工具或技能执行改成失败任务，而是记录结构化交付警告。

## 浏览器访问

UI 通过 `webd` 使用以下已鉴权核心接口：

- `GET /v1/tasks/:task_id/artifacts`：返回受控产物清单。
- `GET /v1/tasks/:task_id/artifacts/:artifact_id/content`：流式传输内容。
- `HEAD`：只读取元数据，不传输文件正文。
- 支持单段字节范围，供音频、视频、PDF 和断点下载使用。

内容接口验证任务归属，并且只解析受控交付目录内的文件。响应包含安全的内容处置、
内容类型、ETag、`nosniff` 和分段请求头。位图、音频、视频和 PDF 可以安全内嵌预览；
SVG、HTML 等主动内容始终作为文件下载，不在页面内执行。

浏览器始终请求同源 `/v1` 路径。只使用 `webd` 时，请求直接代理到 loopback `clawd`；
使用 nginx 时，静态 UI 由 nginx 提供，`/v1` 仍经过 `webd`，因此会话和鉴权边界保持
一致。产物流使用长任务代理客户端，大文件不会被普通 API 请求超时提前中断。

## 通信端兼容

Telegram、微信、飞书、Lark、WhatsApp 等通信守护进程继续使用各自现有的文字和原生
媒体交付链路。任务顶层的产物清单只是新增字段，不替换 `text`、通信端消息数组、技能
`extra` 或现有媒体引用。通信端可以按自身需求显式接入清单，但浏览器下载接口不会成为
通信端交付的隐藏依赖。

这种隔离使每个通信端继续遵守自己的上传限制、格式和重试机制，浏览器则保持统一的鉴权
预览与下载语义。历史记录只恢复产物元数据和 URL，不把二进制或 base64 内容写入浏览器
本地存储。

## 生命周期与验证

删除任务时会删除对应的受控交付目录；后台清理也会移除找不到任务的孤立目录。原始工作区
文件仍由创建它的工具或技能管理。

```bash
cargo test -p clawd task_artifact
cargo test -p clawd conversation_history_projects_downloadable_task_artifacts
cargo test -p webd
cargo test -p telegramd
cd UI && node --import tsx --test src/lib/task-artifacts.test.ts src/lib/chat-history.test.ts
```

这些检查覆盖路径约束、鉴权、字节范围、历史恢复、安全预览策略、长任务代理路径，以及
通信端交付不受影响。
