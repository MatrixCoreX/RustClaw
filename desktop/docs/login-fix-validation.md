# 0.1.2 登录修复

## 原因

Pi 的 `/webd/login` 请求返回 HTTP 200，随后没有发出 `/v1/auth/me` 请求。桌面端 `Session::login` 错把 CSRF 令牌限制为 64 位十六进制；实际 WEBD `new_csrf_token()` 使用 UUID simple 格式，`WEBD_CSRF_TOKEN_HEX_BYTES` 为 32，网页端也只接受 32 位小写十六进制。

因此用户名密码验证已经成功，客户端却返回 `csrf_missing`，未进入已登录状态。先前的 Rust / Python 测试夹具同样误用了 64 位，导致旧的测试通过却没有覆盖真实合同。这是桌面端适配与测试建模错误。

## 修改

- 桌面登录按实际合同严格接受 32 位小写十六进制 CSRF，缺失与非法格式分别报告；没有删除 CSRF 校验，也没有放宽到任意长度。
- 后续写请求仍由原生层绑定当前连接的 Cookie / CSRF，网页不能注入认证 header。TLS 证书指纹继续是 64 位 SHA-256，未受修改影响。
- Rust 夹具改为实际 32 位合同，Python 原生 UI 夹具改为 UUID4 hex。新增共用合法 / 非法样例，同时对照实际网页 normalization 与 WEBD 常量，防止夹具与客户端一起偏离真实合同。
- 只修改 `desktop/`；没有修改 Pi 服务、账号、认证设置、HTTP / HTTPS 配置，没有读取用户密码。

## 回归证据

- 修复前：将夹具纠正为 32 位后，原 HTTPS 登录测试稳定复现 `Err("csrf_missing")`。记录：`test-results/login-fix/regression-before.log`。
- 修复后：Rust 单元与 HTTPS / SSH 协议测试通过，覆盖登录、Cookie / CSRF 后续写请求、密钥登录、证书 / 主机名拒绝、设备隔离、发现等。记录：`test-results/login-fix/rust-tests.log`。
- TypeScript 测试 7 项与类型检查通过，其中新增用例直接对照实际网页 CSRF helper 与服务端常量。
- 产品身份、storage ownership 与跨平台静态检查通过。全仓长文件门禁仍有既存 12 项服务端问题，未修改这些文件。

真实 Pi 的用户名密码仍由用户在桌面端输入；测试使用独立夹具，不读取或重放用户密码。此前的真实 Pi 连通检查仅证明 HTTPS / 匿名接口可用，不能替代真实账户登录验收。

## 安装产物验证

已安装并打开 0.1.2，安装后的二进制与包内二进制一致。最终原生 WebKitGTK 检查 16 项通过，新增检查通过可见的用户名密码表单输入测试账户、点击登录并进入共享控制台，使用真实合同的 32 位 CSRF；其余 HTTPS / SSH、发现、上传、SSE、视频与 AiAPP 隔离检查继续通过。

安装包 SHA-256：`735621e06eabfea8240acc0a711a8bb8e05e04e2cacc8919b8e646c0258fda72`。原设备配对、证书和其他资料均保留。
