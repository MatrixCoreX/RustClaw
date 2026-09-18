# Android client

Android 工程复用 `desktop/frontend` 和 Rust 设备连接、资产、Bancor 协议。平台代码位于
`desktop/src/android`，布局及构建入口位于本目录；不修改网页 UI 或服务端。

目标：Android 8.0 / API 26 以上、Chromium WebView 111 以上；ARM64、ARMv7 和 x86_64；手机、平板、折叠屏、横竖屏。
最终实际验证范围记录在对应安装包的 `manifest.json`，不能把目标范围当作实测结果。

## 构建

安装 JDK 21、Android SDK 36、Build Tools 36、NDK r28c 或更新的兼容版本，安装对应 Rust targets。
设置 `JAVA_HOME`、`ANDROID_HOME`、`NDK_HOME`；可以独立指定 `CARGO_TARGET_DIR` 和
`GRADLE_USER_HOME`。不要删除已有构建缓存。

Tauri 需要 `desktop/gen/android` 指向 `desktop/android/project`。首次检出后在 desktop 执行：

```sh
mkdir -p gen
ln -s ../android/project gen/android
node android/build.mjs --target aarch64 armv7 x86_64
```

构建脚本通过仓库的 ProductIdentity schema 读取展示名，不在本工程维护第二套品牌配置。
发行签名从 `ANDROID_SIGNING_KEYSTORE` 和 `ANDROID_SIGNING_PASSWORD_FILE` 读取；默认位于
本目录的 `.signing/release.p12` 和 `.signing/password`。这些文件已忽略，不能提交或公开。
保留同一签名密钥才能给已经安装的 APK 原位升级。

当前 release 关闭 Java/Kotlin 代码裁剪，保留 JNI 和 AndroidX 接口，以便独立测试 APK
验证相同的发布程序。release 不启用调试、WebView 调试或测试桥；测试桥只存在于测试 APK。
这项选择不改变私钥加密、原生权限检查或签名进程保护。

## 验证

`inspect_apk.py` 检查实际 APK 的签名、三种 ABI、16 KB 对齐、非调试发行配置、私有组件和
备份限制，同时检查测试桥及签名私钥没有进入发行包。

`project/app/src/androidTest` 提供单独测试 APK；测试代码不会进入应用 APK。使用与 release
相同的本地签名配置构建 `assembleUniversalReleaseAndroidTest`。在一次性模拟器中安装两份
APK 后，分别运行 `NativeSecurityTest`、`WalletRoundTripTest`；后者输出的合成签名可用
`verify_signatures.py` 在主机独立验证。测试涵盖系统凭据加密、独立签名进程、跨进程文件锁、
协议拒绝、Android 备份在新密钥库中恢复、桌面 v1/v2 备份恢复及转账/Bancor 签名。

完整 UI 验收先启动 `UiHarnessTest`，再把测试模拟器的 TCP 8765 转发到主机 8766，运行：

```sh
python3 desktop/android/ui_acceptance.py --serial emulator-5556 --output desktop/test-results/android-ui
```

此脚本仅允许 `emulator-*`，使用本机 HTTPS/资产协议夹具及随机生成的测试账户。它会修改
该模拟器的窗口尺寸、字体、剪贴板和 Downloads 测试文件；应从清空应用数据的一次性模拟器
开始，不能对个人账户设备执行。测试 APK 内的桥绑定 loopback，发行 APK 不包含此桥。

UI 检查覆盖中英文、双主题、6 种窗口尺寸、系统键盘、150% 字体、后台/锁屏撤销、系统
文件选择、复制、公钥签名交易、设备登录和导航。模拟器通过不代表所有厂商设备均已实测。

## 原生安全边界

- 设备 HTTPS、证书固定、SSH 和 loopback HTTP 策略复用桌面端；连接失败不降级为 HTTP。
- 签名库在未导出的 `:asset_vault` Service 中运行，不初始化界面。Binder 检查调用 UID；
  专用 socket 使用桌面端有界、顺序校验的协议。父进程 Binder 死亡会结束签名服务。
- 私钥沿用逐账号认证加密与密码保护。系统包装材料额外由 Android Keystore AES-GCM 加密，
  写入 `noBackupFilesDir`。系统是否提供硬件密钥保护由设备决定，不能声称所有设备都有 StrongBox。
- 密钥内存使用按系统页大小分配的保护页、mlock、MADV_DONTDUMP 和释放前清零。
  长期密钥不返回 WebView；签名必须校验交易意图和新输入的密码。
- 签名线程加装 seccomp，禁止新建 IP socket、exec 和跨进程内存接口；ART/Binder 线程保留
  Android 自身策略。这不是整个 Service 的网络沙箱，也不是独立 UID。
- 应用后台、锁屏及超时会锁定账号；系统文件选择期间保留必要的流程，锁屏仍优先。
- 禁用云备份和设备迁移，并开启安全窗口。加密导出走系统文件选择器，写入后读回验证，
  验证完成后才把账号标记为已备份。没有明文私钥导出入口或广泛存储权限。

Java/JNI 和输入控件可能产生短期密码副本；这不能防御已 root 的系统、恶意系统组件或
已经控制本应用主进程的攻击者。签名密钥、APK 签名、余额、交易和恢复必须分别实测。

参考：
- [Tauri Android 前置条件](https://v2.tauri.app/start/prerequisites/#android)
- [Android Keystore](https://developer.android.com/privacy-and-security/keystore)
- [16 KB 内存页兼容](https://developer.android.com/guide/practices/page-sizes)
- [大屏与自适应布局](https://developer.android.com/develop/ui/views/layout/responsive-adaptive-design-with-views)
