# stock Interface Spec

> 本技能用于查询 A 股和美股实时行情；新浪财经为主行情源，腾讯财经为自动回退源。

## Capability Summary
- 查询 A 股（沪/深）和美股实时行情：现价、今开、昨收、涨跌幅、成交量等。
- 支持 A 股代码、美股 ticker、公司名和简称。未配置的名称先通过证券搜索服务解析，
  `configs/stock.toml` 别名及 LLM 纠错仅作为降级路径。
- 新浪行情不可用或响应合同不完整时，自动回退到腾讯行情；结果通过 `provider` 明确来源。
- 支持 provider-free 报价请求预览，只规范化代码或标记名称解析方式，不调用行情接口或 LLM。
- 仅读、不涉及交易或下单。

## Actions
- `preview_quote`：离线检查报价请求并返回规范化结果；不请求新浪财经或 LLM。
- `quote`（默认）/ `query`：按代码、ticker 或公司名称查询单只 A 股/美股行情。

成功的 `quote` / `query` 在 `extra` 中稳定提供 `normalized_code`、`market`、`name`、
`price`、`currency`、`provider` 和带 `+08:00` 时区的 `observed_at`；原始
`current`、`date`、`time` 字段继续保留用于兼容。

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| quote / query | `symbol` 或 `code` 或 `name` | 是 | string | - | A 股代码、美股 ticker 或公司名称，如 600519、sh600519、TSLA、特斯拉、园林股份。 |
| quote / query | `action` | 否 | string | "quote" | 固定为 quote 或 query |
| quote / query | `market` | 否 | enum | `auto` | `auto`、`cn` 或 `us`；名称/代码存在歧义时显式指定。 |
| preview_quote | `symbol` 或 `code` 或 `name` | 是 | string | - | 待检查的股票代码或名称。 |
| preview_quote | `action` | 是 | string | - | 固定为 `preview_quote`。 |
| preview_quote | `market` | 否 | enum | `auto` | 仅离线约束预览，不访问行情或名称搜索服务。 |

- Preview `extra`: `action=preview_quote`, `requested_symbol`, `normalized_code`, `market`, `resolution_mode`, `provider_candidates`, `would_execute=false`, and `external_call_count=0`; no quote, symbol-search, or model provider is contacted.

## Error Contract
- 缺少 symbol/code 时返回明确提示。
- 行情主源和回退源均失败时返回 `quote_provider_chain_failed`，并标记为可重试。
- 动态名称搜索不可用时返回 `symbol_search_unavailable`；确实没有候选时返回
  `symbol_not_found`，不得统一折叠成 `stock_quote_failed`。
- 内部 LLM 返回空内容只会放弃该降级候选，不会覆盖动态搜索结果或直接终止查询。
- 错误 `extra` 包含稳定的 `error_code`、`message_key`、`retryable`；请求中存在
  `symbol` / `code` / `name` 时还会原样提供 `requested_symbol`，运行时无需解析
  `error_text`。

## Config Entry Points
- `configs/stock.toml` controls fallback aliases, name-normalization cleanup metadata, and LLM typo correction.
- Text LLM fallback uses the system default `[llm].selected_vendor` / `selected_model` by default.
- `stock.llm_vendor` / `stock.llm_model` are optional dedicated overrides and should stay commented unless stock name correction needs a separate text model.

## Request/Response Examples

### Example 1：查询贵州茅台
Request:
```json
{"request_id":"demo-1","args":{"symbol":"600519"}}
```
Response:
```json
{"request_id":"demo-1","status":"ok","text":"【SH600519】贵州茅台\n现价 1688.00  今开 1680.00  昨收 1675.00\n涨跌幅 +0.78%\n...","error_text":null}
```

### Example 2：使用 code 与 action
Request:
```json
{"request_id":"demo-2","args":{"action":"quote","code":"000001"}}
```
Response:
```json
{"request_id":"demo-2","status":"ok","text":"【SZ000001】平安银行\n...","error_text":null}
```

### Example 3：使用公司名
Request:
```json
{"request_id":"demo-3","args":{"name":"中国移动"}}
```
Response:
```json
{"request_id":"demo-3","status":"ok","text":"已按“中国移动”匹配查询。\n【SH600941】中国移动\n...","error_text":null}
```

### Example 4：缺少参数
Request:
```json
{"request_id":"demo-4","args":{}}
```
Response:
```json
{"request_id":"demo-4","status":"error","text":"","error_text":"code=missing_symbol"}
```

### Example 5：查询美股 ticker
Request:
```json
{"request_id":"demo-5","args":{"action":"quote","symbol":"TSLA","market":"us"}}
```
Response `extra` includes `normalized_code=US:TSLA`, `market=us`, `currency=USD`, the observed
quote fields, and the selected provider.
