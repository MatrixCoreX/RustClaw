# transform 功能增强计划（仅 skill 侧，不改 clawd）

## 目标
把 transform 从基础数组处理提升到实用的数据整理引擎。

## 当前现状
- 已支持 ilter/sort/dedup/project/group
- 已支持 json/md_table/csv
- 缺少嵌套字段、聚合与类型策略

## 阶段拆分

### 阶段 A：字段能力
1. 嵌套路径
- 支持 .b.c 读取

2. 类型比较规则
- 数字/字符串/布尔比较一致化

### 阶段 B：聚合能力
1. group + aggregate
- count/sum/avg/min/max

2. project 别名（可选）
- 明确是否支持 rename，若支持需显式字段映射

### 阶段 C：输出质量
1. md_table/csv 稳定序
- 字段顺序可控

2. 统计增强
- stats.warnings
- stats.skipped_records

## 接口增强建议
- 新增 strict=true/false
- 新增 
ull_policy
- 新增 ggregations[]

## 验收标准
- 复杂 JSON 数组可稳定处理
- 不支持场景明确报错
- 输出统计准确可解释

## 风险与缓解
- 风险：规则变复杂后可预测性下降
- 缓解：严格模式默认 + 明确错误文案

## 交付清单
- main.rs 路径与聚合支持
- INTERFACE.md op 语义补齐
- prompt 场景示例更新
