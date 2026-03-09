# Crypto 技能功能与聊天测试清单

## 1) 技能全功能（action 级别）

当前 `crypto` 技能支持以下 action：

- `quote` / `get_price`：单币价格（可用来源含 Binance / OKX / GateIO / Coinbase / Kraken / CoinGecko）
- `multi_quote` / `get_multi_price`：多币价格
- `book_ticker` / `get_book_ticker`：买一卖一盘口
- `normalize_symbol`：交易对标准化（如 `btc` -> `BTCUSDT`）
- `healthcheck`：行情接口可用性检查
- `candles`：K线数据
- `indicator`：技术指标（当前有 SMA）
- `onchain`：链上查询（BTC 概况 / ETH 概况 / ETH 地址余额与交易）
- `trade_preview`：下单预览
- `trade_submit`：提交下单（含 confirm 机制）
- `order_status`：查订单
- `cancel_order`：撤单
- `positions`：查持仓/余额

说明：

- `news` 已从 `crypto` 迁移到 `rss_fetch`。
- 默认执行模式是 `binance`。

## 2) Telegram 命令测试（推荐）

### 2.1 `/crypto` 命令

- `/crypto price pepe auto`
- `/crypto price btc gateio`
- `/crypto price eth coinbase`
- `/crypto prices btc,eth,sol all`
- `/crypto prices btc,eth kraken`
- `/crypto book btc auto`
- `/crypto book btc coinbase`
- `/crypto normalize pepe`
- `/crypto health btc`
- `/crypto address 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 usdt 5`

说明：

- 不带交易对的币种（如 `btc`、`eth`、`sol`、`pepe`）会默认补 `USDT`。
- 价格返回中会显示可用来源（Binance / OKX / GateIO / Coinbase / Kraken / CoinGecko），未获取到的来源不显示。

### 2.2 `/cryptoapi` 命令（交易所 API 管理）

- `/cryptoapi show`
- `/cryptoapi set binance <api_key> <api_secret>`
- `/cryptoapi set okx <api_key> <api_secret> <passphrase>`

### 2.3 `/run rss_fetch` 命令（新闻）

- `/run rss_fetch {"action":"latest","category":"general","limit":5}`
- `/run rss_fetch {"action":"latest","category":"crypto","limit":5}`
- `/run rss_fetch {"action":"latest","category":"international","limit":5,"classify":true}`
- `/run rss_fetch {"action":"latest","category":"china","limit":5,"classify":true}`

## 3) 自然语言聊天测试（Agent 路由）

可直接发以下句子：

- `帮我查 pepe 价格`
- `看下 btc、eth、sol 价格`
- `查下 BTC 买一卖一`
- `看下币圈新闻`
- `来5条综合新闻`
- `给我3条国际新闻，按分类展示`
- `来两条中文财经新闻`
- `查这个 ETH 地址 USDT 余额和最近交易：<地址>`
- `先帮我预览买 10u btc`
- `确认下单`
- `查币安持仓`
- `看下我在 OKX 的仓位`
- `先预览在币安卖出 0.01 ETH`
- `确认执行卖出 0.01 ETH（币安）`

### 3.1 持仓查询（建议话术）

- `查币安持仓`
- `查看币安资产`
- `看下 OKX 仓位`

期望行为：

- 路由到 `crypto.action=positions`
- 返回中包含交易所标识与持仓列表；无持仓时明确提示空结果

### 3.2 卖出流程（建议两步）

1) 先预览（安全）  
- `先预览在币安卖出 0.01 ETH`
- `预览卖出 ETHUSDT 0.01（币安）`

2) 再确认提交  
- `确认执行卖出 0.01 ETH（币安）`
- `确认提交：币安卖出 ETHUSDT 0.01`

说明：

- 卖出建议使用基础币数量（`qty`），如 `0.01 ETH`
- 若未写交易对，默认按 `ETHUSDT` 归一化处理（不与上下文冲突时）
- 未明确“确认执行”前，应优先走 `trade_preview`，不直接提交

## 4) 重点回归项

- 价格来源显示是否完整（Binance / OKX / GateIO / Coinbase / Kraken / CoinGecko 成功的要展示）
- 不存在币种或单个来源失败时，不显示 `n/a`
- 不带交易对输入是否自动补 `USDT`
- 失败信息是否原样透传给用户（error_text）
- 自然语言查价是否仍会触发重复动作保护报错
- `/crypto prices ... <exchange>` 末尾交易所参数是否正确识别（不应被当成 symbol）
- 新闻是否走 `rss_fetch`，且默认 `general` 分类生效
- `查持仓/看仓位/资产情况` 是否稳定命中 `positions`
- 卖出请求是否优先 `trade_preview`，且仅在明确确认后 `trade_submit`

