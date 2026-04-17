# kb 功能增强计划（仅 skill 侧，不改 clawd）

## 目标
把 kb 从轻量占位索引提升到可用的本地知识检索层。

## 当前现状
- 已有 ingest/search
- 当前检索偏轻量
- 缺少 metadata 过滤与排序质量

## 阶段拆分

### 阶段 A：索引质量
1. 索引结构规范化
- 文档级 metadata（path/type/mtime）
- chunk 级 metadata（chunk_id/offset）

2. ingest 策略
- 增量更新
- overwrite 精准重建

### 阶段 B：检索质量
1. 关键词检索增强
- 至少 BM25 或倒排评分

2. 过滤能力
- path_prefix
- ile_type
- 时间范围（可选）

### 阶段 C：结果可解释
1. 命中解释
- 命中词片段高亮（或 hit_terms）

2. 返回结构增强
- score_reason
- metadata

## 接口增强建议
- ingest 新增 ile_types / max_file_size
- search 新增 ilters / min_score

## 验收标准
- namespace 内检索准确率明显提升
- 命中结果可追溯到来源
- overwrite 与增量行为一致可预测

## 风险与缓解
- 风险：索引文件膨胀
- 缓解：定期压缩/重建策略

## 交付清单
- main.rs 检索评分升级
- INTERFACE.md filters 与 metadata
- prompt 指引 ingest/search 用法
