# 代码索引合同

`code_index` 是 planner 面向代码仓库智能检索的边界。它在 `.agent-runtime/index/repository-v1.json` 维护增量机器索引，并返回结构化定义、引用、测试、变更影响和有界源码区间。

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
| `find_references` | 返回名称精确匹配、带词法作用域的语法引用候选。 |
| `list_tests` | 返回解析器观测到的测试，可按路径或被引用符号限制。 |
| `changed_impact` | 把显式或 Git 观测到的变更路径关联到依赖文件和测试。 |
| `retrieve_context` | 返回由结构化 symbol 或 path 选出的有界源码片段。 |

## 解析器与缓存边界

- Rust 文件使用 `syn` 解析，包括宏 token stream 内的标识符。
- Python、JavaScript/JSX、TypeScript/TSX 使用固定版本的上游 Tree-sitter 语法。
  可索引具名函数、类、方法、变量、导入别名和参数，TypeScript 还包括接口和类型；
  可发现 Python 测试函数和 JavaScript 测试调用声明。
- 引用包含词法 `scope` 和 `resolution=syntactic_candidate`。同名引用和变更影响
  都是语法候选，不声称已完成语言服务器级别的跨文件类型解析。
- 其他已识别语言保留文件级索引。语法错误标记 `parsed_with_errors`；解析或遍历
  预算耗尽会明确标记为部分结果。索引不会导入或执行用户代码。
- `summary.complete` 仅表示源码文件扫描是否完成，作用域由
  `completeness_scope=source_file_scan` 明示；`symbol_coverage_complete` 单独说明是否
  每个文件都有完整解析器覆盖。部分索引里查不到候选，不等于符号不存在。
- 每个源码的 AST 输入上限为 2 MiB；更大的非 Rust 文件分块计算摘要，不构建语法树；
  大型 Rust 文件保留明确的区间索引兜底。Tree-sitter 单文件解析和遍历预算为 250ms、
  最多 50,000 条索引记录，遗漏部分仍可通过源码区间工具查看。
- 未变化文件通过大小和纳秒级修改时间指纹复用；变化文件重新计算 SHA-256 并解析。
- 排除符号链接以及生成/缓存目录。索引路径始终为 workspace 相对路径；Linux/macOS
  通过目录描述符和 no-follow 操作读取源码/缓存及原子发布缓存。
- 解析器版本变化会使旧缓存失效，不改变公开 schema。缓存读取限制为 128 MiB；
  只读查询不会写入重建结果，仅显式 `refresh` 可以持久化缓存。
- 每个源码位置都包含 `filesystem.read_text_range` 机器 handle。调用方应请求更窄的后续区间，不应加载大范围文件。

## 结果边界

所有 action 都返回包含以下字段的 JSON：

- `schema_version`
- `kind`
- `action`
- `status_code`
- 索引/刷新 `summary`
- action 专属 `data`

错误通过机器 `error_code` 和结构化技能错误 envelope 表达。不得通过解析错误文本选择用户可见措辞。
