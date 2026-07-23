# invest_copy Interface Spec

> Builtin skill: `invest_copy` → binary `invest-copy-skill`。经 `clawd` 调用时默认走内部文本 LLM 网关，使用系统当前 `[llm].selected_vendor` / `selected_model`；单独运行二进制时回退读取 `configs/config.toml` 的 `[llm]` 或 `OPENAI_*`。可选 `use_heuristic=true` **不调用模型**，只返回结构化摘要证据，由 finalizer/i18n/LLM 渲染自然语言。

## Capability Summary

- `draft`：**默认**用大模型按人物 Slug 撰写「数据摘要 + 风格化解读 + 局限 + 免责」；材料仍须来自用户 `data`（或由上游技能传入），技能本身**不**抓取 URL。
- **不负责**网页/HTTP；若需先拉正文，由 agent 先调 `http_basic` / `web_search_extract` / `browser_web` / `rss_fetch` / `doc_parse`，再把正文写入 `data`（或 `{{last_output}}`）。
- **非**该公众人物撰文或背书。**离线模式**（`use_heuristic`）无语义生成，不拼固定多语言模板，只返回 `summary_bullets`、`message_key`、`disclaimer_required` 等结构化字段。

## Collaboration flow（推荐编排）

1. `call_skill` 选择抓取/HTTP 类技能，取得页面或文档正文。
2. `call_skill invest_copy`，`args.data` 使用上一步输出（常写作 `{{last_output}}`），并设置 `args.person`。
3. 若需向用户说明来源，可设置 `source_note`（本技能不验证真伪）。

## Actions

| Action | Default | Description |
|--------|---------|-------------|
| `draft` | yes | 摘要 + 人物风格化解读（省略 `action` 即 `draft`） |
| `list_investors` | | 列出内置人物 slug / 别名 / 一句说明 |

## Parameter Contract

| Action | Param | Required | Type | Default | Description |
|--------|-------|----------|------|---------|-------------|
| draft | `data` | yes* | string | - | 待分析正文；可用 `material`、`user_data` 同义键。 |
| draft | `person` | yes | string | - | 人物 slug（如 `warren_buffett`）或别名（如 `巴菲特`）。 |
| draft | `brief` / `focus` | no | string | - | 用户希望侧重的角度；不替代 `data`。 |
| draft | `source_note` / `data_source` | no | string | - | 材料来源短注（网页摘录等）。 |
| draft | `channel` | no | string | `article` | `short`：更短摘要条数；`article`：较长。 |
| draft | `compliance` | no | string | `standard` | `light` 或 `standard`：免责段落长短。 |
| draft | `locale` / `language` / `lang` | no | string | - | `en`、`en-US` 时段落标题等为英文简述。 |
| draft | `use_heuristic` | no | bool | false | `true` 时不调用 LLM，返回离线规则摘要证据和机器 fallback（无密钥或未配置时使用）。 |
| draft | `action` | no | string | `draft` | 固定 `draft`。 |
| list_investors | `action` | yes | string | - | `list_investors` |
| all | — | — | — | — | 输入信封仍遵循技能协议：`request_id`、`args`、`context`、`user_id`、`chat_id`。 |

\* `data`/`material`/`user_data` 至少其一非空且长度 ≥ 10 字符（按 Unicode 字数）。

## Error Contract

- 未知 `person`、缺少 `data`、文本过短、或正文/侧重点触发配置化合规敏感词表 → `status=error`。
- `error_text` 使用 `code=...` 机器字段形式；运行时不得解析自然语言错误文本。
- 合规敏感词表位于 `configs/invest_copy.toml` / `docker/config/invest_copy.toml` 的 `invest_copy.compliance_sensitive_terms`，命中时 `extra` 返回 `reason_code=configured_compliance_term` 与 `term_index`，不在 Rust 里维护多语言短语数组。

## Success `extra`（`status=ok`）

- `draft`：`schema_version`、`source_skill`、`status`、`message_key=skill.invest_copy.draft_ready`、`action`、`person_slug`、`summary_mode`（`llm` \| `heuristic`）、`data_truncated`（bool）、`compliance`、`disclaimer_required`、`word_count`；`summary_mode=llm` 时含 `llm.credential_source`（`clawd_internal`|`env_openai`|`config_toml`）与 `llm.model`。
- **`summary_mode=heuristic`** 时另有 `summary_bullet_count`、`summary_bullets[]`、`brief`、`source_note`、`rendering.requires_language_rendering=true`。
- `list_investors`：`action`、`count`.

## Request/Response Examples

### Example 1：`draft` — 巴菲特风格

Request:

```json
{"request_id":"i1","args":{"action":"draft","data":"某基金2024年年报摘要：权益仓位约六成，管理费0.15%。策略偏宽基分散，风险提示部分提及利率与地缘政治不确定性。","person":"warren_buffett","source_note":"用户笔记摘录","channel":"article","compliance":"standard"}}
```

Response（节选）：

```json
{"request_id":"i1","status":"ok","text":"<model-rendered markdown>","extra":{"schema_version":1,"source_skill":"invest_copy","status":"ok","message_key":"skill.invest_copy.draft_ready","action":"draft","person_slug":"warren_buffett","summary_mode":"llm","data_truncated":false,"compliance":"standard","disclaimer_required":true,"word_count":1200},"error_text":null}
```

### Example 2：`list_investors`

Request:

```json
{"request_id":"i2","args":{"action":"list_investors"}}
```

Response（节选）：

```json
{"request_id":"i2","status":"ok","text":"…","extra":{"action":"list_investors","count":8},"error_text":null}
```

### Example 3：错误（数据过短）

Request:

```json
{"request_id":"i3","args":{"action":"draft","data":"太短了","person":"warren_buffett"}}
```

Response：

```json
{"request_id":"i3","status":"error","text":"","extra":{"schema_version":1,"source_skill":"invest_copy","status":"error","error_kind":"data_too_short","message_key":"skill.invest_copy.data_too_short","retryable":false,"current_chars":3,"min_chars":10},"error_text":"code=data_too_short current_chars=3 min_chars=10"}
```

## Config Entry Points

- **LLM**：经 `clawd` 调用时走内部文本 LLM 网关，默认使用系统 `[llm].selected_vendor` / `selected_model`。本技能当前没有独立模型覆盖项；如未来需要专用文案模型，应新增显式配置并保持默认注释。单独跑二进制且无内部网关环境变量时，尝试读取 **`WORKSPACE_ROOT`/当前目录向上的** `configs/config.toml` 中 `[llm.selected_vendor]` 与 `[llm.<vendor>]`（支持 `openai`、`minimax`、`mimo`、`deepseek`、`qwen`、`custom`、`grok` 等 OpenAI 兼容段；需在该段填写 **`api_key`**）。
- **人物**：`personas.toml` 编译进二进制，无运行时热加载。
- **合规词表**：`configs/invest_copy.toml` / `docker/config/invest_copy.toml` 的 `invest_copy.compliance_sensitive_terms`。

## Multilingual Reinforcement

### zh-CN

- 口语如「帮我按巴菲特口吻写」「用林奇的调子」仍需显式 **`person`**（或可被别名解析的中文名）；不提供 `person` → 报错。
### en

- Set `locale`/`language`/`lang` starting with `en` for English section headings where supported; persona one-liners may remain Chinese unless extended later.
