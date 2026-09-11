# 独立资产账户：后端实施交接

日期：2026-09-11。协议为 `asset_owner_v1`，桌面版本为 0.3.2。Core、两个 Edge、本机网关/桌面/UI、网站前后端均已部署。已安装桌面通过真实链路完成免密码读取和无余额交易拒绝；资金成功、重放和故障场景使用隔离账本验证，未动用生产资产。部署版本、测试证据及剩余边界见第 13 节。

**给后端 agent 的执行要求：适配已安装桌面客户端生成的独立资产账号，使其无需绑定硬件即可展示资产、进行 Bancor 买卖和转账。** 第 3–5 节是客户端当前实际合同；第 7–12 节是具体改动清单、数据设计、兼容范围和交付验收。不要把桌面本地账号改成设备账号，也不要将安装客户端视为获得服务端权限。

## 1. 任务和边界

让**没有绑定任何硬件的新 K1 资产公钥**独立查询 AIC / USD 余额与记录，完成 Bancor 买入、卖出及转账。私钥始终留在桌面原生密钥库，服务端只接收公钥和针对本次操作的签名。不要要求把新账户替换成硬件绑定账户，不要接收、保存或代管新账户的私钥。

现有绑定账户可以使用对应私钥签名，但这不代表网关已支持任意独立账户。目前 `nni_bancor.rs` 和 `nni_asset_transfer.rs` 的 `asset_owner` 路径会检查配置的绑定账户；不同公钥可能返回 `nni_asset_owner_mismatch`。`nni_financial_account()` 仍通过设备公钥及硬件签名查询。这是本次需要补齐的边界。

桌面私钥操作仍只在 `desktop/`；联合适配在专属网关模块、webd 的可信会话边界及独立 Core 服务内完成：

- 新增专属模块，例如 `crates/clawd/src/http/ui_routes/nni_owner_financial/`，按仓库规则拆分实现与测试。主路由仅做必要注册，避免继续扩展既有超长文件。
- 保留旧浏览器、硬件账户、NNI 加入/解绑、心跳、奖励、设备恢复及已有配置行为。
- 现有浏览器局域网 HTTP 入口保持可用。新功能使用桌面现有已认证连接；局域网桌面连接仍要求可信 HTTPS 或 SSH 隧道，本机支持严格 loopback HTTP。
- 网关到资产节点必须校验证书，当前生产节点配置仅允许 HTTPS。协议和隔离测试允许 literal loopback HTTP，不表示生产配置已开放该例外。不能跳过 TLS 校验、切换到硬件账户或上传私钥来绕过不支持的接口。
- 上游已定位为 `/home/guagua/NNI/nni_server`，代码审计基线 `140e9cc`。具体部署版本与账本身份需在上线时再次核对。

## 2. 桌面已实现什么

| 内容 | 桌面实现位置 |
| --- | --- |
| 原生 K1 密钥生成、密码解锁、系统凭据库、加密备份恢复 | `desktop/src/wallet/` |
| 独立安全窗口，核对实际签名内容后才签署资金操作 | `desktop/frontend/wallet/manager.tsx` |
| 资产/Bancor 账户选择、余额、记录、转账与买卖表单 | `desktop/frontend/wallet/pages.tsx` |
| 固定 API 路径、设备登录与 CSRF、15 秒超时、无自动写重试 | `desktop/src/asset_operations/client.rs` |
| 严格协议类型、金额、费用、滑点、账户/账本/操作绑定校验 | `desktop/src/asset_operations/protocol.rs` |
| 提交前持久化操作记录、未知结果查询、原生窗口权限隔离 | `desktop/src/asset_operations/commands.rs` |

首次请求先读 capabilities。未升级或未开启 owner 接口的后端返回 404/405/501 时，桌面显示“尚未支持桌面本地账户”，保留账户，但不伪造余额、执行交易或降级签名。本次已核验的节点见第 13 节，不能假定任意第三方节点都支持。

## 3. 网关 API 合同

统一前缀：`/v1/nni/assets/owner`。所有接口要求设备管理员登录，继续复用原有认证、CSRF、Origin、请求体大小与限流中间件；不要增加品牌 header 或独立的客户端超级密钥。

成功响应统一 `{"ok":true,"data":<typed data>}`，可以带 `"error":null`。失败使用相应非 2xx HTTP 状态与 `{"ok":false,"error":"稳定错误码"}`。桌面严格拒绝成功响应中的未知字段、错误类型、重复字段；不要把上游大 JSON 直接透传。

| 方法和路径 | 用途 |
| --- | --- |
| `GET /capabilities?service=assets` 或 `bancor` | 获取该配置服务实际支持的账本与操作 |
| `POST /read/request` | 余额、记录、操作状态的只读 challenge |
| `POST /read/public` | 桌面默认读取公开余额、历史、最小操作状态，不签名、不解锁 |
| `POST /read/verify` | 验签后返回只读结果 |
| `POST /operations/request` | 转账或 Bancor 报价及资金操作 challenge；此时不得扣款 |
| `POST /operations/verify` | 对此前保存的精确 challenge 验签，并幂等执行 |

### 3.1 Capabilities

```json
{
  "ok": true,
  "data": {
    "schema_version": 1,
    "protocol": "asset_owner_v1",
    "ledger_id": "ledger-fixture-v1",
    "node_url": "https://ledger.example.test",
    "service": "assets",
    "actions": ["balances", "history", "operation_status", "transfer"]
  }
}
```

`bancor` 对应 actions 为 `balances/history/operation_status/bancor_trade`。不能提前声明尚未完整实现的能力。

