#!/usr/bin/env bash
# zh: 输出当前机器的基础系统诊断信息，方便部署前后排查环境问题。
set -euo pipefail

OS_NAME="$(uname -s)"

print_section() {
# zh: 统一打印报告分区标题。
  printf '\n%s:\n' "$1"
}

# zh: 以下运行时文本保持英文，便于日志和远程排障统一检索。
echo "Operating system information:"
if command -v lsb_release >/dev/null 2>&1; then
  lsb_release -a
elif [[ "$OS_NAME" == "Darwin" ]]; then
  sw_vers
else
  uname -a
fi

print_section "Kernel version"
uname -r

print_section "CPU information"
if command -v lscpu >/dev/null 2>&1; then
  lscpu | awk -F: '/Model name/ {print $2}' | xargs
elif [[ "$OS_NAME" == "Darwin" ]]; then
  sysctl -n machdep.cpu.brand_string
else
  echo "CPU information unavailable"
fi

print_section "Memory usage"
if command -v free >/dev/null 2>&1; then
  free -h
elif [[ "$OS_NAME" == "Darwin" ]]; then
  vm_stat
else
  echo "Memory information unavailable"
fi

print_section "Disk usage"
df -h

print_section "Network interface status"
if command -v ip >/dev/null 2>&1; then
  ip addr show
elif command -v ifconfig >/dev/null 2>&1; then
  ifconfig
else
  echo "Network interface information unavailable"
fi

print_section "Running services"
if command -v systemctl >/dev/null 2>&1; then
  systemctl list-units --type=service --state=running
elif [[ "$OS_NAME" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
  launchctl list
else
  echo "Service list unavailable"
fi
