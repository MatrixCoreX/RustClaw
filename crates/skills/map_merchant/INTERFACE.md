# map_merchant Interface Spec

> This file is managed by `scripts/sync_skill_docs.py`.
> Keep this spec aligned with the map_merchant implementation.

## Capability Summary
- `map_merchant` 是一个多地图商户推荐技能，当前支持 `amap` 与 `google` 两个 provider。
- 默认 provider 从 `configs/map_merchant.toml` 的 `[map_merchant].default_provider` 读取；当前建议默认值为 `amap`。
- 技能支持按“当前位置/经纬度”或“城市/地址/商圈关键词”推荐附近商户。
- 技能支持偏好型筛选，可结合 `keyword`、`category`、`cuisine`、`price_level`、`max_distance_meters`、`sort_by` 做排序。
- 成功响应会返回可读推荐文本、结构化候选列表，以及可供通信端转换为按钮的导航链接行。

## Actions
- `recommend`（默认）：根据坐标或地点描述推荐附近商户。

## Parameter Contract
| Action | Param | Required | Type | Default | Description |
|---|---|---|---|---|---|
| all | `action` | no | string | `recommend` | 当前仅支持 `recommend`。 |
| all | `provider` | 否 | string | 配置默认值 | 支持 `amap` / `google`。 |
| all | `latitude` + `longitude` | 否* | number | - | 当前位置坐标。与地点描述方式二选一。 |
| all | `city` / `district` / `address` / `place` / `location` / `q` | 否* | string | - | 用于确定推荐中心点；可只给城市，也可给更具体的地址或商圈。 |
| all | `keyword` | 否 | string | `configs/map_merchant.toml` 中的 `default_keyword` | 用户想找的商户关键词，如“咖啡”“火锅”“亲子餐厅”。 |
| all | `category` | 否 | string | - | 商户大类偏好，如“餐饮”“咖啡店”“便利店”。 |
| all | `cuisine` | 否 | string | - | 菜系或细分类偏好，如“川菜”“粤菜”“手冲”。 |
| all | `price_level` | 否 | string/number | `any` | 价格偏好。支持 `cheap` / `mid` / `premium`，也支持数字 `1/2/3/4`。 |
| all | `max_distance_meters` 或 `radius` | 否 | number | 见配置 | 最大搜索半径（米），范围会被钳制在 500 到 50000。 |
| all | `sort_by` | 否 | string | `balanced` | 支持 `balanced` / `distance` / `rating` / `price`。 |
| all | `top_k` 或 `topK` | 否 | number | 见配置 | 最多返回多少条推荐，当前实现上限为 10。 |

\* 必须提供「`latitude` + `longitude`」或「地点描述字段」其中一种。

## Success `extra`（`status=ok`）
- `action`=`recommend`
- `mode`=`merchant_recommendation`
- `provider`
- `anchor.source` / `anchor.label` / `anchor.latitude` / `anchor.longitude`
- `query.keyword` / `query.category` / `query.cuisine` / `query.price_level` / `query.sort_by` / `query.radius_meters` / `query.top_k`
- `returned`
- `candidates[]`：每个候选包含 `provider`、`name`、`address`、`distance_meters`、`rating`、`average_cost`、`score`、`reasons`、`category`、`phone`、`location`、`navigation_links`、`place_url`

## Error Contract
- 未提供坐标，且未提供任何可用于定位的地点字段。
- 默认 provider 未启用，或对应 provider 未配置 API Key。
- `action` 非 `recommend`。
- 高德或 Google 地点解析失败。
- 高德或 Google 商户搜索失败或未找到满足条件的商户。

## Request/Response Examples
### Example 1：默认 provider（高德）查询
Request:
```json
{"request_id":"map-1","args":{"action":"recommend","city":"上海","address":"人民广场","keyword":"咖啡","top_k":3}}
```
Response:
```json
{"request_id":"map-1","status":"ok","text":"…","extra":{"action":"recommend","provider":"amap","returned":3},"error_text":null}
```

### Example 2：显式使用 Google
Request:
```json
{"request_id":"map-2","args":{"action":"recommend","provider":"google","latitude":37.422,"longitude":-122.084,"keyword":"coffee","top_k":3}}
```
Response:
```json
{"request_id":"map-2","status":"ok","text":"…","extra":{"action":"recommend","provider":"google"},"error_text":null}
```

### Example 3：错误（provider 未配置 key）
Request:
```json
{"request_id":"map-3","args":{"action":"recommend","provider":"google","keyword":"coffee","city":"Mountain View"}}
```
Response:
```json
{"request_id":"map-3","status":"error","text":"","extra":null,"error_text":"…"}
```