- `ledger_id` 必须是实际账本的稳定、唯一身份，不能使用所有节点共用的 `nni-server-v1`。长度 1–128 字节，不包含控制字符。节点切换或恢复备份后，应保留或更新与实际账本身份一致的值。
- `node_url` 必须来自管理员配置，不由请求体任意指定。允许 HTTPS，或 literal `127.0.0.1` / `::1` 的 HTTP；不允许 URL 用户信息、query、fragment。
- `assets` 与 `bancor` 可以配置不同节点；只有底层确实同账本才可共享余额。所有结果必须标记真实来源。
- 同一 Bancor 服务的公开行情 `market.node_url` 必须与 capabilities 的规范 URL 完全一致。桌面会检查来源一致后才允许按该行情准备交易。
- `actions` 最多 5 项。协议版本或字段变化须同步桌面，不能静默改变 v1 语义。

### 3.2 Request 请求体和 intent

```json
{
  "schema_version": 1,
  "protocol": "asset_owner_v1",
  "ledger_id": "ledger-fixture-v1",
  "node_url": "https://ledger.example.test",
  "service": "assets",
  "account": "<完整 K1 公钥>",
  "operation_id": "<桌面生成的 UUID>",
  "intent": {"kind":"balances"}
}
```

网关必须重新校验 body 的 service、账本、节点与当前配置，不得据此访问任意 URL。将 challenge 绑定认证 actor、会话、服务、账本、公钥、operation_id、操作参数、有效期；不能把设备管理员登录本身当作任意资产账户的所有权证明。

支持的完整 intent：

```json
{"kind":"balances"}
{"kind":"history","page":1}
{"kind":"operation_status","operation_id":"<待查询的原资金操作 UUID>"}
{"kind":"bancor_trade","side":"buy","input_units":"100000000","slippage_bps":300,"max_fee_bps":100}
{"kind":"transfer","asset":"AIC","amount_units":"100000000","recipient":"<收款 K1 公钥>","memo":"可选说明","max_fee_bps":0}
```

- 金额采用十进制**整数字符串**，精度 `10^8`；`100000000` 代表 1 个资产单位。禁止浮点、指数、负号、前导零；最大 `9223372036854775807`。输入/转账金额必须大于 0，余额和手续费可以为 0。
- Bancor 仅允许 `service=bancor`，`side=buy|sell`；buy 支付 USD、收取 AIC，sell 相反。
- 转账仅允许 `service=assets`，`asset=AIC|USD`；收款公钥校验有效且不能等于付款公钥；memo 必须提供，可为空，UTF-8 最多 256 字节。
- 两种 bps 都为整数 0–5000，100 bps = 1%。history 页码 1–100000，每页最多 20 条。
- operation_status 外层 operation_id 是**本次只读 challenge 的新 UUID**；intent 内是要查询的原资金操作 ID，不得混淆。

### 3.3 Challenge 响应和实际签名内容

响应 data 只有 `signing_payload`，值是下述 JSON 的**原始 UTF-8 字符串**：

```json
{
  "schema_version": 1,
  "protocol": "asset_owner_v1",
  "ledger_id": "ledger-fixture-v1",
  "node_url": "https://ledger.example.test",
  "service": "assets",
  "account": "<完整 K1 公钥>",
  "operation_id": "<原 request 的 UUID>",
  "challenge_id": "<服务端随机 UUID>",
  "nonce": "<32 个随机字节的 64 位小写 hex>",
  "expires_at_unix": 1700000120,
  "terms": {"kind":"balances"}
}
```

这是结构示例，时间戳不是可复用请求。实际字符串最长 8192 字节；时间必须满足 `now < expires_at_unix <= now + 300`。challenge_id 非空 UUID，nonce 使用密码学随机源。原始字符串及关联参数保存在服务端；验签不能重新序列化 JSON、重新排序或改变 Unicode 转义。

只读 terms 与 intent 完全相同。资金 terms 保留 intent 全部字段并新增：

| 类型 | 新增字段 | 必须满足的执行语义 |
| --- | --- | --- |
| `bancor_trade` | `fee_units`, `quoted_output_units`, `min_output_units` | fee 从 input 内扣除，input 是总扣款上限；报价按净投入计算；实际输出不能低于 min |
| `transfer` | `fee_units` | 收款方得到 amount；付款方总扣款为 amount + fee；费用使用同资产 |

Bancor：`fee <= floor(input * max_fee_bps / 10000)` 且 `fee < input`；quote/min 均为正数，`min <= quote`，`min >= floor(quote * (10000-slippage_bps)/10000)`。转账：同样限制 fee，amount+fee 不得溢出 i64。服务端执行必须守住**签过的实际 fee、总扣款与最低收到**，不能只校验客户端允许的最大费率后重新加价。

报价失效、价格变化越过最低收到、余额不足时，返回明确未执行结果；不得在后台自动重新报价或要求签名一个不同 payload 来复用原授权。

### 3.4 Verify 请求体

```json
{
  "schema_version": 1,
  "protocol": "asset_owner_v1",
  "ledger_id": "ledger-fixture-v1",
  "node_url": "https://ledger.example.test",
  "service": "assets",
  "account": "<完整 K1 公钥>",
  "operation_id": "<该 challenge 对应的 UUID>",
  "challenge_id": "<challenge UUID>",
  "signature": "<128 位小写 hex>"
}
```

