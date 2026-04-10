# TNG Server Example

这个目录放一个独立的服务端示例，演示如何：

- 校验 `device_cert -> signer_cert -> root_cert` 证书链
- 从 `device_cert` 提取设备公钥
- 校验设备上报的时间戳签名

## 文件

- `verify_tng_chain.py`
  - 命令行示例脚本
- `requirements.txt`
  - 仅示例目录需要的 Python 依赖

## 安装依赖

```bash
python3 -m pip install -r requirements.txt
```

## 运行方式

```bash
python3 verify_tng_chain.py \
  --device-cert-hex "<device_cert_hex>" \
  --signer-cert-hex "<signer_cert_hex>" \
  --root-cert-hex "<root_cert_hex>" \
  --timestamp 1712740000 \
  --signature-hex "<64-byte raw rs hex>"
```

## 输入说明

- `device_cert_hex`
  - 来自 `read_tng_device_cert_via_helper()`
- `signer_cert_hex`
  - 来自 `read_tng_signer_cert_via_helper()`
- `root_cert_hex`
  - 来自 `read_tng_root_cert_via_helper()`
- `timestamp`
  - 设备参与签名的 Unix 时间戳
- `signature_hex`
  - 芯片返回的 `R || S` 原始签名，长度应为 128 个 hex 字符

## 成功输出

脚本成功时会输出一段 JSON，包含：

- `chain_verified`
- `signature_verified`
- `device_public_key_hex`
- `device_subject`
- `device_serial_number`

## 说明

- 这个示例假设设备签名内容是 `sha256(str(timestamp))`
- 芯片输出的是原始 `R || S`，脚本会自动转换成 DER 后再验签
- 即使证书链验证通过，服务端仍建议保留自己的设备绑定表
