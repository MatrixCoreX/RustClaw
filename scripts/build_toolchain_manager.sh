#!/usr/bin/env bash
# Cross-platform build toolchain reporting, update, and minimum-version checks.

RUSTCLAW_MIN_RUST_VERSION="${RUSTCLAW_MIN_RUST_VERSION:-1.85.0}"
RUSTCLAW_MIN_CLANG_VERSION="${RUSTCLAW_MIN_CLANG_VERSION:-14.0.0}"
RUSTCLAW_MIN_PROTOC_VERSION="${RUSTCLAW_MIN_PROTOC_VERSION:-3.12.0}"
RUSTCLAW_MIN_NODE_VERSION="${RUSTCLAW_MIN_NODE_VERSION:-20.19.0}"
RUSTCLAW_MIN_NPM_VERSION="${RUSTCLAW_MIN_NPM_VERSION:-9.0.0}"

rustclaw_version_number() {
  local raw="${1:-}"
  printf '%s\n' "$raw" | sed -nE 's/^[^0-9]*([0-9]+(\.[0-9]+){0,3}).*$/\1/p' | sed -n '1p'
}

rustclaw_version_at_least() {
  python3 - "${1:-0}" "${2:-0}" <<'PY'
import re
import sys


def parts(value: str) -> tuple[int, ...]:
    match = re.search(r"\d+(?:\.\d+)*", value)
    if not match:
        return (0,)
    return tuple(int(part) for part in match.group(0).split("."))


actual = parts(sys.argv[1])
minimum = parts(sys.argv[2])
width = max(len(actual), len(minimum))
actual += (0,) * (width - len(actual))
minimum += (0,) * (width - len(minimum))
raise SystemExit(0 if actual >= minimum else 1)
PY
}

rustclaw_tool_version() {
  local command_name="$1"
  shift
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf '%s\n' "missing"
    return 0
  fi
  "$command_name" "$@" 2>&1 | sed -n '1p'
}

rustclaw_report_build_toolchains() {
  local host_os host_arch
  host_os="$(detect_host_os 2>/dev/null || printf '%s' "unknown")"
  host_arch="$(detect_host_arch 2>/dev/null || printf '%s' "unknown")"
  echo "Build toolchain report (${host_os}/${host_arch}):"
  echo "  rustc:  $(rustclaw_tool_version rustc --version)"
  echo "  cargo:  $(rustclaw_tool_version cargo --version)"
  echo "  clang:  $(rustclaw_tool_version clang --version)"
  echo "  protoc: $(rustclaw_tool_version protoc --version)"
  echo "  node:   $(rustclaw_tool_version node --version)"
  echo "  npm:    $(rustclaw_tool_version npm --version)"
}

rustclaw_check_version() {
  local label="$1"
  local command_name="$2"
  local minimum="$3"
  shift 3
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Error: required tool is missing: $command_name" >&2
    return 1
  fi

  local raw actual
  raw="$("$command_name" "$@" 2>&1 | sed -n '1p')"
  actual="$(rustclaw_version_number "$raw")"
  if [[ -z "$actual" ]] || ! rustclaw_version_at_least "$actual" "$minimum"; then
    echo "Error: $label $actual is below the required version $minimum." >&2
    return 1
  fi
  printf '  %-7s %s (minimum %s)\n' "$label:" "$actual" "$minimum"
}

rustclaw_validate_build_toolchains() {
  local include_ui="${1:-1}"
  local failed=0
  echo "Validating build toolchain minimum versions..."
  rustclaw_check_version "rustc" rustc "$RUSTCLAW_MIN_RUST_VERSION" --version || failed=1
  rustclaw_check_version "cargo" cargo "$RUSTCLAW_MIN_RUST_VERSION" --version || failed=1
  rustclaw_check_version "clang" clang "$RUSTCLAW_MIN_CLANG_VERSION" --version || failed=1
  rustclaw_check_version "protoc" protoc "$RUSTCLAW_MIN_PROTOC_VERSION" --version || failed=1
  if [[ "$include_ui" == "1" ]]; then
    rustclaw_check_version "node" node "$RUSTCLAW_MIN_NODE_VERSION" --version || failed=1
    rustclaw_check_version "npm" npm "$RUSTCLAW_MIN_NPM_VERSION" --version || failed=1
  fi
  return "$failed"
}

