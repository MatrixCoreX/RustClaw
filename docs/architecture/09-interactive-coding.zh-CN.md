# 交互式编码与输出呈现

<!-- ai-learning-stage: development-release -->
<!-- ai-learning-audience: developer -->

<!-- ai-learning-navigation:start -->
上一页：[技能独立存储](08-skill-owned-storage.zh-CN.md) |
[架构索引](README.md) |
下一页：[Web 入口与核心隔离](10-web-entry-security.zh-CN.md)

<!-- ai-learning-navigation:end -->

RustClaw 继续由单一 agent loop 负责语义决策，确定性 runtime 只负责 schema、
权限、路径边界、副作用与证据。交互式编码增加公开输出流和更安全的本地编辑
能力。

## 私有事件面与公开事件面

```mermaid
flowchart LR
    A[Provider 原生流] --> B[Model-turn 解码]
    B --> C[私有模型可观测事件]
    B --> D[Agent 动作解析]
    D -->|工具动作| E[Resolver + verifier]
    E --> F[工具执行 + 证据]
    F --> D
    D -->|已识别终态答案| G[用户可见输出策略]
    G --> H[助手公开呈现事件]
    H --> I[持久化任务 Journal + SSE]
    I --> J[UI reducer]
    I --> K[CLI renderer / JSONL]
    D --> L[最终任务结果物化]
    L --> J
    L --> K
```

Provider 的 `TextDelta` 内容不能直接进入 SSE。当前增量解析器只在证明字节属于
原生终态 `respond` 且 `shape=free_text` 后才允许公开；完整 UTF-8 片段还要通过
用户可见输出策略。其他回复形状和结构化 plan 格式只发送终态结果。

每次公开答案尝试都有稳定的 stream ID 和 attempt ID。如果后续 verifier 要求
重试，就发送 abort 与 replacement 事件，不能把第二个答案追加到第一个答案。
完成事件记录总字节数和 SHA-256。UI 与 CLI 用它和最终任务结果对账，最终任务
结果始终是权威结果。

## 精确本地编辑

```mermaid
flowchart TD
    A[Agent 选择本地修改] --> B{修改形态}
    B -->|一次精确且有界的替换| C[workspace.preview_replace_text]
    C --> D[路径 + 出现次数 + hash + diff 预览]
    D --> E[Verifier / 审批]
    E --> F[workspace.replace_text]
    B -->|多文件或多 hunk| G[workspace.apply_patch]
    B -->|新文件或整文件重写| H[filesystem.write_file]
    F --> I[原子写入 + checkpoint]
    G --> I
    H --> I
    I --> J[Diff + hash + rewind 证据]
```

精确替换要求目标只出现一次。找不到或出现多次都不得修改文件。可选的前置
hash 用于发现预览后文件已经变化。修改必须保留 UTF-8 与换行风格，使用原子
写入，并复用工作区 checkpoint、diff 与 rewind 层。

重放由 runtime 幂等账本在执行前判断。同一个 idempotency key 返回已经记录的
结果；新的 key 会按当前文件状态重新执行，因此可能返回
`replacement_target_not_found`。

## 持久化 CLI 会话

```mermaid
flowchart TD
    A[终端输入] --> B[命令与 @path 语法]
    B --> C[共享 ChatSessionState]
    C --> D{操作类型}
    D -->|消息 / 附件| E[提交或调整任务]
    D -->|模型 / 权限| F[后端策略校验]
    D -->|compact| G[持久化会话压缩]
    D -->|diff| H[工作区 diff 证据]
    D -->|resume| I[恢复会话 / 任务]
    E --> J[公开呈现 + 机器事件]
    F --> C
    G --> C
    H --> C
    I --> C
    J --> K[人类 renderer 或纯 JSONL]
```

CLI 只保存安全的 ID 和偏好，不把本地缓存当作任务或策略权威来源。模型与权限
修改只影响当前会话，并由后端校验。上下文压缩必须保留目标、约束、审批、已完成
副作用、修改文件、工件引用、待办工作和 resume cursor。

`@path`、斜杠命令和附件命令都是显式语法，不使用自然语言短语匹配。路径物化
复用工作区边界、ignore/secret 策略、符号链接检查、大小限制和内容 hash。

CLI 最多保留 10 个待发送附件，单文件上限 20 MiB，总上限 60 MiB。会话只保存
安全的附件 metadata 与内容 hash；提交任务时重新读取并核对 hash，成功提交后
清空待发送集合。模型选择与 `safe|ask|yolo` 是会话范围的请求，最终仍由已认证
服务端校验模型并签发执行策略。

浏览器对话恢复同样以服务端为权威。`GET /v1/tasks/conversation-history` 返回
经过认证、按 owner 过滤、使用 cursor 分页的 ask 轮次，其中只包含有界展示文本、
task 状态、附件种类/数量、持久化的自定义会话名称和页面 SHA-256。
`PUT /v1/tasks/conversations/{conversation_id}/title` 在当前认证 owner 的会话
命名空间内保存名称。接口不会返回 provider prompt、附件字节、工具参数、密钥
或完整 journal。浏览器存储只保存草稿与偏好，教学详情通过受保护的 task-debug
endpoint 重新加载。

首页和活动任务列表使用同一身份范围：管理员看到系统内全部排队/运行任务，
普通 key 看到本人跨会话的任务。因此，排队数量和最老运行时长始终对应当前
操作者可以查看的任务列表。

## 失败与隐私规则

- Runtime 决策消费机器错误 token，不解析 `text` 或 `error_text`。
- 隐藏推理、planner JSON、工具参数、密钥和原始 provider frame 不得成为公开
  呈现内容。
- Stream 缺口、offset 不匹配、摘要不匹配和 replacement 错误都返回结构化协议
  失败。
- 非 TTY、`NO_COLOR` 和 JSONL 模式禁用人类终端动画。
- Linux 与 macOS 共用可移植路径和终端 adapter；平台能力不可用时返回结构化
  unsupported 结果。
