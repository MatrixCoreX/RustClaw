# RustClaw 使用说明 / Usage Guide

本文件帮助开发者快速上手 `RustClaw`，并提供适合 Git 协作的标准流程。  
This document helps developers quickly start using `RustClaw` and follow a Git-friendly collaboration workflow.

## 1. 环境准备 / Environment

- 操作系统 / OS: Linux or macOS (recommended)
- 依赖工具 / Required tools:
  - `git`
  - `rustup`, `cargo`
  - `bash`
- 可选工具 / Optional:
  - `sqlite3` (for local DB inspection and troubleshooting)

## 2. 克隆与初始化 / Clone and Initialize

```bash
git clone <your-repo-url>
cd RustClaw
./setup-config.sh
```

如使用自定义配置，请先检查 `configs/` 下配置再启动。  
If you use custom settings, review files under `configs/` before starting services.

## 3. 本地运行 / Local Run

### 一键启动与停止 / Start and Stop

```bash
./start-all.sh
./stop-rustclaw.sh
```

### 按服务单独启动（按需） / Start Individual Services (Optional)

```bash
./start-clawd.sh
./start-telegramd.sh
./start-whatsappd.sh
./start-whatsapp-webd.sh
```

### 回归与模拟脚本（按需） / Regression and Simulation (Optional)

```bash
./simulate-telegramd.sh
```

## 4. 常用开发命令 / Common Development Commands

```bash
# Build
cargo build

# Test
cargo test

# Lint (if enabled in your workflow)
cargo clippy --all-targets --all-features
```

## 5. 配置与敏感信息 / Config and Secrets

- 不要提交真实密钥、Token、密码。  
  Do not commit real secrets, tokens, or passwords.
- 使用 `.env.example` 作为模板，复制为本地私有文件再填写。  
  Use `.env.example` as a template and keep real values in local private files.
- 提交前检查配置差异，避免泄露本地环境信息。  
  Review config diffs before commit to avoid leaking local environment data.

## 6. Git 协作流程（推荐） / Recommended Git Workflow

### 6.1 新建分支 / Create a Branch

```bash
git checkout -b feat/<short-name>
# or
git checkout -b fix/<short-name>
```

### 6.2 开发与自检 / Develop and Verify

1. 完成功能或修复。/ Implement the feature or fix.
2. 运行 `cargo test`，必要时补充脚本验证。/ Run `cargo test` and add script-level checks if needed.
3. 确认没有误跟踪敏感文件。/ Ensure no sensitive files are tracked:

```bash
git status
```

### 6.3 提交 / Commit

```bash
git add .
git commit -m "feat: add xxx support"
```

提交类型建议 / Recommended commit prefixes:

- `feat:` 新功能 / New feature
- `fix:` 缺陷修复 / Bug fix
- `refactor:` 重构 / Refactor
- `docs:` 文档更新 / Documentation
- `chore:` 构建、脚本、杂项 / Build, scripts, maintenance

### 6.4 推送与合并 / Push and Merge

```bash
git push -u origin <branch-name>
```

PR 建议包含 / PR should include:

- 变更目的 / Why this change
- 主要改动点 / Key changes
- 测试方式与结果 / Test plan and results
- 回滚方案（如有） / Rollback plan (if applicable)

## 7. 发布（按需） / Release (Optional)

仓库已包含发布脚本：  
The repository includes a release script:

```bash
./package-release.sh
```

发布前建议 / Before release:

- 确认分支与 Tag 策略 / Confirm branch and tag strategy
- 确认配置已脱敏 / Ensure configs are sanitized
- 记录版本变更说明 / Prepare release notes

## 8. 故障排查 / Troubleshooting

- 优先查看启动脚本输出与服务日志。  
  Check startup output and service logs first.
- 使用 `check-logs.sh` 汇总关键日志。  
  Use `check-logs.sh` to inspect critical logs quickly.
- 网络问题先查配置与端口，再查外部依赖可达性。  
  For network issues, verify config and ports before external dependency reachability.

## 9. Crypto 技能快速用法 / Crypto Skill Quick Usage

先确认 `configs/crypto.toml`（crypto 技能独立配置），默认是 `cextest` 模式（兼容别名 `paper`）并要求显式确认。  
Check `configs/crypto.toml` first (crypto skill has its own config file); default mode is `cextest` (backward-compatible alias `paper`) with explicit confirmation.

示例（Telegram `/run`）/ Examples (`/run` in Telegram):

```bash
/run crypto {"action":"quote","symbol":"BTCUSDT"}
/run crypto {"action":"multi_quote","symbols":["BTCUSDT","ETHUSDT","SOLUSDT"]}
/run crypto {"action":"indicator","symbol":"ETHUSDT","timeframe":"1h","period":20}
/run rss_fetch {"action":"latest","category":"general","limit":5}
/run crypto {"action":"onchain","chain":"bitcoin"}
```

