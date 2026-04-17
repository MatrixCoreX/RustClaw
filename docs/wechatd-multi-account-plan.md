# `wechatd` 多账号改进计划

## 背景

当前 `wechatd` 已经在单账号模式下对齐了 `@tencent-weixin/openclaw-weixin` 的大部分收发行为，包括：

- `getupdates` 长轮询
- `sendmessage` 文本/图片/视频/文件
- `getconfig` / `sendtyping`
- `context_token` 约束与回退
- 引用消息 / 引用媒体解析
- SILK 语音转 WAV 的单账号链路

但账号层仍然是单实例模型：

- 一个 `session.json`
- 一个 `get_updates_buf.txt`
- 一个 `run/wechatd-status/primary.json`
- 一个轮询任务
- 一个默认 `"primary"` 登录槽位

这和官方插件的多账号模型不一致。官方插件支持：

- 多个账号条目同时存在
- 每账号独立 token / baseUrl / userId
- 每账号独立长轮询
- 每账号独立状态与上下文 token 缓存

## 目标

把 `wechatd` 从“单账号 daemon”升级为“单进程多账号 daemon”，在不破坏现有单账号可用性的前提下，对齐官方插件的账号模型。

## 非目标

- 第一阶段不改微信协议本身
- 第一阶段不改 `clawd` 的 channel binding 语义
- 第一阶段不做 UI 大改，只提供可用接口和状态结构
- 第一阶段不追求和官方插件目录结构完全一致，只追求能力对齐

## 目标能力

完成后应支持：

- 扫码登录多个微信账号
- 多账号同时在线轮询
- 每账号独立保存登录态
- 每账号独立保存 `get_updates_buf`
- 每账号独立状态文件
- 每账号独立 `context_token` 缓存
- 每账号独立 `typing_ticket` 缓存
- `clawd` 主动发送时可指定账号

## 当前架构问题

### 1. 状态是单例

当前 `State` 中这些字段都只有一份：

- `status`
- `session`
- `session_path`
- `sync_buf_path`
- `config_cache`

这导致只能服务一个在线账号。

### 2. 登录流程写死为 `primary`

`login_qr_start()` 当前默认：

- `session_key = "primary"`

这使得并发登录和多账号登录都无法成立。

### 3. 文件路径是单账号路径

当前文件路径固定为：

- `data/wechatd/session.json`
- `data/wechatd/get_updates_buf.txt`
- `run/wechatd-status/primary.json`

多账号下必须拆成账号维度。

### 4. 轮询任务只有一个

当前只有一个：

- `tokio::spawn(monitor_wechat_loop(state.clone()))`

多账号后必须变成“每账号一个 poll loop”。

### 5. `context_token` 与配置缓存未完全账号隔离

虽然 `context_token` 已经开始带账号 key，但运行时仍然依赖单 session 读取当前账号。

## 推荐目标结构

建议把账号维度抽成独立运行单元。

### 1. 运行时模型

新增：

- `AccountRuntime`
- `WechatRuntimeRegistry`

建议结构：

- `AccountRuntime`
  - `account_id: String`
  - `session: PersistedSession`
  - `status: WechatRuntimeStatus`
  - `status_path: PathBuf`
  - `sync_buf_path: PathBuf`
  - `config_cache: WeixinConfigManager`
  - `poll_task_handle`
- `WechatRuntimeRegistry`
  - `accounts: HashMap<String, AccountRuntime>`
  - `active_logins: HashMap<String, ActiveLogin>`

### 2. 文件布局

建议改为：

- `data/wechatd/accounts/index.json`
- `data/wechatd/accounts/<account_id>.json`
- `data/wechatd/sync_buf/<account_id>.txt`
- `run/wechatd-status/<account_id>.json`

兼容读取：

- 若新路径不存在，则回退读取旧的 `data/wechatd/session.json`
- 首次成功登录后写入新路径

### 3. 登录模型

建议：

- `session_key` 不再固定为 `"primary"`
- 新登录默认使用随机 key
- 已存在账号的重登录可允许传 `account_id`

接口层建议保留兼容字段，并增加可选字段：

- `LoginStartRequest.account_id: Option<String>`
- `LoginWaitRequest.account_id: Option<String>`

### 4. 状态接口

当前 `/healthz` 与 `/login/status` 返回的是单账号视角。

建议升级为：

- `/healthz`
  - 返回整体摘要 + 账号列表
- `/accounts/:account_id/status`
  - 返回单账号状态
- `/login/status`
  - 保留兼容，默认返回“最近活动账号”或“主账号摘要”

## 推荐实施阶段

### Phase 1. 路径与数据结构解耦