签名为 secp256k1 ECDSA，对原始 signing_payload UTF-8 字节做**一次 SHA-256**，编码为固定 64 字节 `r || s`，使用 low-S。不是 DER、不是带恢复位签名，不加以太坊消息前缀，不二次哈希。公钥为压缩 33 字节，加 `RIPEMD160(pubkey || ASCII("K1"))[0..4]` 校验和后 Base58 编码，无前缀。

可直接用于后端测试的字节级向量：`desktop/tests/fixtures/asset-owner-v1-vectors.json`，包含余额、转账（中文 memo）、Bancor 买入。其固定公开测试私钥仅供测试，**不可充值**。向量由现有 UI 加密库生成，并由 Rust 原生签名逐字节验证。

## 4. 返回结果

### 余额或记录：`/read/public`（或显式签名读取 `/read/verify`）

```json
{
  "ok": true,
  "data": {
    "account":"<当前签名公钥>",
    "ledger_id":"ledger-fixture-v1",
    "node_url":"https://ledger.example.test",
    "aic_balance_units":"12345000000",
    "usd_balance_units":"9000000000",
    "page":1,
    "total_pages":1,
    "records":[]
  }
}
```

balances 返回 page=1；history 返回请求页。records 项只有 `operation_id`（UUID）、`kind`（`bancor_buy|bancor_sell|transfer_in|transfer_out`）、`asset`（AIC/USD）、`amount_units`（整数字符串）、`counterparty`（公钥或 null）、`created_at_unix`（Unix 秒）。单笔操作若有多条资产变动可重复 operation_id。余额与记录必须属于该签名账户，不能混入硬件账户。

### 资金操作或 operation_status：verify 结果

```json
{
  "ok": true,
  "data": {
    "operation_id":"<原资金操作 UUID>",
    "account":"<付款公钥>",
    "ledger_id":"ledger-fixture-v1",
    "status":"succeeded",
    "receipt_id":"<账本凭证 ID 或 null>"
  }
}
```

status 只允许 `succeeded|failed|pending|expired`。receipt_id 最多 128 字节。每个响应包括信封最长 256 KiB。

- succeeded 表示账本已持久化提交，不是排队成功。
- failed 表示已确认没有执行；expired 表示已确认授权失效且没有执行，不能仅因客户端超时而返回。
- pending 表示仍无法确定；必须保留可查询状态。网络断开、网关重启或上游超时不能变成确定失败。
- 对未找到的操作，也必须与上游和 challenge 存储核对；不能把“本地缓存未找到”误当作未扣款。

已被桌面识别的错误码：`nni_asset_owner_mismatch`、`asset_owner_insufficient_balance`、`asset_owner_challenge_expired`；401/403 显示权限失效。其他稳定错误码可新增，但需同步桌面文案映射。资金 verify 返回异常时，桌面会保留 pending 并要求查询结果，不会自动重发。

## 5. 后端必须落实的安全与一致性

1. 独立账户授权：资金操作以验签公钥确认资产所有者；不依赖硬件绑定、不替换设备配置。余额和公开流水属于公开账本事实，使用受限只读接口，不要求资产私钥。签名证明、凭据及非公开数据不在只读响应中。
2. 重放隔离：只读与资金操作、买卖方向、不同账户、节点、账本、会话和 challenge 均绑定；校验精确类型和全部参数。重复/未知 JSON 字段、非规范金额、过期/未来超限时间、非法曲线点和 high-S 一律拒绝。
3. 原子幂等：以账本+付款账户+operation_id 建唯一约束并保存请求摘要。同 ID 同请求返回同结果；同 ID 不同参数拒绝；并发 verify 只能扣款一次。幂等记录必须跨网关/资产节点重启保留，不能只靠内存 nonce。
4. 端到端所有权证明：如果网关需要把请求转给另一资产服务，后者必须验证此账户签名或验证等效的、严格受控的委托证明。不得因网关持有 client_user_key 就绕过资产所有权。
5. 创建/收款：确认未绑定硬件的合法新公钥可以接收测试转账并获得余额；若上游需要建账，应通过本协议或收款事务轻量完成，不要求加入 NNI。
6. 数据和日志：不得记录私钥、密码、会话 token 或可重放签名。保存的审计/幂等记录需按账户/actor 隔离并有权限、保留和恢复策略；技能私有数据遵循仓库 ownership 合同。
7. 网络与限额：配置节点白名单、正常 TLS 验证、请求超时/限流/大小上限、分页上限；重定向不能扩展到未经授权地址。设备入口 HTTP 的既有行为无需变化。

## 6. 实施顺序与验收清单

- [x] P0：登记网关部署版本、上游仓库/版本/节点、真实账本身份；核验独立公钥收款、client_user_key 用途、服务是否同账本。
- [x] P1：实现 strict 类型、capabilities、只读 request/verify、账户与页码隔离；用交接向量验证字节编码和一次哈希。
- [x] P2：本地实现 Bancor/转账 challenge、费用与最低收到约束、原子幂等及状态查询；服务默认不开放，部署验收后再配置启用。
- [x] P3：隔离账本验证首次收款、独立账户余额、买卖和转账；既有绑定、奖励与浏览器行为由 Core 全量回归另行覆盖。
- [x] P4：Core 与桌面联合覆盖字段/签名篡改、金额与费用/最小输出边界、未知/重复字段、读写隔离及并发重放。
- [ ] P5：扣款后断开响应、网关/上游重启、客户端重启、配置节点变化、余额不足与报价过期；状态查询得出同一最终结果，绝不重复扣款。
- [ ] P6：旧浏览器 HTTP、硬件签名/对应私钥签名、Ubuntu 桌面实际端到端联调；记录测试版本与证据，不以本地 fixture 成功代替真实账本验证。

