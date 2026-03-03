# rss_fetch

- 脚本：`scripts/skill_calls/call_rss_fetch.sh`
- 默认参数：`{"action":"latest","category":"general","limit":5}`
- 示例：
  - `bash scripts/skill_calls/call_rss_fetch.sh`
  - `bash scripts/skill_calls/call_rss_fetch.sh --args '{"action":"fetch","url":"https://www.coindesk.com/arc/outboundfeeds/rss/","limit":3}'`
  - `bash scripts/skill_calls/call_rss_fetch.sh --args '{"action":"latest","category":"general","source_layer":"primary","limit":5}'`
- 常用参数：`action`, `category`, `source_layer`, `classify`, `output_language`, `bilingual_summary`, `url/feed_url/feed_urls`, `limit`, `timeout_seconds`
