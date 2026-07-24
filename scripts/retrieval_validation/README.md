# Retrieval Validation Scripts

用于验证 `kb ingest -> skill-owned retrieval index -> kb search` 这条链路。

## 脚本

- `run_kb_unified_index_e2e.py`
  - 创建隔离临时 workspace
  - 写入测试文档
  - 调用 `kb-skill` 执行 `ingest`
  - 查询 KB 私有数据库中的 `memory_retrieval_index`
  - 调用 `kb-skill` 执行 `search`
  - 打印详细过程与关键字段
  - 默认自动清理临时 workspace
- `run_knowledge_fact_recall_validation.py`
  - 调用专用 `clawd` 单元测试
  - 验证 `knowledge_fact -> semantic_fact -> RELEVANT_FACTS`
  - 打印插入行、召回结果、最终 memory context block
  - 使用内存 SQLite，无磁盘残留，测试结束即完成清理

## 用法

```bash
python3 scripts/retrieval_validation/run_kb_unified_index_e2e.py
```

```bash
python3 scripts/retrieval_validation/run_knowledge_fact_recall_validation.py
```

或直接一键启动：

```bash
sh scripts/retrieval_validation/run_kb_unified_index_e2e.sh
```

```bash
sh scripts/retrieval_validation/run_knowledge_fact_recall_validation.sh
```

保留临时目录便于排查：

```bash
python3 scripts/retrieval_validation/run_kb_unified_index_e2e.py --keep-temp
```

指定命名空间前缀：

```bash
python3 scripts/retrieval_validation/run_kb_unified_index_e2e.py --namespace-prefix demo
```