rustclaw_run_privileged() {
  if [[ "$(id -u)" == "0" ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    echo "Error: updating system toolchains requires root or sudo: $*" >&2
    return 1
  fi
}

rustclaw_package_installed() {
  local manager="$1"
  local package="$2"
  case "$manager" in
    brew)
      brew list --versions "$package" >/dev/null 2>&1
      ;;
    apt)
      dpkg-query -W -f='${Status}' "$package" 2>/dev/null | grep -q "ok installed"
      ;;
    dnf|yum|zypper)
      rpm -q "$package" >/dev/null 2>&1
      ;;
    pacman)
      pacman -Q "$package" >/dev/null 2>&1
      ;;
    apk)
      apk info -e "$package" >/dev/null 2>&1
      ;;
    *)
      return 1
      ;;
  esac
}

rustclaw_detect_package_manager() {
  local candidate
  for candidate in brew apt-get dnf yum zypper pacman apk; do
    if command -v "$candidate" >/dev/null 2>&1; then
      case "$candidate" in
        apt-get) printf '%s\n' "apt" ;;
        *) printf '%s\n' "$candidate" ;;
      esac
      return 0
    fi
  done
  return 1
}

rustclaw_update_rust() {
  if command -v rustup >/dev/null 2>&1; then
    echo "Updating the Rust stable toolchain..."
    rustup update stable
    rustup default stable
  else
    echo "rustup is unavailable; the existing system Rust toolchain will be retained."
  fi
}

rustclaw_load_nvm() {
  local nvm_script="${NVM_DIR:-$HOME/.nvm}/nvm.sh"
  if [[ ! -s "$nvm_script" ]]; then
    return 1
  fi
  # shellcheck source=/dev/null
  . "$nvm_script"
}

rustclaw_update_nvm_node() {
  if ! rustclaw_load_nvm; then
    return 1
  fi
  echo "Updating Node.js through nvm (latest LTS)..."
  nvm install --lts --latest-npm
  nvm use --lts
}

rustclaw_update_package_toolchains() {
  local include_ui="${1:-1}"
  local node_managed_by_nvm=0
  if [[ "$include_ui" == "1" ]] && rustclaw_update_nvm_node; then
    node_managed_by_nvm=1
  fi

  local manager
  manager="$(rustclaw_detect_package_manager 2>/dev/null || true)"
  if [[ -z "$manager" ]]; then
    echo "No supported package manager found; package toolchains were not updated."
    return 0
  fi

  local -a packages=()
  case "$manager" in
    brew)
      packages=(llvm protobuf)
      [[ "$include_ui" == "1" && "$node_managed_by_nvm" == "0" ]] && packages+=(node)
      echo "Refreshing Homebrew package metadata..."
      brew update
      local formula
      for formula in "${packages[@]}"; do
        if rustclaw_package_installed brew "$formula" \
          && [[ -n "$(brew outdated --formula "$formula" 2>/dev/null || true)" ]]; then
          echo "Updating Homebrew formula: $formula"
          brew upgrade "$formula"
        fi
      done
      ;;
    apt)
      packages=(clang libclang-dev protobuf-compiler)
      [[ "$include_ui" == "1" && "$node_managed_by_nvm" == "0" ]] && packages+=(nodejs npm)
      local -a installed_apt=()
      local package
      for package in "${packages[@]}"; do
        rustclaw_package_installed apt "$package" && installed_apt+=("$package")
      done
      echo "Refreshing apt package metadata..."
      rustclaw_run_privileged apt-get update -qq
      if [[ "${#installed_apt[@]}" -gt 0 ]]; then
        rustclaw_run_privileged apt-get install -y --only-upgrade "${installed_apt[@]}"
      fi
      ;;
    dnf|yum)
      packages=(clang llvm-devel libclang protobuf-compiler)
      [[ "$include_ui" == "1" && "$node_managed_by_nvm" == "0" ]] && packages+=(nodejs npm)
      local -a installed_rpm=()
      local package
      for package in "${packages[@]}"; do
        rustclaw_package_installed "$manager" "$package" && installed_rpm+=("$package")
      done
      if [[ "${#installed_rpm[@]}" -gt 0 ]]; then
        rustclaw_run_privileged "$manager" upgrade -y "${installed_rpm[@]}"
      fi
      ;;
    zypper)
      packages=(clang llvm-devel libclang protobuf)
      [[ "$include_ui" == "1" && "$node_managed_by_nvm" == "0" ]] && packages+=(nodejs npm)
      local -a installed_zypper=()
      local package
      for package in "${packages[@]}"; do
        rustclaw_package_installed zypper "$package" && installed_zypper+=("$package")
      done
      if [[ "${#installed_zypper[@]}" -gt 0 ]]; then
        rustclaw_run_privileged zypper --non-interactive update "${installed_zypper[@]}"
      fi
      ;;
    pacman)
      packages=(clang llvm protobuf)
      [[ "$include_ui" == "1" && "$node_managed_by_nvm" == "0" ]] && packages+=(nodejs npm)
      rustclaw_run_privileged pacman -Syu --needed --noconfirm "${packages[@]}"
      ;;
    apk)
      packages=(clang llvm-dev libclang protobuf)
      [[ "$include_ui" == "1" && "$node_managed_by_nvm" == "0" ]] && packages+=(nodejs npm)
      rustclaw_run_privileged apk update
      rustclaw_run_privileged apk upgrade "${packages[@]}"
      ;;
  esac
  hash -r
}