桌面协议回归入口：`cargo test --locked --manifest-path desktop/Cargo.toml --no-default-features`；Ubuntu 原生测试：`dbus-run-session -- /usr/bin/python3 desktop/tests/wallet_native_e2e.py <桌面二进制>`。后者默认使用隔离凭据库、loopback TLS 与测试账本；文件选择由 D-Bus 测试夹具模拟。显式设置 `OWNER_DEPLOYED_ORIGIN` 与权限为 0600 的 `OWNER_DEPLOYED_KEY_FILE` 后，额外验证真实部署链路的公开读取和无余额拒绝，不发送真实资金签名；测试后删除临时凭据文件。

开发后按仓库 AGENTS.md 执行产品身份、跨平台、存储所有权、MCP、长文件及相关 Rust/UI 门禁；部署按仓库要求先提交推送。没有真实资产操作授权时，仅使用隔离测试账户和测试资产。

## 7. 账号含义及客户端接入流程

本需求中的“用户端账号”专指桌面密钥库生成的 K1 **资产账号**，不新增一套网站注册/登录体系。

| 对象 | 所有者及用途 | 后端应如何处理 |
| --- | --- | --- |
| 设备登录身份 / admin actor | 允许用户连接并使用该网关 | 每次请求沿用现有鉴权；与资产所有权分开校验 |
| 硬件设备绑定账号 | 原设备配置中的资产公钥，既有硬件委托签名等流程 | 完整保留现有约束、绑定、奖励与恢复逻辑 |
| 桌面本地资产账号 | 桌面生成或从加密备份恢复的 K1 公私钥 | 公钥作为账本资产所有者；只接收针对当前 challenge 的签名 |
| `test1` 等自定义名称、桌面 account UUID | 本机显示、选择和密钥索引 | 当前协议不上传；不得要求后端登记别名或把本地 UUID 当账本主键 |
| 密钥库密码、私钥、加密备份、系统凭据库条目 | 客户端本地保管 | 不新增上传、同步、托管或恢复接口 |

完整用户流程及后端动作：

1. 用户在客户端创建密钥并备份：全部本地完成，无注册 API。后端必须能处理一个从未绑定设备的合法公钥。
2. 用户连接本机、局域网或公网 HTTPS 网关并登录：复用现有登录。当前桌面资产原生桥要求 `role=admin`；放开普通用户是另外的授权需求，不能在此实现中静默放开。
3. 用户选择“桌面本地账号 · test1”：客户端对所选 `assets` / `bancor` 服务读取 capabilities，以该公钥调用 read/public；无需解锁或签名，后端只返回该账户在指定账本中的公开资产信息。
4. 从硬件账号切换到本地账号，或从 B 切到 C：只改变每次请求的签名账户；不能调用加入、恢复、解绑接口，不能写 `asset_owner_pubkey` 设备配置。
5. 用户填写转账或 Bancor 表单：request 只返回报价和待签名内容。用户在桌面安全窗口核对并逐笔输入密码后才签署并发送 verify；关闭窗口、返回编辑或只看报价均不得扣款。
6. 执行成功后客户端重新查询余额和记录；通信中断时保留 pending，通过 read/public 核实原 operation_id，无需密码，不能再次发起扣款。
7. 用户在另一台客户端恢复同一私钥：解锁、登录后可操作同一账本账户，不应要求重新绑定硬件或迁移余额。本地别名与本机提交记录不自动同步；账本历史仍从服务端查询。

新账号零余额是有效状态：合法但尚未存在的账号通过公开读取返回真实零余额、`page=1,total_pages=1,records=[]`；禁止给它分配硬件账户的余额。基础设施异常或查询失败不能包装成零余额。首次收款建账与余额入账必须在一个账本事务中完成；新建公钥不获得奖励、资金或设备成员资格。

## 8. 已定位的代码及需要适配的位置

以下保留迁移定位表，已完成的新增模块以第 13 节为准。网关不是账本实现：上游位于 `/home/guagua/NNI/nni_server`，所有资金写入和终态幂等均由 Core 所有。

