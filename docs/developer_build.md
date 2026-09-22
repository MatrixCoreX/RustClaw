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

## Publishing / 发布

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

See the [Linux x86_64](ubuntu_x86_64_release.md) and
[Pi aarch64](pi_aarch64_release.md) publishing guides. The macOS artifact workflow
builds each native architecture from an exact source commit. Validate all five
release assets before publication and only then remove older same-platform
releases. Source build instructions remain separate from user installation.
