# weather Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the weather implementation.

## Capability Summary
- `weather` 查询指定城市或经纬度的当前天气。
- 使用 Open-Meteo 地理编码与天气预报接口，无需 API Key。

## Actions
- `query`（默认）：根据城市名或经纬度查询当前天气。

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `city` 或 `location` 或 `place` 或 `q` | 否* | string | - | 城市/地名，如 北京、Beijing、上海。与经纬度二选一。 |
| all | `latitude` + `longitude` | 否* | number | - | 纬度、经度。与 city 二选一。 |
| all | `action` | no | string | `query` | 固定为 query（可省略）。 |

\* 必须提供「城市/地名」或「latitude + longitude」其一。

## Error Contract
- 未提供 city/location/place/q 且未同时提供 latitude、longitude。
- 地理编码未找到该地点时返回“未找到该地点，请换一个城市或地名”。
- 请求超时或接口非 2xx 时返回可读错误信息。

## Request/Response Examples
### Example 1：按城市查询
Request:
```json
{"request_id":"w1","args":{"city":"北京"}}
```
Response:
```json
{"request_id":"w1","status":"ok","text":"Beijing, Beijing, China 白天：晴，气温 12.8°C，风速 13.7 km/h，风向 300°。","error_text":null}
```

### Example 2：按经纬度查询
Request:
```json
{"request_id":"w2","args":{"latitude":39.9,"longitude":116.4}}
```
Response:
```json
{"request_id":"w2","status":"ok","text":"39.90°N, 116.40°E 白天：晴，气温 12.8°C，风速 13.7 km/h，风向 300°。","error_text":null}
```

### Example 3：错误（缺少参数）
Request:
```json
{"request_id":"w3","args":{}}
```
Response:
```json
{"request_id":"w3","status":"error","text":"","error_text":"请提供城市名（city/location/place/q）或经纬度（latitude + longitude）"}
```
