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
- 大 Rust 文件兜底在同一描述符中单遍计算完整摘要；单行最多保留 24 KiB，
  符号名合计不超过 2 MiB，候选最多 50,000 个。超长或非法 UTF-8 行整体跳过，
  保留后续准确行号，以 `range_index_budget_exceeded` 明示部分结果，不把截断行
  前缀当成完整符号。
- 未变化文件通过大小和纳秒级修改时间指纹复用；变化文件重新计算 SHA-256 并解析。
  每次查询在复用前，都通过当前宿主文件策略重新准入全部保留的缓存路径，包括本次
  增量扫描页以外的条目。历史解析数据不代表当前仍有权限读取源码。
- 排除符号链接以及生成/缓存目录。索引路径始终为 workspace 相对路径；Linux/macOS
  通过目录描述符和 no-follow 操作读取源码/缓存及原子发布缓存。
- 受保护目录会被排除，公共代码查询仍可使用。`summary.policy_excluded_entries`、
  `cache_rejected_files` 和 `skipped_files` 明示排除或拒绝检查次数，不代表去重后的
  源文件数量。扫描中存在排除或不可读条目时，`scan_complete=false`；这类部分结果
  不能证明全仓没有某个符号。显式请求的受保护源码和缓存会被拒绝，不会通过旧缓存
  绕过。只读查询不改写缓存文件；刷新通过同一个已准入父目录发布缓存。
- 解析器版本变化会使旧缓存失效，不改变公开 schema。缓存读取限制为 128 MiB；
  只读查询不会写入重建结果，仅显式 `refresh` 可以持久化缓存。
- 每个源码位置都包含 `filesystem.read_text_range` 机器 handle。调用方应请求更窄的后续区间，不应加载大范围文件。
- 自动查询 Git 变更时，进程只读、禁网，使用同一宿主私有目录排除策略和共享显式环境。
  查询使用临时仓库、固定 HEAD 和有大小限制的索引/共享索引、忽略/属性数据副本，
  只投影经过校验的声明式 core 设置，不加载仓库/全局的可执行配置、clean/process
  过滤器、hook、fsmonitor、外部 diff 或文本转换。对象库保持只读，包括 linked worktree。
  `changed_paths_semantics=without_executable_repository_config` 表示保守比较：
  原始文件即使没有用户修改，也可能与过滤器归一化后的 blob 不同。
  NUL 分隔路径保留换行、空白和字面反斜杠；无法表示的非 UTF-8 路径明示部分结果，
  不用有损替换生成另一条路径。Git 缓存中的受保护路径也会过滤；`data.policy_excluded_paths`、
  `git_query_failures`、`changed_paths_complete` 区分部分发现与已验证的空变更集。
  Git 本身不启动嵌套状态命令；已初始化子模块使用同一隔离查询，每次最多 32 个仓库、
  8 层嵌套及 128 MiB 元数据副本。索引/共享索引流式复制，每个文件最多 64 MiB；
  忽略/属性文件每个最多 1 MiB。保留 gitlink HEAD 变化及已检查子模块内的文件变化。
  未初始化、循环引用或超出预算的子模块明确计入
  `git_submodule_worktrees_uninspected`；该值大于 0 时 `changed_paths_complete=false`。
  沙箱不可用时不会退回直接执行。这一准备边界不等同于完整进程归属、取消和生命周期
  准入，也不会启用权威存储。

## 结果边界

所有 action 都返回包含以下字段的 JSON：

- `schema_version`
- `kind`
- `action`
- `status_code`
- 索引/刷新 `summary`
- action 专属 `data`

错误通过机器 `error_code` 和结构化技能错误 envelope 表达。不得通过解析错误文本选择用户可见措辞。
