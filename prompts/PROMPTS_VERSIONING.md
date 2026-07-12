# Prompt 版本号约定（§3.5a）

> 该文档约束 `prompts/` 下 prompt 文件如何声明版本号，使每次 LLM 调用日志可携带
> `prompt_version=<id>` 字段，便于在 `model_io.log` / `task_journal` 中索引、对照、回滚。

## 1. 适用范围

* **必须**带版本号的 prompt（核心审计基线）：
  * `intent_normalizer_prompt.md`
  * `chat_response_prompt.md`
  * `single_plan_execution_prompt.md`
  * `lightweight_execution_prompt.md`
  * `loop_incremental_plan_prompt.md`
  * `plan_repair_prompt.md`
  * `planner_abort_compact_retry_prompt.md`
  * `delivery_text_classifier_prompt.md`
  * `observed_answer_fallback_prompt.md`
* **建议**带版本号的 prompt：
  * 其他 overlay prompt、skill prompt、reference prompt、persona prompt——
    任何修改频次>0 / 输出契约稳定的都建议加。
* **可不**带版本号的 prompt：
  * 极简 / 一次性 / vendor patch 中的小补丁。
  * 没有声明的 prompt 在日志中记 `prompt_version=none`，行为不受影响。

## 2. 声明语法

支持两种形式（按优先级，前者命中则后者忽略）：

### 2.1 HTML 注释（推荐）

最常见，因为 prompt 文件多半已有 `<!-- Purpose / Component / ... -->` 元数据块：

```markdown
<!--
Purpose: ...
Component: ...
Version: 2026-04-17.1
Placeholders: ...
-->
```

也允许单行写法：

```markdown
<!-- version: 2026-04-17.1 -->
```

### 2.2 YAML frontmatter

```markdown
---
title: foo
version: 2026-04-17.1
---

prompt body...
```

## 3. 版本号格式

版本号必须满足：

* 字符集：`[A-Za-z0-9._\-+]`，长度 ≤ 64。
* 不允许空格、中文、`/` 等其他字符（提取器会拒绝并视为未声明）。
* 大小写敏感（`v1.0.0` 与 `V1.0.0` 是不同版本）。

**推荐格式**：`YYYY-MM-DD.N`（基线日期 + 当日序号），例如 `2026-04-17.1`、`2026-04-17.2`、`2026-04-18.1`。

> 选 ISO 风格而非 SemVer 是因为 prompt 不是库 API，不需要严格的 MAJOR/MINOR/PATCH 语义；
> 关键属性是"按时间线性增加、可双向定位"。

## 4. 何时升版本号

* prompt 正文出现**功能/契约/行为**变化时——必须升。
* 仅修缩进、错别字、注释——可不升（但加 `.NN` 序号也可）。
* layered prompt 由多 part 拼接：取**第一个**有 version 的 part 作为整体版本号
  （base → vendor patch → overlay 顺序）。如果改的是 vendor patch，建议**只**升 patch
  自己的版本号，避免触发 base 全员升版的雪崩。

## 5. 提取实现

* 模块：`claw_core::prompt_layers::extract_prompt_version(text: &str) -> Option<String>`
* 加载入口：`claw_core::prompt_layers::load_prompt_template_for_vendor_with_meta(...)`
  返回 `ResolvedPromptTemplate { template, source, version }`。
* clawd wrapper：`bootstrap::prompts::load_prompt_template_for_state_with_meta(...)`。
* 日志入口：`prompt_utils::log_prompt_render_with_version(...)`，输出
  `prompt_version=<id>` 字段；老调用 `log_prompt_render(...)` 不带版本，自动填 `none`。
* 解析只扫前 80 行，避免 prompt 正文里碰巧出现 `version:` 字样误识别。

## 6. 单测

`crates/claw-core/src/prompt_layers.rs` 内单测覆盖：

* HTML 注释 / YAML frontmatter / 多行 metadata 块的各种组合。
* 非法字符、超长、第 80+ 行后才出现等边界。
* 与 `load_prompt_template_for_vendor_with_meta` 的集成（disk / default fallback / layered first-wins）。

## 7. 接入历史

* §3.5a (2026-04-17)：建立基线，给核心审计 prompt 加 `Version: 2026-04-17.1`，
  调用面同步迁移到 with_meta API。后续若某 prompt 退出主链，应及时从“核心审计基线”
  中移除，避免继续把 legacy overlay 当成活跃 contract 维护。
