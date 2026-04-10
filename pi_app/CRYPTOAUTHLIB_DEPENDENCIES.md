# Pi App Cryptoauthlib 依赖说明

本文档说明在 `pi_app` 里使用 `cryptoauthlib` 需要哪些依赖，以及哪些依赖只是开发/重编时才需要。

## 当前方案

当前 `pi_app` 已内置最小运行时到：

- `pi_app/vendor/cryptoauthlib/python/cryptoauthlib/`
- `pi_app/vendor/cryptoauthlib/build-pyfix/lib/libcryptoauth.so*`

这意味着：

- 运行 `pi_app` 时，通常不需要单独 `pip install cryptoauthlib`
- 也不需要依赖 `/home/pi/cryptoauthlib` 这份源码仓库才能让主流程工作
- `pi_app` 会优先使用本地 vendored 运行时

## 运行时依赖

要让 `pi_app` 正常调用安全芯片，至少需要：

### 1. Python

- `python3`

当前机器实测版本：

- `Python 3.13`

### 2. I2C 设备访问

必须满足：

- 系统已启用 I2C
- 存在设备节点 `/dev/i2c-0`
- 当前运行用户有权限访问 `/dev/i2c-0`

如果这几点不满足，`cryptoauthlib` 即使库能加载，也无法真正和 ATECC608 通信。

### 3. vendored 动态库

`pi_app` 依赖以下本地库文件：

- `pi_app/vendor/cryptoauthlib/python/cryptoauthlib/libcryptoauth.so`
- 或 `pi_app/vendor/cryptoauthlib/build-pyfix/lib/libcryptoauth.so`

其中 Python 包目录里的 `.so` 主要用于让 `cryptoauthlib` 在 import 阶段就能成功加载。

### 4. Python 标准库

`signature.py` 目前只依赖 Python 标准库中的：

- `ctypes`
- `hashlib`
- `json`
- `os`
- `sys`
- `time`

这些都随 `python3` 自带，无需额外安装。

## 推荐安装的系统包

如果你是在一台新机器上部署 `pi_app`，推荐至少安装：

```bash
sudo apt update
sudo apt install -y python3 i2c-tools
```

说明：

- `python3`：运行 `pi_app` 和 `signature.py`
- `i2c-tools`：不是运行时硬依赖，但非常建议安装，便于用 `i2cdetect` 排查总线和地址问题

## 调试时常用检查

### 检查 I2C 设备节点

```bash
ls /dev/i2c-0
```

### 检查是否能看到芯片地址

```bash
sudo i2cdetect -y 0
```

对于这块板子：

- 文档常写 7-bit 地址 `0x35`
- `cryptoauthlib` Linux HAL 中脚本里使用 8-bit 写法 `0x6A`

### 检查 pi_app 当前使用的是哪份动态库

```bash
python3 /home/pi/rustclaw/pi_app/signature.py tng_signer_cert
```

输出 JSON 里的 `lib_path` 可用于确认当前加载的是哪一个 `.so`。

## 什么时候还需要 `/home/pi/cryptoauthlib`

一般运行 `pi_app` 不再强依赖它。

但以下场景仍可能需要：

- 你要重新编译 `libcryptoauth.so`
- 你要升级 `cryptoauthlib` 版本
- 你要修改上游 Python binding 或 C 源码
- 你要重新生成 vendored 运行时

## 从源码重编时需要的依赖

如果你要从源码重新构建 `cryptoauthlib`，建议安装：

```bash
sudo apt update
sudo apt install -y build-essential cmake pkg-config python3 python3-venv i2c-tools
```

这些依赖主要用于：

- `build-essential`：编译 C 代码
- `cmake`：生成构建脚本
- `pkg-config`：辅助构建环境探测
- `python3-venv`：需要时创建 Python 虚拟环境
- `i2c-tools`：构建后调试设备通信

## 服务端示例的额外依赖

如果你要运行证书链/验签示例目录：

- `pi_app/tng_server_example/`

则还需要安装：

```bash
python3 -m pip install -r /home/pi/rustclaw/pi_app/tng_server_example/requirements.txt
```

当前额外依赖为：

- `cryptography`

注意：

- 这是服务端示例依赖
- 不是 `pi_app` 访问 ATECC608 的基础运行时依赖

## 最小结论

如果你的目标只是让 `pi_app` 正常使用安全芯片：

1. 安装 `python3`
2. 启用 I2C，并确保 `/dev/i2c-0` 可访问
3. 保留 `pi_app/vendor/cryptoauthlib/`

这样通常就够了。