| 位置 / 函数 | 当前行为 | 实施要求 |
| --- | --- | --- |
| `crates/clawd/src/http/ui_routes.rs` 的 NNI 路由注册 | 注册旧 market/account/quote/trade/transfer/transfers 等接口，没有 owner 前缀接口 | 在现有 `/v1` 挂载下注册第 3 节的 5 个入口；不要产生 `/v1/v1`，不要覆盖旧路由 |
| `ui_routes/nni_bancor.rs::nni_financial_account`、`query_nni_bancor_account_for_node` | 先取设备公钥，调用 `run_nni_signature_helper` 签署上游 account challenge | 独立账户公开读取走新增 read/public；不要求解锁、签名芯片、NNI 加入状态或设备公钥 |
| `ui_routes/nni_bancor.rs::nni_bancor_trade` | asset_owner 模式在存在配置 owner 时执行公钥匹配；其余模式使用硬件路径 | 新协议走独立账户处理器；保留旧模式限制，不通过删比较、伪造设备公钥或改配置兼容 |
| `ui_routes/nni_asset_transfer.rs::nni_asset_transfer` | 有同类 configured owner 限制及既有 challenge/verify 结构 | 新接口实现独立签名、固定费用、账本幂等；不得把新签名强塞进旧任务 challenge |
| `ui_routes/nni_asset_transfer.rs::nni_asset_transfer_history`、`query_nni_asset_transfer_history_for_node` | 管理员按公钥读取公开 explorer transactions，带 source/direction 分页 | 新 owner/history 使用明确账户的 read/public，不复用全局“当前账户” |
| `ui_routes/nni_remote_api.rs::nni_asset_service_remote_nodes`、`nni_bancor_service_remote_nodes` | 分别解析资产/Bancor 配置节点列表 | 按 service 解析并固定来源；capability、challenge、verify、status 全链路使用同一账本身份及来源 |
| `ui_routes/nni_owner_identity.rs::normalize_nni_owner_public_key` | 现有 K1 公钥格式校验入口 | 核验与客户端向量一致后复用基础校验；不能复用设备 owner 的持久化绑定逻辑 |
| `ui_routes/nni_remote_join.rs` 中 `persist_nni_asset_owner_pubkey`、加入/恢复/解绑处理器 | 管理硬件绑定 owner | 本需求不应调用或修改这些流程；以回归测试证明 B/C 操作不改变 A 的绑定 |
| `crates/webd/src/main.rs` 的会话代理、CSRF/Origin 校验及 `require_ui_admin` 所在鉴权模块 | 代理会话登录，网关检查 admin；webd 会消费 CSRF 后再转发 | 确认 owner 路由经过相同鉴权链。不能跳过登录，也不能要求客户端新增未知认证头 |
| `/home/guagua/NNI/nni_server/owner_financial/`，审计基线 `140e9cc` | 新 owner 原字节验签、延迟建账、账本写入和并发幂等已本地实现与测试 | 生产仍需配置节点白名单并验证部署；未配置时返回 unsupported |

`ui_routes/` 相对目录均指 `crates/clawd/src/http/ui_routes/`。建议在 `nni_owner_financial/` 下拆出 `mod.rs`（路由与鉴权）、`protocol.rs`（严格类型）、`challenge.rs`、`service.rs`（读取与资金业务）、`store.rs`（幂等状态）、`upstream.rs`（上游适配），测试放独立 `*_tests.rs`。这只是组织建议，按现有模块边界调整，不扩大旧超长文件。

### 8.1 每个接口的实施步骤

| 接口 | 收到请求后必须完成 | 成功语义及不可做的事 |
| --- | --- | --- |
| capabilities | 管理员鉴权 → 校验 service → 解析配置节点 → 核验上游账本身份及可用能力 → 输出精简类型 | 仅声明已通过端到端验收的 action；不能硬编码所有节点均支持 |
| read/public | 严格反序列化 → 验证 account/账本/来源/只读 intent → 返回公开余额/流水/最小操作状态 | 无签名、无私钥、无 challenge 写入；不存在账户返回零，不开户 |
| read/request | 严格反序列化 → 验证 account/账本/来源/intent → 绑定 actor 和可信会话上下文 → 持久化只读 challenge | 显式签名读取的入口，不由桌面默认查询使用 |
| read/verify | 根据 challenge_id 加载原字节 → 核对完整上下文及期限 → 验签 → 读取对应账户/分页/原操作结果 | 只接受只读 terms；不能产生报价执行或转账副作用 |
| operations/request | 同上参数核验 → 查询可执行报价和费用 → 生成完整资金 terms → 保存 challenge 及意图摘要 | 不扣款、不占用不可恢复余额；同 ID 不同参数返回冲突 |
| operations/verify | 验证保存的精确字节及上下文 → 原子取得执行权 → 由账本事务执行 → 持久化可核验结果 | 同 ID 最多执行一次；不能改价、改收款人、改来源或向下一节点重复提交 |

### 8.2 认证、会话与代理边界

- actor 必须取自服务端认证结果，不能信任 body/query 中自报的 user_id、role、session_id 或设备标识。现有第 3 节客户端请求体没有这些字段，不应要求桌面添加。
- webd cookie 会话中的 POST 继续校验 CSRF/同源；现有可信直接 key 入口沿用其服务端规则。测试应分别覆盖这两种入口，不能为了桌面而关闭整个站点 CSRF 或增加 `Access-Control-Allow-Origin: *`。
- 检查网关当前能获得哪些可信会话信息。如果 webd 只转发用户 key 而不转发会话标识，不能声称已实现逐会话绑定：须在可信代理内部建立可验证的会话上下文，并剥离外部伪造的同名元数据，或明确只提供 actor+key 生命周期绑定的限制，待评审通过后再发布。不得直接信任客户端新造的 session header。
- challenge 限定签发时的 actor/有效认证上下文；登录失效后拒绝原 verify。重新登录后通过 read/public 核实原资金操作的最小状态，不返回签名或私密证明，不能因旧会话失效而无法核实结果。
- 网关管理员权限只允许访问网关，并不授权他替任意公钥扣款；即使管理员持有 `client_user_key`，也仍需该资产账户对精确 terms 的签名。

## 9. 上游账本与持久化适配

### 9.1 先查清再实施的上游事项

- [x] 上游仓库、commit、运行版本、部署入口、API 前缀、数据库及实际账本身份填写到交付记录。
- [x] 合法 K1 公钥在不加入 NNI、不产生 device 记录的情况下零余额查询和首次收款建账。
- [x] 资金账户以资产公钥为 owner，不要求关联设备；原设备与 owner 绑定关系保持独立。
- [x] Core 验证原 signing_payload 与签名，网关不重造签名内容；固定向量和 HTTP 验证通过。
- [x] 原子交易、持久化幂等键、精确费用/最低输出约束与可恢复状态查询均已实现并测试。
- [x] operation、challenge 与 receipt 映射由 Core 持久化；双连接并发测试只有一次扣款。

### 9.2 存储内容与事务

