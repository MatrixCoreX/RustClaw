# Manual Source Builds / 技术人员手动构建

This is a developer workflow, not an installation fallback. Deployment agents
and ordinary users must use [Release packages](release_installation.md).
本页仅供明确需要修改源码的技术人员使用；安装部署请使用
[Release 包](release_installation.zh-CN.md)，不得自动退回源码编译。

## Toolchain / 工具链

Use the repository's pinned Rust toolchain, Python 3.11+, Node.js 22/npm,
Clang/libclang, and protoc. Additional skill adapters can require Go or other
runtimes. Inspect before explicitly updating tools:

```bash
bash scripts/build_toolchain_manager.sh check
# Optional developer action, not part of installation:
bash scripts/build_toolchain_manager.sh update
```

## Build and Verify / 构建与验证

Work in a source checkout, preserve local changes, and use the current branch's
tests. Build scripts adapt concurrency to available memory; do not increase
parallelism blindly on a Pi or other small host.

```bash
git status --short
./build-all.sh
# Core-only rebuild when verified UI assets already exist:
./build-all.sh no-ui
# Link already-built programs; this step does not compile:
bash install-agent-cmd.sh --user --no-deploy-ui
```

For UI development, run `npm ci`, `npm run dev`, `npm run lint`, and
`npm run build` inside `UI/`. A UI development server is not a production
deployment. Source-only UI updates use `build-ui.sh`; do not run this as part of
installing a prebuilt Release.

普通构建不主动编译 Skill Store 的按需技能。正式发行流程显式预编译匹配平台的
按需包，校验收据后随发行包交付。开发者可以按明确范围构建单个技能。
跨平台脚本位于 `scripts/archive/cross-build/`，仅由维护者和发布 CI 显式调用。
保留最近构建缓存，不要在每次构建后执行 `cargo clean`。

## Explicit Cache Cleanup / 显式清理缓存

Cache retention remains the normal workflow. Clear local compilation caches
only when explicitly requested; deployment programs and installed skills are
not cache targets. Preview the known Cargo compilation files before deleting:

```bash
python3 scripts/clean_dev_artifacts.py
python3 scripts/clean_dev_artifacts.py --apply
# Select a relocated Cargo target root explicitly; never follow a target symlink:
python3 scripts/clean_dev_artifacts.py --target-dir /path/to/cargo-target
```

The root must have a valid `CACHEDIR.TAG`, and a profile must have an owned
regular `.cargo-lock`. The cleaner refuses active builders/busy profiles and
changed file identities. It only removes recognized compilation metadata,
objects and incremental records; executables, shared libraries, unknown files,
configuration, databases, logs, test evidence and symlinks are preserved.
All selected roots are admitted before deletion begins. Linux and macOS use
descriptor-relative, no-follow access; unsupported platforms fail explicitly.
This is not a runtime-data retention or factory-reset command.

默认保留缓存；只有明确要求清理时才使用 `--apply`。脚本先预览已识别的 Cargo 编译文件，
不会清除运行程序、动态库、配置、数据库、日志、测试证据或未知文件。符号链接不会作为
删除授权；迁移过的构建根目录需要显式指定。正在编译、锁被占用、文件身份变化或平台
不支持时明确拒绝，不会退回删除运行数据的方式。这也不是恢复出厂设置功能。

## Publishing / 发布

Before committing configuration, run `bash scripts/check-secrets.sh --staged`.
The lightweight guard checks immutable Git-index blobs, not unstaged worktree
contents. Diagnostics show only an escaped filename, line number and rule;
matching values are never printed. Empty indexes do not fall back to scanning
local configuration. Explicit file/directory arguments opt into a worktree
configuration check. `--install` installs a pre-commit hook only if no hook
already exists. Unreadable, oversized, non-UTF8 or non-regular configuration
inputs fail explicitly. This heuristic guard is not a complete source secret
scanner: publication still requires a redacted scan of the exact candidate
sources, including non-configuration files.

提交前可运行 `bash scripts/check-secrets.sh --staged`。轻量检查器读取暂存区固定对象，
不读取尚未暂存的修改；输出只包含转义后的文件名、行号和规则，不打印密钥原文。
空暂存区不会转而扫描本地配置。只有显式提供路径时才检查工作区文件；安装钩子不会
覆盖已有钩子。不可读、超大、非 UTF-8 或非常规配置输入明确失败。此检查仅是配置
启发式防护，发布前仍需对精确候选源码执行覆盖其他文件类型的脱敏密钥扫描。

Release CI builds binaries and UI on dedicated runners, prepares locked browser
runtime dependencies, checks contracts, precompiles platform-compatible Skill
Store packages, and signs the archive manifest. `package-release.sh` refuses
missing UI assets or embedded credentials; it must not package live local
configuration, credentials, wallet keys, or mutable runtime data.

The reusable `release-python-wheels.yml` workflow prepares and verifies locked
Python 3.13 and 3.14 dependencies on matching native publisher runners. Native
extensions are built there, never during UI installation. Download both workflow
artifacts into one directory and provide it as
`APP_RELEASE_PYTHON_WHEELS_ROOT` when invoking `package-release.sh` manually.
Packaging verifies each bundle's target, source lock digest, and wheel hashes.
Full bundled-skill packages also require the native, relocatable runtime roots
selected by the release builder through `APP_RELEASE_PYTHON_RUNTIME_ROOT` and
`APP_RELEASE_NODE_RUNTIME_ROOT`. The package script copies them into the signed
archive, materializes symlinks, and executes the relocated runtimes before
signing. They are release dependencies, not user-installed prerequisites.

See the [Linux x86_64](ubuntu_x86_64_release.md) and
[Pi aarch64](pi_aarch64_release.md) publishing guides. The macOS artifact workflow
builds each native architecture from an exact source commit. Validate all five
release assets before publication and only then remove older same-platform
releases. Source build instructions remain separate from user installation.
