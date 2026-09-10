# 桌面客户端

独立的 Tauri 2 桌面工程，用于连接已经运行服务的设备。客户端复用 `../UI/src` 的页面，不运行远端服务、模型或技能。

已验证的安装版本为 **Ubuntu 26.04 x86_64 0.1.4 内部测试版**。0.2.0 增加 Windows x64、macOS Apple 芯片与 Intel 的原生构建和打包支持；对应原生构建、安装与测试的完成状态以 [跨平台验收记录](docs/cross-platform-validation.md) 为准。Ubuntu 包声明 GLIBC 2.39+、GTK 3、WebKitGTK 4.1 和音视频依赖，Ubuntu 22.04/24.04 尚未验证。

## 安装和使用

双击 [Ubuntu 安装包](releases/agent-desktop_0.1.4_amd64.deb)，使用 Ubuntu 软件安装器安装。安装后从应用菜单打开所选产品名称对应的桌面控制台。日常使用不需要 Rust、Node 或终端。

1. 选择“添加设备”，给设备取一个易懂的名称。
2. 选择 HTTPS 或 SSH，两种方式都通向相同的管理页面。
3. 建立加密连接后，使用设备上的用户名密码或用户 Key 登录。
4. 顶部始终显示当前设备、连接方式和登录角色。重启、部署、技能安装等操作作用于这台远端设备。
5. “切换设备 / 断开”关闭本机连接，并清除当前页面状态。设备任务继续执行。

**HTTPS**：输入完整 origin，例如 `https://device.local:8443`。系统已信任的证书使用正常验证。私有证书需要选择公开 CA 证书，并输入从设备本地或已可信入口核对的完整 SHA-256 证书指纹。不导入系统根证书，不接收 TLS / CA 私钥，不跳过主机名和有效期验证。

**SSH**：输入主机、SSH 端口、用户名、远端 WEBD 端口，以及核对过的 `SHA256:…` 主机公钥指纹。支持密码和用户选择的 SSH 私钥文件。连接只建立到设备 `127.0.0.1:WEBD端口` 的 direct-tcpip 通道，不执行远端命令，不开放 SSH 本地代理端口。SSH 登录和应用账户登录分别完成。

“保存到系统凭据库”默认不勾选。勾选后，应用账户凭据使用 Linux Secret Service、Windows Credential Manager 或 macOS Keychain；锁定或不可用时提供本次会话继续使用的入口，不写入明文配置。SSH 密码和私钥仅本次使用。忘记设备会删除其本机信任、保存登录及页面缓存，不删除设备账户和数据。

打开“添加设备”会自动查找 `_agent-runtime._tcp.local.` DNS-SD 广播，约 4 秒。点击“扫描局域网”会同时进行有界 IPv4 查找；可随时停止。它仅扫描物理有线 / Wi-Fi 接口当前所在的至多 /24 范围，最多 508 个地址、24 个并发、20 秒；不会扫描公网、VPN 或所有端口。匿名读取 80 端口的 `/webd/session` 合同，匹配后才检查 443 或 22 端口，分别提供 HTTPS 或 SSH 候选。没有广播的 HTTP 设备也可能被找到；自定义端口、仅 HTTPS、IPv6-only、跨 VLAN 或隔离网络请使用 DNS-SD / 手工地址。没有 HTTP 探测匹配时，不把普通 SSH 主机列为设备。

广播和扫描结果始终是待验证地址。不会自动接受证书、导入系统根 CA、保存登录或发送账号。选择结果后，仍需通过设备本地或已核验 SSH 获取公开 CA / 主机指纹。广播里的私有 CA 字段只用于显示配对表单，不能建立信任。

设备端 Avahi 声明示例在 `deploy/agent-desktop-https.service`，端口须对应已验证可用的 HTTPS 服务。安装声明仅发布主机名、端口和协议类型，不发布账号、指纹、证书或私钥。桌面安装器不自动修改设备端服务。HTTPS 与原有 HTTP 并存的部署和验收说明见 [LAN 验证记录](docs/lan-discovery-validation.md)。