以下是逻辑字段要求，不是在主 runtime 数据库新增技能业务表。当前由 Core SQLite 统一持有 challenge、资金操作幂等和账本事务；网关只建立可信会话绑定并转发，不另外保存一份可能失真的金融状态。迁移与备份必须包括 Core 的这些元数据。

| 持久化对象 | 最少保存信息 | 约束 |
| --- | --- | --- |
| challenge | challenge UUID、原始 UTF-8 payload、摘要、nonce、有效期、actor/可信会话绑定、service、ledger、规范 node、account、operation UUID、读写类别、消费状态 | UUID 唯一；验签使用原字节；禁止反序列化再序列化替代原文 |
| 资金操作 | ledger、付款 account、operation UUID、意图摘要、绑定 challenge、service/node、内部状态、上游关联 ID、receipt、结果和时间 | 唯一键 `(ledger_id, account, operation_id)`；service/kind/参数也进入摘要，跨 service 重用 ID 不允许变成第二次扣款 |
| 账本交易与资产流 | 付款/收款 owner、两种资产的整数变动、实际费用、买卖方向、同笔事务关联、账本凭证、提交时间 | 余额、交易、资产流与最终结果在账本侧原子提交；不得先扣款再异步写去重结果 |

需要保存的原始签名证明只能进入权限受控的验证/恢复存储，并采用明确保留策略；不要写普通请求日志、错误日志或审计导出。绝不存私钥、钱包密码或客户端备份。

推荐内部状态为 `prepared → executing/pending → succeeded|failed|expired`，但**对客户端仍只输出第 4 节四种 status**。具体规则：

1. request 建立意图摘要及 challenge；活动或已执行的 operation ID 不同参数拒绝。重复相同 request 不得生成可以分别执行两次的授权。未执行且过期的 challenge 可清理，之后同 ID 必须重新签署全新 nonce；桌面始终生成新 UUID，不复用废弃 ID。已执行 ID 的终态索引不删除。
2. verify 使用事务 / 比较交换取得唯一执行权；已终态返回持久化结果；执行中返回 pending。并发请求不能各自向上游提交一笔。
3. 网关和上游之间无法组成单机事务时，先持久化提交意图，再使用上游同一个幂等键；崩溃后只核对原 ID，不生成新 ID“补单”。上游若不支持此能力必须先适配。
4. 扣款后连接断开、超时、网关或上游重启，结果先标为 pending，恢复任务查询账本事实；不能仅凭没有本地 receipt 判定失败。
5. 授权过期只禁止尚未执行的操作。已提交成功的操作即使 challenge 后来过期也仍返回 succeeded；已进入未知执行状态不能根据时间流逝改成 expired。
6. 去重记录保留期不能短于重放风险期；删除旧状态也必须保留足以拒绝重放的持久化终态索引。备份恢复不得让已扣款 ID 重新可执行。
7. 配置切换到不同节点后，不把旧 pending 操作送到新账本。当前桌面按所选服务查询状态，来源不匹配时应提示恢复原服务/节点配置后核实；支持跨旧来源恢复需要额外协商合同。

### 9.3 金额和历史数据转换

- 全过程用受检查的整数/定点数；乘 bps 时先提升到足够宽的整数再除，明确向下取整。不得经过 JS number 或浮点币值中转。
- 买入记录 USD 扣款、AIC 收入；卖出记录 AIC 扣款、USD 收入；转账分别记录付款及收款账户的资产变动。Bancor 和转账费用执行语义按第 3.3 节，不能沿用与之冲突的旧节点计费方式。
- owner/history 的余额快照和记录从相同账本读取；使用稳定排序（提交时间加唯一序号等），每页最多 20 条；空账本总页数为 1。页码保持请求值，不能悄悄返回第 1 页。
- v1 `operation_id` 必须是非空 UUID。对于通过旧浏览器产生、只有非 UUID transaction ID 的历史记录，应提供稳定、持久化的 UUID 映射或明确的命名空间映射；同一交易多条资产流共用同一个 UUID，不能每次查询随机生成。
- counterparty 无适用对象时返回 null，不能用 device_pubkey 或无效占位公钥。所有 *_units 字段严格输出十进制字符串，零输出 `"0"`。

## 10. 与统一页面相关的能力边界

0.3.2 的桌面本地账号与硬件账号共用资产总览、资产列表、转账表单、Bancor 行情/K 线、标准/SWAP 布局和账户选择位置，公钥复制保持可用。统一页面不会将两种账户的认证方式混用。

**当前必须实现的 v1：** 第 4 节余额及资产变动记录、分页、操作状态；公开 market/candles/trades 继续使用现有接口，但必须允许已登录且没有签名硬件的客户端读取，且 market.node_url 与所选 Bancor capabilities 来源一致。

**v1 没有的字段：** 流水总条数、source/direction 服务端筛选、完整成对成交明细、memo/费用明细。当前本地账号页面对不支持的筛选明确禁用，展示真实返回记录；不能通过取一页再本地过滤来冒充全量筛选。

若要求流水功能与网页完全一致，作为后续独立协议升级实现：

| 扩展内容 | 后端与桌面须共同定义 |
| --- | --- |
| source / direction / asset 筛选 | 枚举、默认值；所有筛选条件进入 intent/terms 并签名，不能 verify 后再替换过滤参数 |
| 完整分页 | `total_count`、`per_page`、稳定排序/游标规则、快照一致性及最大页大小 |
| 单笔转账详情 | 原始 transaction/receipt ID、收付款方、amount、fee、memo、最终状态和时间 |
| 完整 Bancor 成交 | side、input/output 资产及实际 units、fee、价格含义、关联 receipt；同笔交易两条资产流不能显示成两笔成交 |