rustclaw_check_toolchain_updates() {
  local include_ui="${1:-1}"
  echo "Checking available toolchain updates (no changes will be made)..."
  if command -v rustup >/dev/null 2>&1; then
    rustup check || true
  fi

  local manager
  manager="$(rustclaw_detect_package_manager 2>/dev/null || true)"
  case "$manager" in
    brew)
      local -a packages=(llvm protobuf)
      [[ "$include_ui" == "1" ]] && packages+=(node)
      brew outdated "${packages[@]}" || true
      ;;
    apt)
      local -a packages=(clang libclang-dev protobuf-compiler)
      [[ "$include_ui" == "1" ]] && packages+=(nodejs npm)
      apt-cache policy "${packages[@]}" || true
      echo "apt candidates use the local package index; run --update-toolchains to refresh it."
      ;;
    dnf)
      local -a packages=(clang llvm-devel libclang protobuf-compiler)
      [[ "$include_ui" == "1" ]] && packages+=(nodejs npm)
      dnf check-update "${packages[@]}" || true
      ;;
    yum)
      local -a packages=(clang llvm-devel libclang protobuf-compiler)
      [[ "$include_ui" == "1" ]] && packages+=(nodejs npm)
      yum check-update "${packages[@]}" || true
      ;;
    zypper)
      zypper list-updates || true
      ;;
    pacman)
      pacman -Qu || true
      ;;
    apk)
      apk version -l '<' || true
      ;;
    *)
      echo "No supported package manager found for update discovery."
      ;;
  esac
}

rustclaw_manage_build_toolchains() {
  local mode="${1:-ensure}"
  local include_ui="${2:-1}"
  case "$mode" in
    ensure)
      ;;
    check)
      rustclaw_check_toolchain_updates "$include_ui"
      ;;
    update)
      rustclaw_update_rust
      rustclaw_update_package_toolchains "$include_ui"
      ;;
    report)
      ;;
    *)
      echo "Error: unsupported toolchain mode: $mode" >&2
      return 1
      ;;
  esac
  rustclaw_report_build_toolchains
}

rustclaw_toolchain_manager_self_test() {
  rustclaw_version_at_least "20.19.4" "20.19.0"
  rustclaw_version_at_least "1.85" "1.85.0"
  if rustclaw_version_at_least "20.18.9" "20.19.0"; then
    echo "Version comparison accepted a version below the minimum." >&2
    return 1
  fi
  [[ "$(rustclaw_version_number "Apple clang version 16.0.0")" == "16.0.0" ]]
  [[ "$(rustclaw_version_number "rustc 1.95.0 (hash)")" == "1.95.0" ]]
  echo "BUILD_TOOLCHAIN_MANAGER_SELF_TEST ok"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  # shellcheck source=/dev/null
  source "$SCRIPT_DIR/scripts/shell_compat.sh"
  case "${1:-report}" in
    self-test)
      rustclaw_toolchain_manager_self_test
      ;;
    report|check|update)
      rustclaw_manage_build_toolchains "${1:-report}" "${RUSTCLAW_INCLUDE_UI_TOOLCHAIN:-1}"
      if [[ "${1:-report}" != "check" ]]; then
        rustclaw_validate_build_toolchains "${RUSTCLAW_INCLUDE_UI_TOOLCHAIN:-1}"
      fi
      ;;
    *)
      echo "Usage: $0 [report|check|update|self-test]" >&2
      exit 1
      ;;
  esac
fi
