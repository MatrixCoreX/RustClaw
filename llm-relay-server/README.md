# LLM Relay Server

一个独立的 OpenAI 兼容大模型中转服务端。客户端只连接本服务，本服务按环境变量配置转发到上游大模型，并按 API key 做请求次数与 token 配额限制。

客户端可以通过 `model` 字段选择公开模型别名，例如 `default`、`minimax`、`deepseek`、`mimo`。真实供应商地址、真实模型名和 API key 只保存在服务端。

## 启动

```bash
cd llm-relay-server
export RELAY_API_KEYS=dev-local-key
export RELAY_UPSTREAM_BASE_URL=https://api.openai.com/v1
export RELAY_UPSTREAM_API_KEY=sk-your-upstream-key
export RELAY_UPSTREAM_MODEL=gpt-4o-mini

# 可选：中国模型。设置对应 key 后会自动出现在 /v1/models。
export RELAY_MINIMAX_API_KEY=your-minimax-key
export RELAY_DEEPSEEK_API_KEY=your-deepseek-key
export RELAY_MIMO_API_KEY=your-mimo-key
cargo run
```

默认监听：

```text
127.0.0.1:8788
```

## 常用环境变量

```text
RELAY_LISTEN_ADDR=127.0.0.1:8788
RELAY_API_KEYS=dev-local-key,another-key
RELAY_PUBLIC_MODEL=default
RELAY_UPSTREAM_BASE_URL=https://api.openai.com/v1
RELAY_UPSTREAM_API_KEY=sk-your-upstream-key
RELAY_UPSTREAM_MODEL=gpt-4o-mini
RELAY_UPSTREAM_VENDOR=openai
RELAY_UPSTREAM_TIMEOUT_SECONDS=60

RELAY_MINIMAX_ALIAS=minimax
RELAY_MINIMAX_BASE_URL=https://api.minimaxi.com/v1
RELAY_MINIMAX_API_KEY=
RELAY_MINIMAX_MODEL=MiniMax-M3

RELAY_DEEPSEEK_ALIAS=deepseek
RELAY_DEEPSEEK_BASE_URL=https://api.deepseek.com/v1
RELAY_DEEPSEEK_API_KEY=
RELAY_DEEPSEEK_MODEL=deepseek-chat

RELAY_MIMO_ALIAS=mimo
RELAY_MIMO_BASE_URL=https://token-plan-sgp.xiaomimimo.com/v1
RELAY_MIMO_API_KEY=
RELAY_MIMO_MODEL=mimo-v2.5-pro

RELAY_REQUESTS_PER_MINUTE=20
RELAY_REQUESTS_PER_DAY=1000
RELAY_TOKENS_PER_DAY=200000
RELAY_TOKENS_PER_MONTH=3000000
RELAY_MAX_TOKENS_PER_REQUEST=4096
```

## 接口

### 健康检查

```bash
curl http://127.0.0.1:8788/health
```

### 模型列表

```bash
curl http://127.0.0.1:8788/v1/models \
  -H 'Authorization: Bearer dev-local-key'
```

### 配额查询

```bash
curl http://127.0.0.1:8788/v1/quota \
  -H 'Authorization: Bearer dev-local-key'
```

### Chat Completions

使用默认模型：

```bash
curl http://127.0.0.1:8788/v1/chat/completions \
  -H 'Authorization: Bearer dev-local-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "default",
    "messages": [
      { "role": "user", "content": "你好" }
    ],
    "max_tokens": 1024
  }'
```

选择中国模型：

```bash
curl http://127.0.0.1:8788/v1/chat/completions \
  -H 'Authorization: Bearer dev-local-key' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "deepseek",
    "messages": [
      { "role": "user", "content": "请用一句话介绍你自己" }
    ],
    "max_tokens": 512
  }'
```

把 `model` 改成 `minimax` 或 `mimo` 即可转发到对应供应商。只有设置了对应 `RELAY_*_API_KEY` 的模型才会出现在 `/v1/models` 中。

## 当前边界

- 第一版只支持非流式请求，`stream=true` 会返回 `stream_not_supported`。
- 配额账本保存在内存中，服务重启后会清零。
- 客户端不能传上游 `base_url`、`api_key` 或自定义 header。
- 错误响应使用稳定的 `error.code` 和 `error.message_key`，便于以后接 UI 或多语言文案。
