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
- `run_rag_benchmark.py`
  - 使用 27 个固定 case 覆盖多语言、长 CJK 尾部、filter-before-limit、无答案、注入内容和相似干扰文档
  - 生成真实 PDF/DOCX/XLSX/PPTX fixture，并检查 page/section/sheet/slide 类型化引用
  - 在同一 namespace 验证双租户隔离，并执行 update -> stale removal -> resumable reindex -> delete 生命周期
  - 输出 Recall@K、MRR/nDCG、context/citation precision、no-answer accuracy、injection violations、p50/p95、峰值 RSS 与数据库增长
  - 按 schema v7 盘点 namespace/document/chunk/location/vector/ANN 行数和 embedding profile 分布
  - 大数据摄取按持久 continuation 运行，不扩大单次事务上限
  - 规模运行验证 ANN 行数、实际 ANN backend、精排上限和标准主机性能门禁
  - 不调用 live LLM 或远程 embedding provider，默认清理隔离工作区

## 用法

```bash
python3 scripts/retrieval_validation/run_kb_unified_index_e2e.py
```

```bash
python3 scripts/retrieval_validation/run_knowledge_fact_recall_validation.py
```

运行确定性 RAG 基准并保存机器报告：

```bash
python3 scripts/retrieval_validation/run_rag_benchmark.py \
  --output tmp/rag-benchmark-report.json
```

对现有 KB 私有数据库只读盘点：

```bash
python3 scripts/retrieval_validation/run_rag_benchmark.py \
  --inventory-only \
  --inventory-db data/skills/kb/state.db
```

10k/100k 规模门禁使用同一脚本的 `--synthetic-chunks` 参数。大规模运行只用于发布门禁，
不放入每次快速测试。规模不小于 10k 时，标准主机还必须满足 search p95 不超过 500 ms、
KB 技能子进程峰值 RSS 不超过 100 MiB：

```bash
python3 scripts/retrieval_validation/run_rag_benchmark.py --synthetic-chunks 10000
python3 scripts/retrieval_validation/run_rag_benchmark.py --synthetic-chunks 100000
```

报告的 `scale_gate` 会验证实际 chunk/vector 数不小于请求规模、每个 vector 恰有 8 行 ANN
projection、查询确实进入 ANN backend，并且 exact rerank 不超过 4096 行。`performance_gate`
单独记录延迟、RSS 和每 chunk 数据库字节数的实测值、阈值及通过状态；质量、引用、隔离、
生命周期、规模或性能任一门禁失败，脚本都返回非 0。快速基准仍会记录资源指标，但只有规模不小于
10k 时执行标准主机硬门槛，避免用小数据库固定开销误判增长率。

快速基准的 `cases` 会逐项输出命中路径、rank、runtime/process 耗时、候选阶段耗时、vector backend、
精排行数和引用校验计数。`lifecycle` 记录 ingest/reindex continuation 数与最终 active generation；
这些字段用于定位失败，不包含文档正文或凭据。

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
