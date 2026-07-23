# 技能、多媒体与模型

上一页：[编码与可观测性](04-coding-observability.zh-CN.md) |
[架构索引](README.md) |
下一页：[发布验证](06-release-validation.zh-CN.md)

Registry 是技能可用状态、capability、effect、risk、schema、安装模式和 runner
元数据的机器事实源。自然语言短语不得进入 alias 或 runtime 派发分支。

```mermaid
flowchart TD
    A{任务来源} -->|ask| B[Planner call_capability]
    A -->|run_skill| C[显式 skill_name]
    B --> D[CapabilityResolver]
    C --> E[规范化机器 token alias]
    D --> E
    E --> F[Skills registry<br/>enabled + kind + runner + policy]
    F --> G{实现类型}
    G -->|builtin| H[进程内 adapter]
    G -->|runner| I[skill-runner 子进程]
    G -->|external| J[External adapter]
    I --> K[技能二进制<br/>单行 JSON stdin/stdout]
    H --> L[结构化技能响应]
    J --> L
    K --> L
    L --> M[CapabilityResultEnvelope<br/>状态码 + 证据 + 产物]
```

长尾多媒体能力使用 start、poll、cancel 合同。Provider 工作继续运行时，前台任务
可以先返回 checkpoint。

```mermaid
flowchart TD
    A[图片 / 语音 / 视频 / 音乐 capability] --> B[Registry async contract]
    B --> C[Verifier + provider preflight]
    C --> D[启动 provider job]
    D --> E{Provider 结果}
    E -->|完成| F[Artifact refs + observation]
    E -->|进行中| G[pending_async_job<br/>job_id + poll_ref]
    G --> H[Checkpoint<br/>next_check_after + can_poll + can_cancel]
    H --> I[Worker recovery 或显式 poll]
    I --> J[Poll adapter]
    J -->|进行中| G
    J -->|完成| F
    J -->|失败或不可用| K[结构化等待 / 修复 / 终态]
    H --> L[Cancel capability]
    L --> M[Cancel adapter + terminal projection]
```

模型能力通过 catalog 投影，不能根据模型名称短语猜测。文本规划、图片/视频理解、
生成、流式、工具调用、上下文长度、凭据、异步和 dry-run 都是明确机器字段，供
UI、CLI 和 runtime readiness 检查使用。

```mermaid
flowchart LR
    A[Provider 配置] --> D[ModelCatalog builder]
    B[多媒体配置] --> D
    C[Vendor capability patches] --> D
    D --> E[Catalog entries<br/>provider + model + modality flags]
    E --> F[Runtime readiness decision]
    E --> G[GET /v1/models/catalog]
    E --> H[clawcli models catalog/readiness]
    G --> I[UI 模型配置]
    F --> J[Planner/provider call trace]
```
