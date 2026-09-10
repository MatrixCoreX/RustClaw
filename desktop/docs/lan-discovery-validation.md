# HTTPS 与局域网发现（0.1.1）

本次桌面源代码改动全部位于 `desktop/`。没有修改服务端、网页 UI、根 workspace 或已有渠道配置。远端未部署或修改源码，仅按用户授权新增 Nginx / Avahi 运行配置。

## Pi 配置

- 设备：`192.168.31.243`，主机名 `claw.local`，Raspberry Pi 5 / Linux aarch64。
- 新增 `/etc/nginx/conf.d/agent-desktop-https.conf`，使用现有 `/etc/agent-runtime/tls/lan.pem` 与 `lan-key.pem`。HTTPS 443 与原有 HTTP 80 并存，没有重定向或 HSTS。
- HTTPS 站点复用现有静态 UI / WEBD 路由；允许 RFC1918 IPv4、loopback 和 IPv6 ULA / link-local 客户端，其余来源拒绝。转发的客户端 IP 由 Nginx 覆盖，不能由请求方注入。
- 原有 80 端口配置 SHA-256：`3f7d55110fd15b55041eab266853f430c946386d556b85b6cd554d71b7b9c78a`。配置后文件摘要、HTTP 网页内容摘要均不变，HTTPS 返回相同网页。`clawd` / `webd` 进程未重启，WEBD 仍只监听 `127.0.0.1:8788`。
- CA 通过已验证 SSH 获取，仅保存公开证书。CA SHA-256：`30FA687DA30327AF9C1F48B19CC9D116F86584D0C0E01476A4FC57FA71BE88CA`。未导出私钥、未修改客户端系统 / 浏览器根证书。
- 新增 `/etc/avahi/services/agent-desktop-https.service`，内容来自本目录的 `../deploy/agent-desktop-https.service`。Avahi 自动加载且日志确认服务发布成功，无需重启。
- `curl --cacert` 验证证书链、IP 与 HTTPS `/webd/session` 均成功。没有使用忽略 TLS 验证的参数，没有登录实际账户或提交业务操作。

原 HTTP 按要求保留，其传输仍为明文。新增桌面连接只走 HTTPS / SSH；若浏览器也要安全登录，应另行信任公开 CA 后使用 HTTPS。

## 发现边界与验证

- 打开添加设备自动浏览 DNS-SD；主动扫描只在用户点击时执行。当前 IPv4 物理局域网范围最多 508 个目标 / 24 并发 / 20 秒，扫描过程与登录会话彻底分离。
- 实机扫描完成 253 个目标，找到 Pi 的 `https://claw.local:443` 广播，并识别到另一台未广播、仅可 SSH 连接的设备。广播与扫描候选按 IP / 端口合并，不会重复保存信任。
- Rust 测试覆盖私网与子网边界、总量上限、恶意 / 不兼容广播拒绝、匿名结构化识别、响应大小上限、HTTP 跳转目标零连接、Cookie / 认证信息不发送、取消等。
- 原生 UI 的实机验证可显式设置 `DESKTOP_TEST_LAN_HOST` 和 `DESKTOP_TEST_LAN_CA`，运行 `python3 tests/native_e2e.py /usr/bin/agent-desktop`。默认测试仍使用隔离本机夹具；启用 LAN 验证也只执行发现和匿名 HTTPS 连通检查，使用隔离 profile，不登录真实账户。
- 详细记录在 `test-results/pi-https/` 与 `test-results/native/report.json`。当前验证平台为 Ubuntu 26.04 x86_64；macOS 未验证发行，Apple target 检查受当前 Linux 环境缺少 Apple SDK 限制。

## 本次检查结果

- 桌面 TypeScript 测试 6 项通过；发现 Rust 测试 6 项通过，原有 HTTPS / SSH / 策略测试继续通过。桌面与共享 UI 类型检查通过，双品牌 UI 构建通过。
- 安装包实际 WebKitGTK 验证共 15 项通过，包括真实 Pi 的广播 / 扫描 / 取消 / HTTPS 连接，以及原有 16 个管理页面、流式上传、SSE、视频、AiAPP 权限隔离等。本次没有登录真实 Pi 账户执行业务操作。
- 产品名耦合自测 / inventory、跨平台静态检查、MCP 合同检查均通过。
- 仓库已有 12 个服务端超长文件与 4 个历史运行时语言字面量仍导致相应全仓门禁失败；均位于 `desktop/` 之外，本次未修改，也未新增发现项。Apple target 因 Linux C 编译器不支持 Apple SDK 编译选项而失败，不能据此声明 macOS 已通过。

## 本机交付

- 已安装并打开 `0.1.1`，本机配置中已添加 `Pi · claw`，地址 `https://claw.local`。通过已核验 SSH 取得的公开 CA 完成应用内配对，原生 HTTPS 连通验证成功。没有保存账号密码或登录状态，也未修改其他设备配置。
- 最终安装包：`releases/agent-desktop_0.1.1_amd64.deb`。SHA-256：`00881e659b384f2dcad8a67f35b01b73098a657a1ab6a22748dc4cfdc8c576a5`。安装后的二进制与包内二进制完全一致；最终安装产物再次通过全部 15 项原生检查。
- 0.1.0 原始安装包与本次中间验证包均保留，没有删除 Cargo target / 缓存。

## 恢复方法

只有需要撤回本次新增入口时，移除本次新增的上述两个 `/etc` 配置文件，并在 `nginx -t` 通过后重载 Nginx。原 HTTP 配置不需要恢复，已有证书和设备数据保留。不要调用会覆盖原 HTTP 站点的全量恢复脚本。
