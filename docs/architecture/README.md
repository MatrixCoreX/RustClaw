# Agent Runtime Architecture Guide

GitHub does not provide true pagination inside a README. Agent Runtime therefore
keeps only the primary agent-loop overview in the repository README and
organizes the detailed diagrams as the ordered pages below.

GitHub 的 README 不支持真正的页内分页。Agent Runtime 因此只在仓库 README 保留主 Agent Loop 总览图，并把详细流程按以下顺序拆成独立页面。

| Page | English | 中文 |
| --- | --- | --- |
| 1 | [Agent loop and planning](01-agent-loop.md) | [Agent Loop 与规划](01-agent-loop.zh-CN.md) |
| 2 | [Security and execution](02-security-execution.md) | [安全与执行](02-security-execution.zh-CN.md) |
| 3 | [Task state and context](03-task-state-context.md) | [任务状态与上下文](03-task-state-context.zh-CN.md) |
| 4 | [Coding and observability](04-coding-observability.md) | [编码与可观测性](04-coding-observability.zh-CN.md) |
| 5 | [Skills, media, and models](05-skills-media-models.md) | [技能、多媒体与模型](05-skills-media-models.zh-CN.md) |
| 6 | [Release validation](06-release-validation.md) | [发布验证](06-release-validation.zh-CN.md) |
| 7 | [Office artifact workspace](07-office-artifacts.md) | [Office 工件工作区](07-office-artifacts.zh-CN.md) |
| 8 | [Skill-owned storage](08-skill-owned-storage.md) | [技能独立存储](08-skill-owned-storage.zh-CN.md) |
| 9 | [Interactive coding and presentation](09-interactive-coding.md) | [交互式编码与输出呈现](09-interactive-coding.zh-CN.md) |
| 10 | [Web entry and core isolation](10-web-entry-security.md) | [Web 入口与核心隔离](10-web-entry-security.zh-CN.md) |
| 11 | [Task artifact delivery](11-task-artifact-delivery.md) | [任务产物交付](11-task-artifact-delivery.zh-CN.md) |
| 12 | [Browser media discovery](12-media-discovery.md) | [浏览器媒体发现](12-media-discovery.zh-CN.md) |
| 13 | [NNI capability and heartbeat control](13-nni-capability.md) | [NNI 能力与心跳控制](13-nni-capability.zh-CN.md) |

These files are also the source documents rendered by the UI's Learning / Maintenance
page. Edit a diagram here instead of copying it into UI source.

这些文件同时是 UI“学习/维护”页面的内容源。流程图应直接在这里修改，不要复制到 UI 源码中维护第二份内容。

Architecture pages describe the current implementation only: current owners,
current request flow, current machine contracts, and current validation.
Migration history and retired behavior belong in Git history or archived local
plans, not in these pages or the Learning / Maintenance UI.

架构页面只描述当前实现：当前责任方、当前请求流程、当前机器合同和当前验收方式。
迁移历史与停用行为由 Git 历史或本地归档计划保存，不进入这些页面，也不进入
UI“学习/维护”模块。
