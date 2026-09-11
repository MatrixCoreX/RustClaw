# 162 的独立 HTTPS 入口

日期：2026-09-11。目标为 `xuhao@192.168.31.162`，Intel Mac。

## 访问与配对

- 原 HTTP：`http://192.168.31.162`。
- 新 HTTPS：`https://192.168.31.162:8443`。
- Bonjour 地址：`https://xuhaodeMacBook-Pro.local:8443`。
- 本机配对材料：`desktop/connections/192.168.31.162/ca.crt` 与 `CA-SHA256.txt`；只包含公开 CA。
- CA SHA-256：`0087D6E23393D4145E9BBAB38B1AD69AFDA9C0F42F38A27A34DE78AA1A1304E9`。

设备普通用户无权绑定 443，使用 8443 避免获取管理员密码或修改原有服务。发现广播携带实际端口，桌面端已找到该 HTTPS 候选；发现不会自动授予信任，选择公开 CA 并核对指纹后才能连接。浏览器仍可直接访问原 HTTP，浏览器 HTTPS 的证书信任独立配置。

## 部署边界

配置工具在 `desktop/deploy/macos_https/`，先在本机完成配置注入 / 网络边界测试和 Nginx 语法检查，再提交推送。只同步该提交的工具到远端隔离目录，并逐文件比较 SHA-256；未改动远端已有源码工作区。

初始部署提交为 `256556ab6cf51b9c8458190533f68e83522396d3`；当前 HTTPS 配置的本地提交与实际部署提交均为 `33b8d5ebfec3937b9063007d886dcf07000a5276`。配置更新前比较旧摘要并保存备份，只重载独立 HTTPS 进程，保留原证书。

- 运行配置和 TLS 材料位于目标用户的 `~/.agent-runtime/desktop-https/`，目录 0700、私钥 0600；私钥在 Mac 本地生成并保留。
- 使用现有 Homebrew Nginx / OpenSSL。独立 Nginx 进程仅监听该 LAN 地址和回环地址的 8443，访问规则仅允许 `192.168.31.0/24`、IPv4 / IPv6 回环。
- TLS 1.2 / 1.3，禁用会话票据，TLS 1.2 仅使用 ECDHE 与 AEAD 密码套件。转发客户端地址、协议由入口覆盖，拒绝无关 Host。
- UI 直接读取现有静态目录，API 直接代理本机 `127.0.0.1:8788`，保留原账户与 CSRF 认证。不读取凭据、不登录真实账户、不执行业务操作。
- 新增用户 LaunchAgent `org.agent-runtime.desktop-https` 与 `org.agent-runtime.desktop-discovery`，分别负责 HTTPS 与 Bonjour。随该用户登录启动，退出登录后不可用；未配置系统级服务。
- 没有修改 HTTP 配置，没有 reload 原 Nginx，没有重启 `clawd` / `webd`，没有 HTTP 重定向或 HSTS。HTTP 继续保持原来的明文传输行为。

## 验证

证据位于 `desktop/test-results/host-162-https/`。

- HTTP、HTTPS 均返回 200，HTML 内容完全一致；与部署前 HTTP 摘要一致：`44e4628cd944dc5f30e8829514e9de1e739bb5d479521f4d40e5e93bbbe2dc13`。
- 原 Nginx 主配置摘要前后均为 `93a11ec044f973b89df6c644b50ae9055ca43c6993a2118f95936011bc2c8bb9`；原 HTTP 站点摘要均为 `a68e29844605b11c2f88d2f919f6c0f667ec9c326c06629f3ea03a5a16d9d208`。
- 原 Nginx、`clawd`、`webd` 的 PID 保持 55892、59811、59843。
- 使用取回的公开 CA 验证 IP、DNS 主机名及匿名 session 接口成功；未信任 CA 时 TLS 失败，错误 Host 返回 400。
- 代理保留 Host 中的 8443 端口。带真实桌面 Origin 的 IP / DNS session 请求通过，错误端口 Origin 被拒绝；空登录输入到达表单校验且不触发账户验证。没有 Cookie 的匿名退出响应设置 Secure 清除 Cookie，未操作任何已有登录会话。
- 原生发现模块真实返回该 Mac 的 HTTPS 8443 候选，同时仍可找到 Pi 和当前电脑的本机服务。
- 产品身份检查、双品牌 UI 构建、跨平台与存储合同检查通过。

## 后续维护

IP 变化或证书到期需要更新监听地址和证书；工具遇到已有配置会明确停止，避免覆盖已有信任。服务器证书有效期 397 天，CA 有效期 10 年。

需要暂停本次入口时，只对上述两个用户 LaunchAgent 执行 `launchctl bootout gui/$(id -u)/<label>`；不操作原 Homebrew Nginx 服务。保留证书、配置、部署来源和既有构建产物。再次启用需使用对应 plist 执行 `launchctl bootstrap`。
