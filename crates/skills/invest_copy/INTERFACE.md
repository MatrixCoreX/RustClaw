# invest_copy Interface Spec

> Builtin skill: `invest_copy` → binary `invest-copy-skill`。默认通过 **OpenAI 兼容 `/chat/completions`** 调用**与 clawd 当前默认 `openai_compat` 提供商一致**的环境（`OPENAI_*`，由 `skill-runner` 注入）或回退读取 `configs/config.toml` 的 `[llm]`；可选 `use_heuristic=true` **不调用模型**，改用内置规则摘要。

## Capability Summary

- `draft`：**默认**用大模型按人物 Slug 撰写「数据摘要 + 风格化解读 + 局限 + 免责」；材料仍须来自用户 `data`（或由上游技能传入），技能本身**不**抓取 URL。
- **不负责**网页/HTTP；若需先拉正文，由 agent 先调 `http_basic` / `web_search_extract` / `browser_web` / `rss_fetch` / `doc_parse`，再把正文写入 `data`（或 `{{last_output}}`）。
- **非**该公众人物撰文或背书。**离线模式**（`use_heuristic`）无语义生成，仅规则选句+模板。

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
| draft | `use_heuristic` | no | bool | false | `true` 时不调用 LLM，使用离线规则摘要+模板（无密钥或未配置时使用）。 |
| draft | `action` | no | string | `draft` | 固定 `draft`。 |
| list_investors | `action` | yes | string | - | `list_investors` |
| all | — | — | — | — | 输入信封仍遵循技能协议：`request_id`、`args`、`context`、`user_id`、`chat_id`。 |

\* `data`/`material`/`user_data` 至少其一非空且长度 ≥ 10 字符（按 Unicode 字数）。

## Error Contract

- 未知 `person`、缺少 `data`、文本过短、或正文/侧重点触发「喊单/保本保收益」敏感表述 → `status=error`，`error_text` 为可读中文说明。

## Success `extra`（`status=ok`）

- `draft`：`action`、`person_slug`、`summary_mode`（`llm` \| `heuristic`）、`data_truncated`（bool）、`compliance`、`word_count`；`summary_mode=llm` 时含 `llm.credential_source`（`env_openai`|`config_toml`）与 `llm.model`。
- **`summary_mode=heuristic`** 时另有 `summary_bullet_count`（number）。
- `list_investors`：`action`、`count`.

## Request/Response Examples

### Example 1：`draft` — 巴菲特风格

Request:

```json
{"request_id":"i1","args":{"action":"draft","data":"某基金2024年年报摘要：权益仓位约六成，管理费0.15%。策略偏宽基分散，风险提示部分提及利率与地缘政治不确定性。","person":"warren_buffett","source_note":"用户笔记摘录","channel":"article","compliance":"standard"}}
```

Response（节选）：

```json
{"request_id":"i1","status":"ok","text":"…","extra":{"action":"draft","person_slug":"warren_buffett","data_truncated":false,"summary_bullet_count":3,"compliance":"standard","word_count":1200},"error_text":null}
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
{"request_id":"i3","status":"error","text":"","extra":null,"error_text":"args.data/material 有效长度过短…"}
```

## Config Entry Points

- **LLM**：与主程序一致——经 `skill-runner` 时优先使用注入的 `OPENAI_BASE_URL` / `OPENAI_MODEL` / `OPENAI_API_KEY`（对应运行中第一个 `openai_compat` provider）。单独跑二进制且无环境变量时，尝试读取 **`WORKSPACE_ROOT`/当前目录向上的** `configs/config.toml` 中 `[llm.selected_vendor]` 与 `[llm.<vendor>]`（支持 `openai`、`minimax`、`mimo`、`deepseek`、`qwen`、`custom`、`grok` 等 OpenAI 兼容段；需在该段填写 **`api_key`**）。
- **人物**：`personas.toml` 编译进二进制，无运行时热加载。

## Multilingual Reinforcement

### zh-CN

- 口语如「帮我按巴菲特口吻写」「用林奇的调子」仍需显式 **`person`**（或可被别名解析的中文名）；不提供 `person` → 报错。
### en

- Set `locale`/`language`/`lang` starting with `en` for English section headings where supported; persona one-liners may remain Chinese unless extended later.