客户端使用 `deny_unknown_fields`，**不得直接在现有 v1 JSON 中添加这些字段**。先定义版本化能力/接口及桌面类型与验证器升级，同时保留已安装 v1 客户端。先完成现有合同闭环，扩展不应阻塞基础余额、买卖和转账。

## 11. 错误、限流和兼容性验收

第 4 节已列出的错误码是当前客户端专门映射的合同。下表其余码是后端实现建议，不表示客户端已有专门文案；采用前应与桌面错误映射同步。v1 错误信封仍用第 3 节结构，不新增成功响应字段。

| 场景 | HTTP / error 建议 | 要求 |
| --- | --- | --- |
| 未登录 / 无权访问 | 401 / 403，沿用认证错误 | 不签发 challenge、不访问私有账户 |
| 不支持独立 owner 协议 | 501；旧版本无路由可为 404/405 | 桌面识别 unsupported；不能返回假余额或硬件账户数据 |
| 非法字段、金额、公钥、intent | 400 / `asset_owner_request_invalid` | 拒绝重复及未知字段；执行前无副作用 |
| 签名不正确或与 challenge 不一致 | 403 / `asset_owner_signature_invalid` | 不暴露其他账户的 challenge 内容 |
| 过期且明确未执行 | 409 / `asset_owner_challenge_expired` | 已执行或未知结果按真实状态返回 |
| 余额不足 | 409 / `asset_owner_insufficient_balance` | 返回明确未执行；不能部分扣款 |
| 同 ID 不同请求 | 409 / `asset_owner_operation_conflict` | 原操作结果及余额不被覆盖 |
| 账本或配置来源变化 | 409 / `asset_owner_context_changed` | 要求重新获取能力并重新确认，不能改原 payload |
| 限流 / 上游不可用 | 429 / 503，稳定机器错误码 | 不输出内部 key/URL 凭据；写结果未知时仍保留 pending |

后端设置并记录管理员/会话/公钥维度的 challenge 速率上限、未消费 challenge 总量上限、每账户并发资金操作上限、过期回收策略与上游超时。客户端总请求预算 15 秒、响应最多 256 KiB；后端应在预算内给出确定结果或 pending，不让客户端无限等待。

必须新增的验收矩阵（全用隔离测试资产）：

| 用例 | 通过标准 |
| --- | --- |
| 完全无硬件的网关，未加入 NNI，本地公钥 B | 登录后可读 capabilities，B 真实零余额；不能报设备公钥/签名芯片缺失 |
| 网关绑定 A，但使用桌面 B/C | B/C 均能完成独立读写；A 绑定公钥、设备加入状态和奖励记录不变 |
| B 首次收款 → 余额 → B 买入 → 卖出 → 向 C 转账 | 余额、费用、资产流、receipt 相互核对；C 入账；A 没有非预期扣款 |
| B/C 连续切换、同一公钥多客户端、两个网关同账本 | 返回数据不串账户；并发同 operation ID 只产生一次账本写入 |
| 重新登录后查询原 pending | 按明确公钥和原 operation_id 免密读取公开结果；不能用另一公钥查到属于原账户的 receipt，也不返回签名证明 |
| 所有请求字段篡改与编码边界 | 覆盖中文 memo、8 位小数、上限/溢出、high-S、错误哈希、DER、重复字段、过期和跨读写重放 |
| 扣款已提交但响应丢失、进程崩溃、主从切换、备份恢复 | 结果最终一致且不重复扣款；不将未知状态当失败 |
| 标准/SWAP 页面、K 线最大化、资产转账、两类账户公钥复制 | 页面位置及布局保持；桌面本地写操作仅调用 owner 路由，不回落到旧硬件 trade/transfer |
| 旧网页硬件委托/绑定 owner 私钥路径及历史筛选 | 原有合同与权限保持；旧接口测试全部通过 |
| 浏览器局域网 HTTP、本机 loopback HTTP、局域网可信 HTTPS、公网域名 HTTPS | 既有 HTTP 入口仍可访问；HTTPS 正常验链与域名，登录/CSRF 正常，无私钥出站 |

## 12. 后端 agent 交付要求

- [ ] 提交网关与上游各自的代码位置、commit、迁移脚本、实际部署版本；明确哪些组件需要升级，不只更新 UI。
- [ ] 提交 5 个新 API 的严格 schema、测试样例、签名向量测试结果、真实隔离账本的读写与故障恢复证据。
- [ ] 交付持久化幂等、状态恢复、清理保留期及备份恢复说明，证明多实例和重启不会重复扣款。
- [ ] 记录实际开放的 actions、规范 node_url、稳定 ledger_id、各环境是否有硬件、TLS 入口和 HTTP 回归结果；不在文档写 token 或私钥。
- [ ] 在 Ubuntu 已安装客户端上创建全新 B/C 账号完成联调；仅本仓库 fixture 通过不能标记“后端已支持”。Windows/macOS 共用同一协议，无需按 OS 分支鉴权。
- [ ] 按本地修改 → 验证 → 提交推送 → 目标机拉取已提交版本 → 部署的顺序交付，保留现有 HTTP/HTTPS 配置。回滚先停止新资金请求，保留并核实 pending，不能回滚数据库丢弃去重记录。
- [ ] 最终回填本文件开头状态和第 6 节任务复选框，区分“已实现”“已测试”“已部署”；尚未支持的能力不得对客户端宣告可用。

