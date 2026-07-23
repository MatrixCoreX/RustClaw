# 发布验证

上一页：[技能、多媒体与模型](05-skills-media-models.zh-CN.md) |
[架构索引](README.md)

发布验证由确定性架构合同、聚焦组件测试、UI 检查和精简 NL 验收组成。每个 gate
都写入机器可读证据，防止汇总显示通过但内部检查被跳过或产物格式损坏。

```mermaid
flowchart TD
    A[源码修改] --> B{受影响边界}
    B --> C[聚焦 Rust / UI / script 测试]
    B --> D[架构合同 self-test]
    B --> E[Registry、prompt、policy、<br/>多语言与长文件检查]
    C --> F[Agent parity gate]
    D --> F
    E --> F
    F --> G[Artifact contract 校验<br/>内容 + path refs + nested summaries]
    G --> H[精简 NL 验收<br/>覆盖 capability 与失败类别]
    H --> I{发布证据是否完整}
    I -->|否| J[结构化 finding<br/>修复并重跑受影响范围]
    I -->|是| K[Release candidate]
```

主要合同类别包括：

- planner/runtime 边界、已删除的 pre-route 兼容路径和仅限 loop 内的 repair；
- policy decision、授权、registry effect、幂等性和副作用 reconciliation；
- 任务生命周期、checkpoint/resume、事件归档回放、上下文、编码和 subagent；
- 生成式技能 prompt、registry parity、alias、异步多媒体合同和模型 readiness；
- 禁止自然语言硬匹配、禁止固定多语言 runtime 回复、密钥扫描、跨平台与长文件限制；
- CLI exec/replay/session/goal/TUI/LLM trace 产物及 UI lint/build/test。

Live provider 测试是验收证据，不能把失败句子编码成 runtime 分支。失败应在
capability contract、registry metadata、prompt、verifier、adapter 或 provider
边界修复。
