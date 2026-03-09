# Hard Rules 配置说明

本文说明当前“硬规则引擎”配置位置、加载方式与字段含义。  
硬规则用于安全兜底，不替代 LLM 主流程。

## 1. 规则文件位置

- 交易硬规则：`configs/command_intent/trade_rules.toml`
- 语音模式 alias 规则：`configs/command_intent/voice_mode_intent_aliases.toml`

## 2. 交易规则（trade_rules.toml）

### 意图与买卖方向

- `[intent].keywords`：是否进入交易硬路由的关键词
- `[side].buy_keywords`：识别买单方向
- `[side].sell_keywords`：识别卖单方向

### 订单类型与交易所

- `[order_type].limit_keywords`：命中则按限价单处理，否则按市价单处理
- `[exchange].default`：未命中任何交易所 alias 时的默认值
- `[exchange.aliases]`：交易所别名映射，例如 `okx = ["okx","欧易"]`

### 确认词与数值提取

- `[confirm].yes`：确认提交下单的词表（精确匹配）
- `[confirm].no`：取消下单的词表（精确匹配）
- `[regex].qty_patterns`：提取数量的正则数组（按顺序尝试）
- `[regex].price_patterns`：提取价格的正则数组（按顺序尝试）

### 加载失败回退

- 文件缺失 / TOML 解析失败 / 正则无效时，系统自动回退内置默认规则。

## 3. 语音模式兜底规则

`voice_mode_intent_aliases.toml` 使用以下键：

- `voice_aliases`
- `text_aliases`
- `both_aliases`
- `reset_aliases`
- `show_aliases`
- `none_aliases`

语音模式识别顺序为：

1. LLM 输出标准标签（`voice/text/both/reset/show/none`）
2. alias 文本兜底
3. 极小范围 contains 兜底（例如 `voice/语音`、`text/文字/文本/打字`）

## 4. 代码接入位置

- 通用规则实现：`crates/claw-core/src/hard_rules/`
- 交易硬路由消费方：`crates/clawd/src/main.rs`
- 语音兜底消费方：`crates/telegramd/src/main.rs`