建议给后端 agent 的任务说明：**先读完本文件和桌面 `protocol.rs` / `client.rs`，定位真实账本服务，按 P0–P6 完成已安装客户端的独立资产账号适配。保留硬件账号及浏览器访问行为；无需上传私钥、无需加入 NNI、无需替换设备绑定。实现、真实测试、部署三个状态分别给证据。**

## 13. 联合实施记录（2026-09-11）

| 层 | 已实现位置与责任 |
| --- | --- |
| Core | `/home/guagua/NNI/nni_server/owner_financial/{protocol,service,ledger,routes}.mjs`：严格输入、原文验签、容量/频率限制、延迟建账、转账和 Bancor 共用现有账本事务、不可变操作凭证 |
| Edge | `/home/guagua/NNI/deploy/nginx/nni-edge.conf.template`：新 owner 写接口进入 asset_transfer 限流；读取进入 bancor_private；不缓存私有结果，不重试写入 |
| 网关 | `crates/clawd/src/http/ui_routes/nni_owner_financial.rs`、`nni_owner_response.rs`：管理员准入、仅选配置节点、禁止重定向、10 秒上游超时、256 KiB 类型化响应投影 |
| webd | `crates/webd/src/main.rs`、`crates/claw-core/src/owner_gateway_context.rs`：保留登录和 CSRF，剥离外部伪造内部头，签署当前会话、请求方法/路径/正文摘要和时间 |
| 桌面 | `desktop/src/asset_operations/`、`desktop/frontend/wallet/`：严格核对能力/记录/回执，区分限流、费用变化、市场不可交易、冻结等错误；未知结果保留待核实，不自动再次扣款；历史收支符号按资产和买卖方向显示 |
| 网站 | `/home/guagua/matrixai-web-security-worktree-20260807`：已核对现有 Explorer 公共查询与后台私网 mTLS 链路；新交易进入同一账本投影，无需增加账户注册、数据库副本或公开管理员代理 |

**新生成账号不落库：** 创建/恢复密钥完全在桌面。公开查询不存在的账户返回零，不创建资产账户、owner、device 或绑定记录；首次真实收款与开户在同一个资金事务里完成。已发生过业务的零余额账户不能删除其历史。临时 challenge 独立限额和过期清理，不等于注册账号。

**启用条件：** Core 新增锁定的 `jsonc-parser` 依赖，部署前在 `nni_server` 执行 `npm ci --omit=dev --ignore-scripts`。`NNI_SERVER_OWNER_NODE_URLS` 未配置时，新接口返回 501，不影响旧硬件接口。配置只接受明确的规范节点来源；实际生产值不写入测试 fixture。

**已验证：** 固定签名字节、中文 memo、严格字段和金额边界、1000 个不同公钥的零余额签名查询不增长账户表、真实 Core HTTP、首次收款后独立转账/买卖、并发两连接只有一次扣款、重启后查询原结果、精确费用/最少输出/溢出拒绝、事务回滚、Explorer 投影、网关权限/重定向/响应投影、桌面协议与页面单测、网站原有管理权限回归。

本地 Core 全量回归 292 项通过，部署后的 Core 隔离 HTTP 测试 3 项通过。Ubuntu 已安装桌面
验收记录位于 `desktop/test-results/wallet-native-e39f5566/acceptance.json`：8 组流程，隔离账本
6 次真实 K1 资金签名验证、16 次无密码公开读取，另含真实网关/两个 Edge/Core 联调。
桌面 23 项单元测试、HTTP/HTTPS/SSH/权限集成、11 项前端测试、类型检查和发布编译通过。
网站前端 37 项、后端 37 项通过；部署后 Explorer 节点切换、记录分类/方向筛选、商品保留、
未登录管理员拒绝和后台 mTLS 读取通过。仓库长文件门禁仍有 12 个未触及模块的既有问题。

**当前密码行为：** 按用户确认后的方案，余额、历史、原操作结果走 read/public，密钥库
锁定或账号未备份时也可查看。买卖/转账在原生安全窗口逐笔输入密码，已解锁管理会话也
不能跳过验证；签完即清除解密密钥。错误密码保留本次确认但不提交请求。创建、备份、
恢复仍需管理解锁。密码不会发送到 webd、Edge、Core 或网站。

**实际部署：** Core `28ac26b8`；两个 Edge 更新 owner 路由限流分类；本机网关/桌面业务源码
`b4b2ef8e8`；网站前后端与保留源码 `27a26af`。Assets 实测走 api-2，Bancor 走 api-1，均返回
账本 `d3ed23ff-d827-41a7-a755-7b7614ac6269`。Core 升级前已备份，在生产快照副本上完成迁移
幂等、完整性与账本核对，升级后再次核对成功。生产新测试公钥的 owner、账户、账本、
challenge、操作表均为零行；没有为本轮测试增加 USD 或操作真实余额。

**剩余专项：** 混合心跳/交易容量测试、全链路杀进程故障注入、陈旧备份恢复防重放演练、
真实硬件与独立账户之间的生产资金联调。本轮未运行 macOS/Windows 实机验收。原生夹具、
实际 Core 隔离 HTTP、生产只读联调是三种不同证据，不能互相替代，也不能声称抵御所有攻击。

**灾备边界：** 数据库恢复前必须封锁资金写入，保留并核对恢复点之后已提交的凭证。不能恢复陈旧快照、继续使用旧 ledger_id 就宣称防重复扣款；当前不具备自动检测陈旧恢复或跨数据库共识的能力。完整运维与限制见上游 `owner_financial/README.md`。