文件上传使用系统选择器与分块传输。保存附件时选择电脑上的目标位置，下载使用临时文件、长度校验和原子完成，支持进度与取消。音视频通过仅监听本机回环地址、带独立临时授权的媒体通道播放，远端流量继续加密。不会自动打开下载文件。动态 AiAPP 在独立受限窗口中打开，其 bridge 由设备的 manifest 合同和服务端权限继续约束。

0.1.4 将资产页、交易页的默认账号标签统一为“硬件设备绑定账号”，与网页共用同一处文案。

0.1.3 修复了页头遮挡导航和 Linux 原生下拉框在深色主题下显示白底的问题；见 [界面修复记录](docs/layout-fix-validation.md)。

0.1.2 修复了用户名密码登录时的 CSRF 令牌格式不匹配；原因与回归验证见 [登录修复记录](docs/login-fix-validation.md)。

## 开发

Windows / macOS 的依赖、构建、安装、权限与签名说明见 [平台指南](docs/windows-macos.md)。

所有桌面源码、配置、测试和构建输出入口在本目录，Cargo workspace、锁文件与服务端独立。共享页面由 `scripts/shared-ui-adapter.ts` 在桌面构建时适配；它不改写、不复制 UI 业务源码，已知入口发生变化时构建会明确失败。

```bash
cd desktop
node scripts/check-dependencies.mjs
npm ci --prefix ../UI
npm ci
npm run lint
npm test
cargo test --no-default-features
npm run dev
./target/debug/agent-desktop
npm run build
```

开发依赖：Rust、Node 20.19+、GTK 3、WebKitGTK 4.1 开发包、OpenSSL 开发包及 Tauri 所需 Linux 库。Ubuntu 可通过 `apt install libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev librsvg2-dev` 安装。这里只描述开发机；安装包用户不需要编译依赖。

可以设置 `CARGO_TARGET_DIR` 将缓存放到空间充足的位置。不要删除已有 target / 编译缓存。`npm run build` 的 `.deb` 位于相应 target 的 `release/bundle/deb/`。

产品展示只读取 `../configs/product_identity.toml`，或由 `APP_PRODUCT_IDENTITY_CONFIG` 选择同 schema 文件。原生构建复用仓库 `ProductIdentity` 验证器，UI 构建复用原 UI 配置。包名、可执行文件、菜单文件、数据目录与系统凭据库 namespace 都使用中性名称，改展示名称不会改变连接资料身份。

构建入口通过独立的小型 Rust helper 调用相同的身份验证器，Windows 不需要借助 Bash 解析产品配置。它的锁文件与 target 缓存位于桌面工程内；macOS 显示名和权限清单由配置生成。

## 验证

```bash
python3 scripts/inventory.py
python3 tests/native_e2e.py
```

原生测试需 `WebKitWebDriver`、`xvfb-run`、Python、OpenSSL、ffmpeg 和位于 `.build/tools/bin/tauri-driver` 的测试驱动；可使用 `cargo install tauri-driver --locked --root .build/tools` 安装。测试只连接自建 loopback TLS/SSH 协议夹具，使用隔离数据目录，不操作真实设备或资产。测试驱动不进入发行安装包。

功能来源清单见 [docs/feature-inventory.md](docs/feature-inventory.md)，安全合同见 [docs/security.md](docs/security.md)，本机验证与限制见 [docs/ubuntu-validation.md](docs/ubuntu-validation.md)。

## 升级与卸载

安装更新的同名 `.deb` 即可更新本机客户端。客户端更新不会升级或重启设备。内部测试版采用手工安装包更新；签名自动更新 feed 尚未启用，远端设备无法指定客户端更新包。

从 Ubuntu 应用管理中卸载桌面客户端不会影响远端服务。需要删除连接资料时，先在客户端逐个“忘记设备”。程序数据采用 `org.agent-runtime.desktop` 应用目录，密钥保存在系统凭据库，不应只删除 JSON 文件来代替忘记设备。
