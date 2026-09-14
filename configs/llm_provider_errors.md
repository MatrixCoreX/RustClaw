# 模型服务商错误兼容与扩展

## 范围与原则

本次依据官方文档维护模型调用错误词表，不承诺枚举所有平台、代理和未来错误码。
覆盖 `clawd` 的 OpenAI-compatible、Anthropic Messages、Gemini generateContent
三种协议的普通文本与 native tool-turn 六个 HTTP 入口，以及 OpenAI-compatible SSE/NDJSON。
这不等于为每个服务商增加全新协议，也不改变独立媒体技能内部的 HTTP 客户端、技能超时、
认证、用户权限或通信端逻辑。

错误分类由 provider adapter 处理；planner、任务恢复与通信端继续消费已有
`ProviderFailureClass` / `TaskProviderBlocker`，不读取用户自然语言来决定重试。
供应商名来自配置与实际 endpoint，不从模型名字或错误文案推断供应商。

## 已核查的来源（2026-09-14）

| 服务商 | 此次重点 | 官方来源 |
| --- | --- | --- |
| OpenAI | 429 中区分余额、组织/项目支出额度与短期限流 | [错误码](https://developers.openai.com/api/docs/guides/error-codes) |
| Anthropic | 402 billing、429 rate、529 overload；200 后流中也可出错；413 不等于上下文超长 | [错误处理](https://platform.claude.com/docs/en/api/errors) |
| Gemini | RPC status、Interactions 的 quota/rate 区别；裸 RESOURCE_EXHAUSTED 不直接认定欠费 | [排障](https://ai.google.dev/gemini-api/docs/troubleshooting)、[API errors](https://ai.google.dev/gemini-api/docs/api-errors) |
| MiniMax | 1008 余额、2056 Token Plan、1002/2045 限流、数值错误码 | [错误码](https://platform.minimax.cn/docs/api-reference/errorcode) |
| DeepSeek | 402 余额、429 限流、500/503 服务错误 | [错误码](https://api-docs.deepseek.com/quick_start/error_codes/) |
| 千问 / DashScope | Arrearage / FreeTierOnly；insufficient_quota 也可表示 TPM 限流 | [错误码](https://help.aliyun.com/zh/model-studio/error-code) |
| MiMo | 402 余额、421 内容过滤；429 本身不能区分套餐耗尽与频率限制 | [错误码](https://mimo.mi.com/docs/zh-CN/api/guidance/error-codes) |
| xAI | 标准 HTTP 权限、参数、限流错误 | [排障](https://docs.x.ai/developers/debugging) |
| 智谱 | 1113 欠费、1308/1310 与 1316–1321 配额、1302 限流、1305 过载、1261 上下文 | [错误码](https://docs.bigmodel.cn/cn/api/api-code) |
| SiliconFlow | 20012 模型不存在；其他已核实的 HTTP 错误 | [错误处理](https://docs.siliconflow.cn/en/faqs/error-code)、[文本生成](https://docs.siliconflow.cn/docs/userguide/capabilities/text-generation) |
| OpenRouter | 数值 error.code，包括流中报错；402、429、524、529 | [官方 SDK 错误合同](https://openrouter.ai/docs/client-sdks/typescript/api-reference/responses) |
| Groq | 498 Flex 容量不足可重试；424/499 不盲目当成临时故障 | [错误码](https://console.groq.com/docs/errors) |

Moonshot、火山方舟、Mistral 的候选文档在本轮检索中未取得可核实的错误表；没有根据
第三方转载猜造专用数字码。它们和其他 OpenAI-compatible 服务仍可使用通用 HTTP /
结构化错误兜底，也可由管理员按下面方式新增 profile。Azure、Bedrock 等托管层应单独核验，
不能仅凭底层模型是某品牌就复用它的所有错误码。

## 配置入口

`configs/llm_provider_errors.toml` 是唯一内置规则源。构建时嵌入同一个文件，独立二进制
发行不必额外复制文件才能工作。启动时：

1. 如设置 `APP_LLM_PROVIDER_ERRORS_CONFIG`，读取该路径；缺失或非法即启动失败。
2. 否则读取启动主配置文件旁的 `llm_provider_errors.toml`。
3. 只有第二项文件不存在时使用内嵌版本；存在但非法不会被静默忽略。

外部文件是完整规则集，不是隐式合并补丁。修改外部文件后重启 `clawd` 生效，无需重新编译；
只更新仓库内嵌版本而不提供外部文件时，需要重新编译。规则不会联网自动更新。
日志 `provider_error_rules_loaded` 记录 revision/profile 数量。

`schema_version` 控制解析合同，`revision` 标注规则版本；未知字段、非法类别、重复 ID /
绑定、坏 JSON pointer、缺少来源或 default profile 都会被拒绝。

## 匹配顺序与边界

- 显式 `bindings.provider_name -> profile` 优先；其次匹配 URL 的真实 hostname
  （精确域名或点分隔子域名，最长匹配优先）；再次匹配配置内 `provider_names`；最后 default。
- 先匹配所选 profile 的业务码，再匹配 default。profile 的额外 code_paths 先于公共路径；
  具体 `error.code` 先于宽泛 `error.type`。同一路径的代码规则按文件顺序匹配。
- 数字与字符串错误码统一成字符串，大小写保持供应商定义，不全局模糊匹配。
- HTTP 非成功总是进入错误分类。成功响应只在顶层 error/type=error 或该 profile 声明的
  `failure_code_paths` 上出现非成功值时进入；模型正文、工具参数里的 error 不算供应商错误。
- `failure_code_paths` 的空值不表示失败；成功码取该 profile 的 `success_codes`，未设置时
  取根级 success_codes（内置为 `0`、`200`、`OK`、`Success`）。供应商变更成功码也只需改配置。
- MiniMax 的 Anthropic 兼容响应实测把 2056 放在 `error.message` 末尾。
  `message_code_paths` 仅提取末尾 ASCII `(数字)`，且只在指定供应商的错误 envelope 及
  `message_code_http_statuses` 声明的状态内生效（本例 429），防止普通参数错误回显被误判。
  这是数字码的封装适配，不是中英文关键字判断；明确的结构化业务码优先。
- `http_status_paths` 显式声明哪些数值字段真的是 HTTP 状态，供 HTTP 200 错误/流帧使用。
  不把任何服务商业务数字码自动当 HTTP 状态码。OpenRouter 声明 `/error/code`。
- 不知道的新错误保留原始证据并使用 HTTP 兜底，不猜测自然语言。401/403/413/普通 4xx
  不短重试，402 按额度等待，408/504 按超时，429 按限流，5xx 按服务故障处理。

## 重试与用户反馈

已确认的额度耗尽不再执行 5/15/30/60 秒短重试，交给原有后台 checkpoint 等待。
未提供更准确提示时沿用既有额度等待默认值 3 小时；本改动不声称该值是每个套餐的真实恢复时间。
普通暂时性故障保留已有有界短重试，不新增任务寿命上限。

支持 `Retry-After` 秒数、HTTP 日期，以及 Google RPC `RetryInfo.retryDelay`。
服务商给出正等待时间时立即把等待交给原有后台恢复路径，不占着 worker 睡眠，不提前重试。
同时存在多个等待提示时取较晚者，非法/溢出提示忽略；不会用此提示改变认证/参数错误的终态类别。
Google 的重试字段定义见 [RetryInfo](https://docs.cloud.google.com/php/docs/reference/common-protos/latest/Rpc.RetryInfo)。

分类日志含 rule、profile、revision、HTTP/effective status 和规范类别；原始响应留在既有
模型 I/O 证据中。对外 error.message 只含机器 token，不直接拼接供应商原始文案或凭据。
本次没有增加通信端固定回复；解释与现有 message_key / 模型合成链路保持分离。

歧义必须保留：例如千问 `insufficient_quota` / `Throttling.AllocationQuota`、Anthropic
裸 `rate_limit_error`、MiMo 裸 429，可能缺少能区分短期限流与长期额度的结构化信息。
不能承诺全部识别为准确原因，也不能用缺少 Retry-After 反推欠费。

## 新增或更新服务商（无需改任务核心）

复制现有 TOML，新增 profile 和来源，在其 rules 中填写已核实的 codes / class；新增 profile
必须放在一个新的 `[[profiles]]` 块中，不能意外附加到前一 profile 的规则下面。例如：

```toml
[[bindings]]
provider_name = "my-relay"
profile = "new_vendor"

[[profiles]]
id = "new_vendor"
hosts = ["llm.example.invalid"]
provider_names = ["vendor-new"]
sources = ["https://docs.example.invalid/errors"]
code_paths = ["/problem/reason"]
failure_code_paths = ["/problem/reason"]

[[profiles.rules]]
id = "balance_empty"
class = "quota_exhausted"
codes = ["NEW_BALANCE_EMPTY"]
http_statuses = [200, 403, 429]
```

`http_statuses` 可省略；设置时与 codes 同时满足才命中；只写 http_statuses 可表达供应商特有
HTTP 状态，如 Groq 498。可用类别为 quota_exhausted、rate_limited、timeout、
provider_retryable_response、context_length_exceeded、provider_non_retryable_business。
鉴权、安全过滤、无效参数/模型均可归入最后一类，用 rule ID 保留细分诊断。
不要把服务商未经核实的消息改写成永久兼容规则。

如果新服务商使用已支持协议，仅规则扩展即可；全新请求协议或全新错误封装
仍需在 provider adapter 增加实现及测试，不需要在主 agent loop 加服务商分支。

## 验证

离线测试使用本地 HTTP fixture，不向收费服务商发请求：

```sh
cargo test -p clawd --bin clawd providers:: -j 8
```

包含：每条配置错误码的字符串/数字矩阵、跨服务商冲突、2056 封装、歧义保留、成功正文不误判、
新 profile/relay binding、坏配置拒绝、6 个 HTTP 入口、HTTP 200 业务失败、SSE 尾帧报错、
Retry-After 后不额外请求、正常文本/native/stream 回归。测试结果与仓库已有门禁问题另见验收记录。

### 本轮验收记录（2026-09-14）

- 规则集：12 个服务商 profile + default，30 条规则，138 个代码映射；代码矩阵覆盖
  197 个字符串/数字输入变体（属于单个矩阵测试，不能冒充 197 个独立测试）。
- 最终 `cargo test -p clawd --bin clawd providers:: --no-fail-fast -j 8`：
  **93 passed / 0 failed**，增量编译 1 分钟，测试 1.32 秒。未向真实模型发请求。
- 首轮测试发现 SSE 最后错误帧缺少空行时被当成普通断流；已修正并回归。
  只允许完整错误 JSON 提供失败证据，不把未结束的成功帧升级为成功。
- `product_identity_tests.sh --with-ui`：两套品牌 UI 构建 PASS。
- `check_repair_no_user_text_fields.py`、`check_no_runtime_hard_reply.py`、
  `check_cross_platform_contracts.py`、`check_channel_provider_error_contracts.py` 及其自测、
  `check_chinese_model_catalog.py`、中文模型 smoke matrix/summary 自测、相关文件
  `git diff --check` 均通过。
- `check_long_files.py` 全仓检查未通过：11 个现有超长文件，包括 config、skill_registry、
  agent_engine 测试、NNI/UI route、repo/tasks 与 verifier_tests；本轮修改未增加这些超限。
- `check_historical_hardcoded_language.py --fail-on-runtime --fail-on-ui-visible` 未通过：
  既有 `agent_engine/capability_result_synthesis.rs` 的 4 处音频转写多语言标签。
  本轮未修改该文件；新增 provider 错误消息只有机器 token。
- 产品命名检查自测通过，但全仓 inventory 被
  `desktop/docs/wallet-platform-validation-20260912.md` 两个既有仓库 URL 阻挡；没有改动该文件。
- 已执行 `cargo check -p clawd --bin clawd --target aarch64-apple-darwin -j 8`，
  在 ring/aws-lc-sys 的 C 编译阶段失败：当前 GNU cc 不支持 `-arch` /
  `-mmacosx-version-min` 等 Apple 参数。缺少可用 Apple C 交叉工具链，本轮不能声称 macOS
  编译或实机测试通过；未去远端修改源码、安装工具链或部署。
- 本轮只改本机源码与规则/说明，没有提交、推送、重启生产服务，也没有恢复已取消任务。

通用规则中的部分 context/quota token 沿用仓库原有合同；服务商专用新增项按上述官方资料维护。
