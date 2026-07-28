# Archived Cross-Build Scripts

本目录保存当前部署流程暂不使用的四套交叉编译入口：

- `cross-build-pi.sh`
- `cross-build-upload.sh`
- `cross-build-upload-cloud.sh`
- `local-cross-build-upload-pi.sh`

它们不参与默认构建、安装或发布流程，但仍由脚本语法检查和 Skill Store
按需构建门禁扫描。重新启用前必须在目标 Linux/macOS 主机上复核交叉工具链、
远端地址、SSH 凭证、目标架构和产物回传目录。

These scripts are not part of the default build, install, or release flow. They
remain syntax-checked and covered by the on-demand Skill Store build guard. Before
reactivating one, verify its cross toolchain, remote host, SSH credentials, target
architecture, and artifact pullback paths on the intended Linux or macOS host.
