# stock Interface Spec

> 本技能用于查询 A 股实时行情，数据来源新浪财经。

## Capability Summary
- 查询 A 股（沪/深）实时行情：现价、今开、昨收、涨跌幅、成交量等。
- 支持股票代码查询，也支持通过配置的公司名/简称/别名查询后再取行情。
- 仅读、不涉及交易或下单。

## Actions
- `quote`（默认）/ `query`：按股票代码，或按已配置的公司名/别名，查询单只 A 股行情。

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| quote / query | `symbol` 或 `code` 或 `name` | 是 | string | - | 股票代码，或 `configs/stock.toml` 中配置的公司名/简称/别名，如 600519、000001、sh600519、sz000001、中国移动、茅台 |
| quote / query | `action` | 否 | string | "quote" | 固定为 quote 或 query |

## Error Contract
- 缺少 symbol/code 时返回明确提示。
- 接口失败或响应格式异常时返回 status=error 与可读 error_text。
- 无效代码或非 A 股时返回「未获取到行情」类提示。
- 名称未命中映射时返回明确提示，并建议补充 `configs/stock.toml`。

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
{"request_id":"demo-4","status":"error","text":"","error_text":"args.symbol 或 args.code 或 args.name 必填，例如 600519、000001、sh600519、sz000001、中国移动"}
```