目标：

- 把“账号相关路径”从全局单例改为按 `account_id` 生成
- 引入账号索引文件

工作项：

- 新增路径 helper
  - `wechat_account_file_path(workspace_root, account_id)`
  - `wechat_sync_buf_file_path(workspace_root, account_id)`
  - `wechat_runtime_status_file_path(workspace_root, account_id)`
  - `wechat_account_index_path(workspace_root)`
- 新增账号索引读写
- 保留旧路径兼容读取

验收：

- 单账号旧数据仍可正常启动
- 新登录账号会写到账号维度新路径

### Phase 2. 登录接口支持多账号

目标：

- 一个 `wechatd` 进程能管理多个登录态

工作项：

- `ActiveLogin` 增加
  - `account_id: Option<String>`
  - `target_base_url: Option<String>`
- `login_qr_start()` 支持：
  - 新账号登录
  - 指定账号重登录
- `login_qr_wait()` 成功后：
  - 写入 `accounts/<account_id>.json`
  - 更新账号索引
  - 不再覆盖全局单 session

验收：

- 能连续扫两个不同微信号
- 两个账号凭证都能落盘

### Phase 3. 每账号独立轮询

目标：

- 多账号同时在线

工作项：

- 把 `monitor_wechat_loop(state)` 改为
  - `monitor_wechat_account_loop(state, account_id)`
- 启动时从账号索引加载全部有效账号
- 每账号各自维护：
  - token
  - base_url
  - `get_updates_buf`
  - `config_cache`
  - runtime status

验收：

- 两个账号能同时收消息
- 一个账号失效不会拖垮另一个账号

### Phase 4. 消息处理链路带账号

目标：

- 入站与出站都严格绑定账号

工作项：

- `WeixinMessage` 处理时附带 `account_id`
- `context_token` key 固化为 `(account_id, user_id)`
- `typing_ticket` cache 也以账号隔离
- `submit_wechat_task_and_reply()` / `deliver_wechat_clawd_reply()` 加 `account_id`

验收：

- A 账号收到的消息不会错误使用 B 账号的 token / context token

### Phase 5. `clawd` 主动发送链路支持选账号

目标：

- `clawd` 主动发微信时可以指定账号，而不是默认唯一账号

工作项：

- 扩展 `WechatSendConfig`
- 主动发送时读取目标 `account_id`
- 默认策略：
  - 若未指定账号且只有一个账号，沿用该账号
  - 若未指定账号且多账号存在，返回明确错误

验收：

- 定时任务 / 主动通知可显式选择账号发送

## 兼容策略

为了减少一次性破坏，建议保留以下兼容行为：

- 若只有旧的 `session.json`，启动时自动映射为单账号
- 若系统中只有一个账号，旧接口行为尽量不变
- 旧的 `/login/status` 继续可用
- 单账号配置文件不强制新增字段

## 风险点

### 1. `clawd` 身份绑定没有账号维度

当前微信绑定使用：

- `external_user_id = from_user_id`
- `external_chat_id = from_user_id`

多账号后，可能需要变成：

- `external_user_id = <account_id>:<from_user_id>`

否则不同账号上同一个微信用户 ID 可能发生绑定冲突。

第一阶段可以先不改，只要业务接受“同用户跨账号共享绑定”。

### 2. 状态接口兼容复杂

原来前端可能默认只有一个 `primary.json`。  
多账号后需要确认是否允许：

- 保留一个聚合 `primary.json`
- 或前端改为读取账号列表

### 3. 主动发送默认账号选择

单账号时代可以隐式发送；多账号时代若不显式指定账号，行为会变得不确定。

建议默认严格报错，而不是猜测账号。

## 测试计划

至少验证：

1. 旧单账号数据可正常启动
2. 新扫第一个账号成功
3. 新扫第二个账号成功
4. 两个账号同时长轮询
5. 两个账号都能收文本并回复
6. 两个账号都能收图/文件/语音
7. `context_token` 不串账号
8. 一个账号 session 过期不影响另一个
9. `clawd` 主动发送在单账号下兼容
10. `clawd` 主动发送在多账号下未指定账号时返回明确错误

## 推荐落地顺序

建议按下面顺序推进：

1. 路径与账号索引
2. 登录接口多账号化
3. 多 poll loop
4. 入站/出站链路带 `account_id`
5. `clawd` 主动发送支持选账号
6. 前端 / 状态接口整理

## 当前结论

现在的 `wechatd` 适合作为：

- 单账号稳定实现
- 多账号改造的基础版本

下一次继续时，建议从 `Phase 1 + Phase 2` 一起开始，不要直接跳到多 poll loop。
