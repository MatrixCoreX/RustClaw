#!/usr/bin/env bash
set -euo pipefail

OS_NAME="$(uname -s)"

print_section() {
  printf '\n%s:\n' "$1"
}

echo "操作系统信息:"
if command -v lsb_release >/dev/null 2>&1; then
  lsb_release -a
elif [[ "$OS_NAME" == "Darwin" ]]; then
  sw_vers
else
  uname -a
fi

print_section "内核版本"
uname -r

print_section "CPU 信息"
if command -v lscpu >/dev/null 2>&1; then
  lscpu | awk -F: '/Model name/ {print $2}' | xargs
elif [[ "$OS_NAME" == "Darwin" ]]; then
  sysctl -n machdep.cpu.brand_string
else
  echo "CPU 信息不可用"
fi

print_section "内存使用情况"
if command -v free >/dev/null 2>&1; then
  free -h
elif [[ "$OS_NAME" == "Darwin" ]]; then
  vm_stat
else
  echo "内存信息不可用"
fi

print_section "磁盘使用情况"
df -h

print_section "网络接口状态"
if command -v ip >/dev/null 2>&1; then
  ip addr show
elif command -v ifconfig >/dev/null 2>&1; then
  ifconfig
else
  echo "网络接口信息不可用"
fi

print_section "当前运行的服务"
if command -v systemctl >/dev/null 2>&1; then
  systemctl list-units --type=service --state=running
elif [[ "$OS_NAME" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
  launchctl list
else
  echo "服务列表不可用"
fi
