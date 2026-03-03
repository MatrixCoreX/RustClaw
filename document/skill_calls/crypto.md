# crypto

- 脚本：`scripts/skill_calls/call_crypto.sh`
- 默认参数：`{"action":"quote","symbol":"BTCUSDT"}`
- 示例：
  - `bash scripts/skill_calls/call_crypto.sh`
  - `bash scripts/skill_calls/call_crypto.sh --args '{"action":"quote","symbol":"PEPEUSDT"}'`
  - `bash scripts/skill_calls/call_crypto.sh --args '{"action":"quote","symbol":"BTCUSDT","exchange":"gateio"}'`
  - `bash scripts/skill_calls/call_crypto.sh --args '{"action":"book_ticker","symbol":"BTCUSDT","exchange":"coinbase"}'`
  - `bash scripts/skill_calls/call_crypto.sh --args '{"action":"quote","symbol":"BTCUSDT","exchange":"cextest"}'`
  - `bash scripts/skill_calls/call_crypto.sh --args '{"action":"onchain","chain":"ethereum","address":"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045","token":"usdt","tx_limit":5}'`
- 常用参数：`action`, `symbol`, `symbols`, `exchange`, `chain`, `address`, `token`
- `exchange` 常见值：`auto`, `all`, `binance`, `okx`, `gateio`, `coinbase`, `kraken`, `coingecko`, `cextest`（兼容别名 `paper`）
- 新闻能力已迁移到 `rss_fetch`（例如：`bash scripts/skill_calls/call_rss_fetch.sh --args '{"action":"latest","category":"general","limit":5}'`）
