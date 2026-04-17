# Pi App TNG/签名接入说明

本文档说明 `pi_app` 里已经预埋好的安全芯片能力，以及服务端应如何接入。

独立的服务端证书链/验签示例放在 `pi_app/tng_server_example/`。

`cryptoauthlib` 运行时/构建依赖说明见 `pi_app/CRYPTOAUTHLIB_DEPENDENCIES.md`。

适用硬件：

- M5Stack `Unit ID`
- 芯片型号 `ATECC608B-TNGTLSU-G`
- 文档标称 I2C 地址 `0x35`（7-bit）；在 `cryptoauthlib` Linux I2C HAL 中实际使用 `0x6A`（8-bit 写法）

参考资料：

- [M5Stack Unit ID](https://docs.m5stack.com/en/unit/id)

## 当前已预埋能力

`pi_app/signature.py` 现已支持以下 helper action：

- `pubkey`
  - 读取当前业务使用的设备公钥
- `sign_timestamp <unix_time>`
  - 对时间戳字符串的 `sha256` 做芯片内签名
- `tng_device_pubkey`
  - 读取 TNG 设备公钥
- `tng_device_cert`
  - 读取 TNG 设备证书（DER，hex 编码返回）
- `tng_signer_cert`
  - 读取 TNG signer 证书（DER，hex 编码返回）
- `tng_root_cert`
  - 读取 TNG 根证书（DER，hex 编码返回）

`pi_app/small_screen_cryptoauth_service.py` 现已提供对应包装函数：

- `read_slot0_pubkey_via_helper()`
- `sign_unix_time_via_helper(unix_time)`
- `read_tng_device_pubkey_via_helper()`
- `read_tng_device_cert_via_helper()`
- `read_tng_signer_cert_via_helper()`
- `read_tng_root_cert_via_helper()`

## 当前机器上的验证结论

已验证成功：

- I2C 通信正常
- `slot 0` 公钥可读
- 时间戳签名可用
- `tng_device_pubkey` 与 `slot 0` 公钥一致
- `tng_signer_cert` 可读
- `tng_device_cert` 可读
- `tng_root_cert` 可读

补充说明：

- 当前这块板子识别为 `TNGTLS template_id=3`
- `tng_device_cert` 走“库内部自动从设备读取 signer 公钥”的分支时仍可能触发 `-16`
- `pi_app/signature.py` 已改为先读取 `signer_cert` 再显式传给底层 API，因此实际 helper 调用已稳定返回设备证书

## 推荐接入方式

推荐优先采用：

- 激活码绑定
- 芯片公钥登记
- 请求签名验签

原因：

- 当前 `pi_app` 已稳定打通这条链路
- 不依赖 TNG 证书链作为主流程前提
- 服务端实现简单
- 当前若希望接标准证书链，也已经具备升级条件

## 推荐服务端流程

### 1. 激活阶段

设备上报：

- `activation_code`
- `device_pubkey`
- `timestamp`
- `signature`

服务端执行：

1. 校验 `activation_code` 是否有效、未使用、属于当前渠道/用户
2. 校验 `timestamp` 是否在允许窗口内，例如 300 秒
3. 计算 `sha256(str(timestamp))`
4. 用设备上报的 `device_pubkey` 验证 `signature`
5. 验证成功后，建立绑定关系并作废激活码

建议保存字段：

- `device_id`
- `owner_user_id`
- `activation_code_id`
- `device_pubkey_hex`
- `device_pubkey_fingerprint`
- `chip_type`
- `activated_at`
- `last_seen_at`
- `status`

### 2. 后续鉴权阶段

设备上报：

- `device_id`
- `timestamp`
- `signature`

服务端执行：

1. 根据 `device_id` 查出绑定时保存的 `device_pubkey_hex`
2. 校验 `timestamp` 防重放
3. 重新计算 `sha256(str(timestamp))`
4. 用保存的公钥验签
5. 验签成功后放行业务请求

## 可选的 TNG 证书模式

当前 `device_cert / signer_cert / root_cert` 已可从 `pi_app` 读取；若你希望走更标准的证书链模式，可扩展为：

设备额外上报：

- `device_cert_hex`
- 可选：`signer_cert_hex`

服务端额外执行：

1. 用内置 `root_cert` 或信任锚校验证书链
2. 从设备证书中提取公钥
3. 再用证书中的公钥验签
4. 再检查这张证书或其公钥是否已绑定到你的设备表

注意：

- 证书链解决的是“这是真设备”
- 激活码/设备表绑定解决的是“这是不是你的设备”

因此即使上了 TNG 证书，服务端仍需要自己的设备绑定表。

## 最小接口设计

推荐至少实现两个接口。

### `POST /v1/device/activate`

请求体：

```json
{
  "activation_code": "ACT-XXXX-XXXX",
  "device_pubkey_hex": "<128 hex chars>",
  "timestamp": 1712740000,
  "signature_hex": "<128 hex chars>"
}
```

成功响应：

```json
{
  "ok": true,
  "device_id": "dev_xxx",
  "bound": true
}
```

### `POST /v1/device/auth`

请求体：

```json
{
  "device_id": "dev_xxx",
  "timestamp": 1712740000,
  "signature_hex": "<128 hex chars>"
}
```

成功响应：

```json
{
  "ok": true,
  "authenticated": true
}
```

## 服务端验签示意

伪代码：

```python
digest = sha256(str(timestamp).encode("utf-8")).digest()
public_key = load_p256_public_key_from_raw_xy(device_pubkey_hex)
verify_ecdsa_sha256_raw_rs(public_key, digest, signature_hex)
```

说明：

- 芯片返回的是 `R || S` 形式的 64 字节原始签名
- 你的服务端验签库如果要求 ASN.1 DER，需要先把原始 `R/S` 转成 DER

## 设备端调用示例

### 读取设备公钥

```python
from small_screen_cryptoauth_service import read_slot0_pubkey_via_helper

pubkey_hex, error = read_slot0_pubkey_via_helper()
```

### 生成时间戳签名

```python
from small_screen_cryptoauth_service import sign_unix_time_via_helper

payload, error = sign_unix_time_via_helper(1712740000)
```

### 尝试读取 TNG 根证书

```python
from small_screen_cryptoauth_service import read_tng_root_cert_via_helper

payload, error = read_tng_root_cert_via_helper()
```

## 当前建议

现阶段请按以下优先级实施：

1. 先上“激活码 + 公钥登记 + 签名验签”
2. 如需标准化设备身份链路，可直接接入 TNG 证书链验签
3. 即使启用 TNG 证书模式，也仍建议保留激活码/设备绑定表
