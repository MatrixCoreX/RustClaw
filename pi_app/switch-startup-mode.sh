#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-status}"
PI_APP_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
SERVICE_NAME="rustclaw-small-screen-headless.service"
SERVICE_SRC="${PI_APP_DIR}/systemd/${SERVICE_NAME}"
SERVICE_DST="/etc/systemd/system/${SERVICE_NAME}"
TARGET_USER="${SUDO_USER:-${USER}}"
TARGET_HOME="$(getent passwd "${TARGET_USER}" | cut -d: -f6)"

if [[ -z "${TARGET_HOME}" ]]; then
  echo "无法确定用户 ${TARGET_USER} 的 HOME，退出。"
  exit 1
fi

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "缺少命令: $1"
    exit 1
  }
}

need_cmd sudo
need_cmd raspi-config
need_cmd systemctl

sudo -n true >/dev/null 2>&1 || {
  echo "需要 sudo 权限（当前为非交互执行）。请先确保 sudo 可用。"
  exit 1
}

run_as_user() {
  sudo -u "${TARGET_USER}" -H bash -lc "$*"
}

ensure_service_installed() {
  if [[ ! -f "${SERVICE_SRC}" ]]; then
    echo "未找到服务模板: ${SERVICE_SRC}"
    exit 1
  fi
  sudo install -m 644 "${SERVICE_SRC}" "${SERVICE_DST}"
  sudo systemctl daemon-reload
}

set_desktop_mode() {
  run_as_user "\"${PI_APP_DIR}/enable-autostart.sh\""
  ensure_service_installed
  sudo systemctl disable --now "${SERVICE_NAME}" >/dev/null 2>&1 || true
  sudo raspi-config nonint do_boot_behaviour B4
  sudo systemctl set-default graphical.target >/dev/null
  echo "已切换到 desktop 模式：开机自动登录桌面，并自动启动小屏。"
  echo "建议执行: sudo reboot"
}

set_headless_mode() {
  run_as_user "\"${PI_APP_DIR}/disable-autostart.sh\""
  ensure_service_installed
  sudo raspi-config nonint do_boot_behaviour B1
  sudo systemctl set-default multi-user.target >/dev/null
  sudo systemctl enable --now "${SERVICE_NAME}"
  echo "已切换到 headless 模式：开机不登录，直接启动小屏服务。"
  echo "此模式没有桌面会话，无法“退回桌面”。"
  echo "建议执行: sudo reboot"
}

show_status() {
  local default_target autostart_state svc_enabled svc_active autologin
  default_target="$(systemctl get-default 2>/dev/null || true)"
  if [[ -f "${TARGET_HOME}/.config/autostart/rustclaw-small-screen.desktop" ]]; then
    autostart_state="enabled"
  else
    autostart_state="disabled"
  fi
  svc_enabled="$(systemctl is-enabled "${SERVICE_NAME}" 2>/dev/null || echo disabled)"
  svc_active="$(systemctl is-active "${SERVICE_NAME}" 2>/dev/null || echo inactive)"
  autologin="$(sudo awk -F= '/^autologin-user=/{print $2}' /etc/lightdm/lightdm.conf 2>/dev/null | head -n1)"
  autologin="${autologin:-disabled}"

  echo "mode-status"
  echo "  user: ${TARGET_USER}"
  echo "  default-target: ${default_target}"
  echo "  desktop-autostart: ${autostart_state}"
  echo "  headless-service-enabled: ${svc_enabled}"
  echo "  headless-service-active: ${svc_active}"
  echo "  lightdm-autologin-user: ${autologin}"
}

case "${MODE}" in
  desktop)
    set_desktop_mode
    ;;
  headless)
    set_headless_mode
    ;;
  status)
    show_status
    ;;
  *)
    echo "用法: $0 {desktop|headless|status}"
    exit 1
    ;;
esac
