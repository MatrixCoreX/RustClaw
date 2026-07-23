# 代码索引合同

`code_index` 是 planner 面向代码仓库智能检索的边界。它在 `.rustclaw/index/repository-v1.json` 维护增量机器索引，并返回结构化定义、引用、测试、变更影响和有界源码区间。

## 所有权

- 模型决定何时需要仓库智能能力，并提供显式 `symbol`、`symbols`、`path` 或 `paths` 字段。
- Runtime 验证这些机器字段，把路径限制在 workspace 内，刷新索引并返回解析器生成的证据。
- Runtime 不得检查任务自然语言来推断 symbol、path、搜索模式或所需答案。
- 普通用户可见说明由模型基于通用 capability result 合成。

## Actions

| Action | 用途 |
| --- | --- |
| `refresh` | 增量刷新源码文件指纹和解析器数据。 |
| `search_symbols` | 通过显式 `exact`、`prefix` 或 `contains` 模式搜索符号名。 |
| `find_definitions` | 返回精确符号定义和源码区间 handle。 |
| `find_references` | 返回解析器观测到的精确符号引用。 |
| `list_tests` | 返回解析器观测到的测试，可按路径或被引用符号限制。 |
| `changed_impact` | 把显式或 Git 观测到的变更路径关联到依赖文件和测试。 |
| `retrieve_context` | 返回由结构化 symbol 或 path 选出的有界源码片段。 |

## 解析器与缓存边界

- Rust 文件使用 `syn` 解析，包括宏 token stream 内的标识符。
- 已识别的非 Rust 源文件会进入文件索引；在接入成熟解析器 adapter 前，不声称其符号结果准确。
- 未变化文件通过大小和纳秒级修改时间指纹复用；变化文件重新计算 SHA-256 并解析。
- 排除符号链接以及生成/缓存目录。索引路径始终为 workspace 相对路径。
- 每个源码位置都包含 `filesystem.read_text_range` 机器 handle。调用方应请求更窄的后续区间，不应加载大范围文件。

## 结果边界

所有 action 都返回包含以下字段的 JSON：

- `schema_version`
- `kind`
- `action`
- `status_code`
- 索引/刷新 `summary`
- action 专属 `data`

错误通过机器 `error_kind` 和结构化技能错误 envelope 表达。不得通过解析错误文本选择用户可见措辞。
