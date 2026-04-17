# web_search_extract 功能增强计划（仅 skill 侧，不改 clawd）

## 目标
保留其搜索层定位，接入真实搜索后端，避免与 browser_web 职责混淆。

## 当前现状
- 结构已完整
- 当前多为占位返回
- 你主链路偏 browser_web，因此它应做轻量搜索入口

## 阶段拆分

### 阶段 A：后端接入
1. 抽象 backend adapter
- provider 可切换
- 统一返回字段

2. 基础搜索
- query/top_k/lang/time_range/domains_allow/domains_deny

### 阶段 B：结果质量
1. 去重与归一
- URL 归一化
- source 标准化

2. 轻量摘要
- summary 仅基于结果，不编造正文

### 阶段 C：与 browser_web 协同
1. search-only 明确边界
- 不承担重浏览器抓取

2. 可选衔接字段
- 输出可直接给 browser_web.extract 的 URL 列表

## 接口增强建议
- 新增 ackend 参数
- 新增 include_snippet=true/false

## 验收标准
- 搜索成功时结果字段稳定
- 无后端时错误提示明确
- 与 browser_web 分工清晰

## 风险与缓解
- 风险：供应商限流
- 缓解：重试、退避、配额错误码透传

## 交付清单
- main.rs backend adapter
- INTERFACE.md 后端能力说明
- prompt 边界规则强化