交易双阶段建议 / Recommended 2-step trading flow:

```bash
# 1) Preview first
/run crypto {"action":"trade_preview","symbol":"BTCUSDT","side":"buy","order_type":"market","qty":0.01}

# 2) Submit only after explicit confirmation
/run crypto {"action":"trade_submit","symbol":"BTCUSDT","side":"buy","order_type":"market","qty":0.01,"confirm":true}
```

说明 / Notes:

- `trade_submit` 在未 `confirm=true` 时会被拒绝（默认策略）。
- 风控字段：`max_notional_usd`、`allowed_symbols`、`allowed_exchanges`、`blocked_actions`。
- `cextest`（兼容 `paper`）订单事件保存在 `data/crypto-paper-orders.jsonl`。
- 实盘接入支持：`binance` 与 `okx`（在 `configs/crypto.toml` 设置 `enabled=true` 并填写密钥后生效）。
- Binance 下单会自动携带 `newOrderRespType=RESULT`，并约束 `recvWindow` 在 `1..60000`。
- OKX 现货 `market` 单会自动设置 `tgtCcy=base_ccy`，保证 `qty` 语义统一为“基础币数量”。
- `order_status` / `cancel_order` 支持 `order_id` 或 `client_order_id`（二选一即可）。
- crypto skill 硬提示已 i18n 化：默认读取 `configs/i18n/crypto.zh-CN.toml`，可通过 `crypto.language` / `crypto.i18n_path` 切换。

实盘最小配置模板 / Live trading minimal config (`configs/crypto.toml`):

```toml
[crypto]
default_exchange = "binance"        # 或 "okx"
execution_mode = "binance"          # 或 "okx"
require_explicit_send = true
max_notional_usd = 200
allowed_exchanges = ["cextest", "paper", "binance", "okx", "gateio", "coinbase", "kraken", "coingecko"]
allowed_symbols = ["BTCUSDT", "ETHUSDT"]

[binance]
enabled = true
base_url = "https://api.binance.com"
api_key = "YOUR_BINANCE_API_KEY"
api_secret = "YOUR_BINANCE_API_SECRET"
recv_window = 5000

[okx]
enabled = false
base_url = "https://www.okx.com"
api_key = "YOUR_OKX_API_KEY"
api_secret = "YOUR_OKX_API_SECRET"
passphrase = "YOUR_OKX_PASSPHRASE"
simulated = true
```

实盘启用建议 / Recommended rollout:

```bash
# 1) 先验证行情和账户读取
/run crypto {"action":"quote","exchange":"binance","symbol":"BTCUSDT"}
/run crypto {"action":"positions","exchange":"binance"}

# 2) 再做预览
/run crypto {"action":"trade_preview","exchange":"binance","symbol":"BTCUSDT","side":"buy","order_type":"market","qty":0.001}

# 3) 最后确认提交（小额）
/run crypto {"action":"trade_submit","exchange":"binance","symbol":"BTCUSDT","side":"buy","order_type":"market","qty":0.001,"confirm":true}

# 4) 订单查询/撤单（支持 order_id 或 client_order_id）
/run crypto {"action":"order_status","exchange":"binance","symbol":"BTCUSDT","order_id":"123456789"}
/run crypto {"action":"order_status","exchange":"binance","symbol":"BTCUSDT","client_order_id":"my-order-001"}
/run crypto {"action":"cancel_order","exchange":"okx","symbol":"BTCUSDT","client_order_id":"my-order-001"}
```

一键回归 / One-command regression:

```bash
chmod +x scripts/regression_crypto_skill.sh
./scripts/regression_crypto_skill.sh debug
./scripts/regression_crypto_skill.sh release
./scripts/regression_crypto_skill.sh debug --auto-build
```

`clawd` 自然语言触发回归（测试 LLM 是否会触发 `crypto` 技能）:

```bash
chmod +x scripts/regression_clawd_crypto_trigger.sh

# 默认触发模式：允许“触发型失败”记为通过（用于验证路由触发）
./scripts/regression_clawd_crypto_trigger.sh --wait-seconds 120

# 严格模式：必须任务成功（status=succeeded）才算通过
./scripts/regression_clawd_crypto_trigger.sh --wait-seconds 120 --strict

# 严格模式 + 关闭瞬时错误重试
./scripts/regression_clawd_crypto_trigger.sh --wait-seconds 120 --strict --no-retry
```

---

建议扩展顺序：环境准备 -> 启动方式 -> Git 流程 -> 排障。  
Recommended extension order: Environment -> Run Flow -> Git Workflow -> Troubleshooting.
