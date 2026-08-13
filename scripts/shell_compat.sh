#!/usr/bin/env bash

SHELL_COMPAT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "$SHELL_COMPAT_DIR/product_identity.sh"

resolve_path_python() {
  local target="$1"
  python3 - "$target" <<'PY'
from pathlib import Path
import sys

print(Path(sys.argv[1]).resolve())
PY
}

resolve_script_dir() {
  local source_path="$1"
  dirname "$(resolve_path_python "$source_path")"
}

default_macos_deployment_target() {
  local product_version="${1:-}"
  local major=""
  local remainder=""
  local minor=""

  product_version="${product_version%%-*}"
  major="${product_version%%.*}"
  remainder="${product_version#*.}"
  if [[ "$remainder" != "$product_version" ]]; then
    minor="${remainder%%.*}"
  fi
  case "$major" in
    ''|*[!0-9]*) return 1 ;;
  esac

  if (( major >= 11 )); then
    printf '%s.0\n' "$major"
    return 0
  fi
  if [[ "$major" == "10" ]]; then
    case "$minor" in
      ''|*[!0-9]*) return 1 ;;
    esac
    printf '10.%s\n' "$minor"
    return 0
  fi
  return 1
}

configure_macos_target_rustflags() {
  local deployment_target="$1"
  local deployment_flag="-C link-arg=-mmacosx-version-min=${deployment_target}"
  local rustflags_var=""
  local current=""
  local updated=""

  for rustflags_var in \
    CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS \
    CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS; do
    current="$(printenv "$rustflags_var" 2>/dev/null || true)"
    if [[ "$current" == *"-mmacosx-version-min="* ]]; then
      continue
    fi
    if [[ -n "$current" ]]; then
      updated="$current $deployment_flag"
    else
      updated="$deployment_flag"
    fi
    export "$rustflags_var=$updated"
  done
}

configure_macos_deployment_target() {
  local host_os="${1:-}"
  local host_version="${2:-}"
  local target=""

  if [[ -z "$host_os" ]]; then
    host_os="$(detect_host_os 2>/dev/null || true)"
  fi
  [[ "$host_os" == "macos" ]] || return 0

  if [[ -n "${MACOSX_DEPLOYMENT_TARGET:-}" ]]; then
    # An explicit standard variable is authoritative. The Rust toolchain used
    # by that build must provide compatible standard libraries.
    target="$MACOSX_DEPLOYMENT_TARGET"
  elif [[ -n "${APP_MACOS_DEPLOYMENT_TARGET:-}" ]]; then
    target="$APP_MACOS_DEPLOYMENT_TARGET"
  else
    if [[ -z "$host_version" ]]; then
      host_version="$(sw_vers -productVersion 2>/dev/null || true)"
    fi
    target="$(default_macos_deployment_target "$host_version" 2>/dev/null || true)"
    if [[ -z "$target" ]]; then
      echo "Unable to determine a macOS deployment target from: $host_version" >&2
      return 1
    fi
  fi

  if [[ ! "$target" =~ ^[0-9]+([.][0-9]+){1,2}$ ]]; then
    echo "Invalid macOS deployment target: $target" >&2
    return 2
  fi

  export MACOSX_DEPLOYMENT_TARGET="$target"
  # Cargo does not reliably invalidate already-linked host artifacts when only
  # MACOSX_DEPLOYMENT_TARGET changes. A target-specific link flag both enforces
  # the Mach-O minimum version and makes the deployment target part of Cargo's
  # build fingerprint without affecting Linux targets.
  configure_macos_target_rustflags "$target"
}

append_existing_command_path() {
  local candidate="$1"
  [[ -d "$candidate" ]] || return 0
  case ":${PATH:-}:" in
    *":${candidate}:"*) ;;
    *) PATH="${PATH:+${PATH}:}${candidate}" ;;
  esac
  export PATH
}

prepend_existing_command_path() {
  local candidate="$1"
  [[ -d "$candidate" ]] || return 0
  case ":${PATH:-}:" in
    *":${candidate}:"*) ;;
    *) PATH="${candidate}${PATH:+:${PATH}}" ;;
  esac
  export PATH
}

configure_user_toolchain_command_path() {
  local cargo_home="${CARGO_HOME:-}"
  if [[ -z "$cargo_home" && -n "${HOME:-}" ]]; then
    cargo_home="$HOME/.cargo"
  fi
  if [[ -n "$cargo_home" ]]; then
    # Runtime environment files and service managers commonly provide a
    # minimal PATH. Restore rustup's user-level proxy directory after those
    # environments are loaded so Cargo-backed Skill Store installs work from
    # non-login processes on Linux and macOS.
    prepend_existing_command_path "$cargo_home/bin"
  fi
}

configure_platform_command_path() {
  local candidate
  configure_user_toolchain_command_path
  if [[ "$(uname -s 2>/dev/null || true)" != "Darwin" ]]; then
    return 0
  fi
  # GUI launchers, LaunchAgent, and non-login SSH sessions do not reliably
  # inherit Homebrew paths. Add only existing standard prefixes, after the
  # caller's PATH so an explicitly selected rustup or managed toolchain is not
  # silently replaced by a Homebrew compiler that lacks cross-target stdlibs.
  for candidate in \
    /usr/local/sbin \
    /usr/local/bin \
    /opt/homebrew/sbin \
    /opt/homebrew/bin; do
    append_existing_command_path "$candidate"
  done
  configure_macos_deployment_target
}

resolve_python3_with_tomllib() {
  local candidate resolved seen=""
  for candidate in \
    "${APP_PYTHON_BIN:-}" \
    "$(command -v python3 2>/dev/null || true)" \
    /usr/local/bin/python3 \
    /opt/homebrew/bin/python3 \
    "$(command -v python3.14 2>/dev/null || true)" \
    "$(command -v python3.13 2>/dev/null || true)" \
    "$(command -v python3.12 2>/dev/null || true)" \
    "$(command -v python3.11 2>/dev/null || true)"; do
    [[ -n "$candidate" && -x "$candidate" ]] || continue
    resolved="$(cd "$(dirname "$candidate")" 2>/dev/null && pwd -P)/$(basename "$candidate")"
    case ":$seen:" in
      *":$resolved:"*) continue ;;
    esac
    seen="${seen:+$seen:}$resolved"
    if "$resolved" -c 'import tomllib' >/dev/null 2>&1; then
      printf '%s\n' "$resolved"
      return 0
    fi
  done
  echo "Python 3.11+ with stdlib tomllib is required; install it or set APP_PYTHON_BIN." >&2
  return 1
}

configure_python3_with_tomllib() {
  APP_PYTHON_BIN="$(resolve_python3_with_tomllib)" || return 1
  export APP_PYTHON_BIN
  # Keep the caller's PATH ordering intact so selecting a Homebrew Python on
  # macOS cannot silently replace an explicitly selected rustup Cargo.
  python3() {
    command "$APP_PYTHON_BIN" "$@"
  }
}

append_to_array() {
  local array_name="$1"
  local value="$2"
  local length=0
  local quoted_value
  if [[ ! "$array_name" =~ ^[a-zA-Z_][a-zA-Z0-9_]*$ ]]; then
    echo "invalid array variable name: $array_name" >&2
    return 2
  fi
  eval "length=\${#${array_name}[@]}"
  # Bash 3.2 (the system shell on older macOS releases) cannot use printf -v
  # with an indexed-array destination. Quote into a scalar, then assign it.
  printf -v quoted_value '%q' "$value"
  eval "${array_name}[${length}]=${quoted_value}"
}

array_from_command_lines() {
  local array_name="$1"
  shift
  local line
  eval "${array_name}=()"
  while IFS= read -r line; do
    append_to_array "$array_name" "$line"
  done < <("$@")
}

array_from_string_lines() {
  local array_name="$1"
  local data="${2-}"
  local line
  eval "${array_name}=()"
  while IFS= read -r line; do
    append_to_array "$array_name" "$line"
  done <<< "$data"
}

run_with_timeout() {
  local timeout_seconds="$1"
  shift

  if command -v timeout >/dev/null 2>&1; then
    timeout "$timeout_seconds" "$@"
    return $?
  fi

  if command -v gtimeout >/dev/null 2>&1; then
    gtimeout "$timeout_seconds" "$@"
    return $?
  fi

  python3 - "$timeout_seconds" "$@" <<'PY'
import subprocess
import sys

timeout_seconds = int(sys.argv[1])
command = sys.argv[2:]

try:
    completed = subprocess.run(command, check=False, timeout=timeout_seconds)
except subprocess.TimeoutExpired:
    sys.exit(124)

sys.exit(completed.returncode)
PY
}

file_mtime_epoch() {
  python3 - "$1" <<'PY'
import os
import sys

try:
    print(int(os.path.getmtime(sys.argv[1])))
except OSError:
    print(0)
PY
}

file_size_bytes() {
  python3 - "$1" <<'PY'
import os
import sys

try:
    print(os.path.getsize(sys.argv[1]))
except OSError:
    print(0)
PY
}

latest_tree_mtime_epoch() {
  local root="$1"
  local suffix="${2:-}"
  python3 - "$root" "$suffix" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
suffix = sys.argv[2]
latest = 0
try:
    for path in root.rglob("*"):
        if path.is_file() and (not suffix or path.name.endswith(suffix)):
            latest = max(latest, int(path.stat().st_mtime))
except OSError:
    pass
print(latest)
PY
}

format_epoch_local() {
  python3 - "$1" <<'PY'
from datetime import datetime
import sys

try:
    print(datetime.fromtimestamp(int(sys.argv[1])).strftime("%Y-%m-%d %H:%M:%S"))
except (ValueError, OSError, OverflowError):
    print("unknown")
PY
}

normalize_host_os() {
  local raw="${1:-}"
  case "$raw" in
    Darwin) printf '%s\n' "macos" ;;
    Linux) printf '%s\n' "linux" ;;
    *)
      printf '%s\n' "unknown"
      return 1
      ;;
  esac
}

normalize_host_arch() {
  local raw="${1:-}"
  case "$raw" in
    x86_64|amd64) printf '%s\n' "x86_64" ;;
    arm64|aarch64) printf '%s\n' "aarch64" ;;
    armv7l|armv7) printf '%s\n' "armv7" ;;
    *)
      printf '%s\n' "$raw"
      return 1
      ;;
  esac
}

detect_host_os() {
  normalize_host_os "$(uname -s)"
}

detect_host_arch() {
  normalize_host_arch "$(uname -m)"
}

rust_target_for_platform() {
  local host_os="${1:-}"
  local host_arch="${2:-}"
  case "${host_os}:${host_arch}" in
    macos:x86_64) printf '%s\n' "x86_64-apple-darwin" ;;
    macos:aarch64) printf '%s\n' "aarch64-apple-darwin" ;;
    linux:x86_64) printf '%s\n' "x86_64-unknown-linux-gnu" ;;
    linux:aarch64) printf '%s\n' "aarch64-unknown-linux-gnu" ;;
    linux:armv7) printf '%s\n' "armv7-unknown-linux-gnueabihf" ;;
    *)
      printf '%s\n' ""
      return 1
      ;;
  esac
}

host_rust_target() {
  local host_os host_arch
  host_os="$(detect_host_os)" || return 1
  host_arch="$(detect_host_arch)" || return 1
  rust_target_for_platform "$host_os" "$host_arch"
}

cargo_host_memory_kb() {
  local host_os bytes
  host_os="$(detect_host_os 2>/dev/null || printf '%s' "unknown")"
  case "$host_os" in
    linux)
      awk '/MemTotal:/ {print $2; exit}' /proc/meminfo 2>/dev/null
      ;;
    macos)
      bytes="$(sysctl -n hw.memsize 2>/dev/null || printf '%s' "0")"
      case "$bytes" in
        ''|*[!0-9]*) printf '%s\n' "0" ;;
        *) awk -v value="$bytes" 'BEGIN { printf "%.0f\n", value / 1024 }' ;;
      esac
      ;;
    *)
      printf '%s\n' "0"
      return 1
      ;;
  esac
}

cargo_host_available_memory_kb() {
  local host_os bytes page_size
  host_os="$(detect_host_os 2>/dev/null || printf '%s' "unknown")"
  case "$host_os" in
    linux)
      awk '/MemAvailable:/ {print $2; exit}' /proc/meminfo 2>/dev/null
      ;;
    macos)
      page_size="$(sysctl -n hw.pagesize 2>/dev/null || printf '%s' "4096")"
      vm_stat 2>/dev/null | awk -v page_size="$page_size" '
        /Pages free:|Pages inactive:|Pages speculative:/ {
          value = $NF
          gsub(/\./, "", value)
          pages += value
        }
        END { printf "%.0f\n", pages * page_size / 1024 }
      '
      ;;
    *)
      printf '%s\n' "0"
      return 1
      ;;
  esac
}

cargo_host_cpu_count() {
  local count
  count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || printf '%s' "0")"
  case "$count" in
    ''|*[!0-9]*) printf '%s\n' "0" ;;
    *) printf '%s\n' "$count" ;;
  esac
}

launch_detached_process() {
  local log_file="$1"
  shift
  if [[ "$#" -eq 0 ]]; then
    echo "launch_detached_process requires a command" >&2
    return 2
  fi
  mkdir -p "$(dirname "$log_file")"
  if command -v setsid >/dev/null 2>&1; then
    setsid "$@" >"$log_file" 2>&1 </dev/null &
  else
    nohup python3 -c \
      'import os, sys; os.setsid(); os.execv(sys.argv[1], sys.argv[1:])' \
      "$@" >"$log_file" 2>&1 </dev/null &
  fi
  DETACHED_PROCESS_PID=$!
  export DETACHED_PROCESS_PID
}

cargo_jobs_for_host_capacity() {
  local host_arch="${1:-unknown}"
  local total_kb="${2:-0}"
  local available_kb="${3:-0}"
  local cpu_count="${4:-0}"
  case "$total_kb:$available_kb:$cpu_count" in
    *[!0-9:]*) return 1 ;;
  esac

  if [[ "$host_arch" == "aarch64" || "$host_arch" == "armv7" ]]; then
    printf '%s\n' "1"
    return 0
  fi
  if [[ "$total_kb" -gt 0 && "$total_kb" -le 16777216 ]]; then
    if [[ "$total_kb" -ge 14680064 && "$available_kb" -ge 10485760 \
      && "$cpu_count" -ge 8 ]]; then
      printf '%s\n' "4"
    elif [[ "$total_kb" -ge 12582912 && "$available_kb" -ge 8388608 \
      && "$cpu_count" -ge 4 ]]; then
      printf '%s\n' "2"
    else
      printf '%s\n' "1"
    fi
    return 0
  fi
  return 1
}

cargo_jobs_for_small_host() {
  local host_arch mem_kb available_kb cpu_count
  host_arch="$(detect_host_arch 2>/dev/null || printf '%s' "unknown")"
  mem_kb="$(cargo_host_memory_kb 2>/dev/null || printf '%s' "0")"
  available_kb="$(cargo_host_available_memory_kb 2>/dev/null || printf '%s' "0")"
  cpu_count="$(cargo_host_cpu_count 2>/dev/null || printf '%s' "0")"
  case "$mem_kb" in
    ''|*[!0-9]*) mem_kb=0 ;;
  esac
  case "$available_kb" in
    ''|*[!0-9]*) available_kb=0 ;;
  esac
  case "$cpu_count" in
    ''|*[!0-9]*) cpu_count=0 ;;
  esac
  cargo_jobs_for_host_capacity "$host_arch" "$mem_kb" "$available_kb" "$cpu_count"
}

configure_cargo_build_jobs_for_small_host() {
  if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
    return 0
  fi

  local jobs
  jobs="$(cargo_jobs_for_small_host 2>/dev/null || true)"
  if [[ -z "$jobs" ]]; then
    return 0
  fi

  export CARGO_BUILD_JOBS="$jobs"
  echo "CARGO_BUILD_JOBS not set; using $CARGO_BUILD_JOBS after CPU and available-memory capacity detection."
}

cargo_uses_sccache_wrapper() {
  if [[ -n "${RUSTC_WRAPPER:-}" ]]; then
    [[ "$(basename "$RUSTC_WRAPPER")" == "sccache" ]]
    return $?
  fi

  local cargo_home config
  cargo_home="${CARGO_HOME:-${HOME:-}/.cargo}"
  for config in \
    "$cargo_home/config.toml" \
    "$cargo_home/config" \
    "${PWD}/.cargo/config.toml" \
    "${PWD}/.cargo/config"; do
    [[ -f "$config" ]] || continue
    if awk '
      /^[[:space:]]*\[build\][[:space:]]*$/ { in_build = 1; next }
      /^[[:space:]]*\[/ { in_build = 0 }
      in_build && /^[[:space:]]*rustc-wrapper[[:space:]]*=/ {
        found = 1
        value = $0
        sub(/^[^=]*=[[:space:]]*/, "", value)
        sub(/[[:space:]]*#.*/, "", value)
        single_quote = sprintf("%c", 39)
        gsub(single_quote, "", value)
        gsub(/^[[:space:]"]+|[[:space:]"]+$/, "", value)
        count = split(value, parts, "/")
        if (parts[count] == "sccache") exit 0
        exit 1
      }
      END { if (!found) exit 1 }
    ' "$config"; then
      return 0
    fi
  done
  return 1
}

configure_cargo_build_environment() {
  configure_platform_command_path
  configure_cargo_build_jobs_for_small_host
  if cargo_uses_sccache_wrapper; then
    unset CARGO_INCREMENTAL
    export CARGO_PROFILE_DEV_INCREMENTAL=false
    export CARGO_PROFILE_TEST_INCREMENTAL=false
    export CARGO_PROFILE_RELEASE_INCREMENTAL=false
    export CARGO_PROFILE_BENCH_INCREMENTAL=false
    echo "sccache Rust wrapper detected; disabling Cargo profile incremental compilation for compatibility."
    return 0
  fi
  if [[ -z "${CARGO_INCREMENTAL:-}" && "${CI:-}" != "true" && "${CI:-}" != "1" ]]; then
    export CARGO_INCREMENTAL=1
    echo "CARGO_INCREMENTAL not set; enabling it for faster repeated local builds."
  fi
}

package_flavor_for_target() {
  local target="${1:-}"
  case "$target" in
    x86_64-apple-darwin) printf '%s\n' "macos-x86_64" ;;
    aarch64-apple-darwin) printf '%s\n' "macos-aarch64" ;;
    x86_64-unknown-linux-gnu) printf '%s\n' "linux-x86_64" ;;
    aarch64-unknown-linux-gnu) printf '%s\n' "linux-aarch64" ;;
    armv7-unknown-linux-gnueabihf) printf '%s\n' "linux-armv7" ;;
    *)
      printf '%s\n' "$target"
      return 1
      ;;
  esac
}

resolve_requested_target() {
  local requested="${1:-host}"
  if [[ -z "$requested" || "$requested" == "host" ]]; then
    host_rust_target
    return $?
  fi
  printf '%s\n' "$requested"
}

host_package_flavor() {
  local target
  target="$(host_rust_target)" || return 1
  package_flavor_for_target "$target"
}

target_release_dir() {
  local repo_root="$1"
  local target="${2:-}"
  if [[ -z "$target" ]]; then
    printf '%s\n' "$repo_root/target/release"
  else
    printf '%s\n' "$repo_root/target/$target/release"
  fi
}

preferred_release_dir_for_target() {
  local repo_root="$1"
  local target="${2:-}"
  local host_target=""
  host_target="$(host_rust_target 2>/dev/null || true)"
  if [[ -z "$target" || "$target" == "$host_target" ]]; then
    target_release_dir "$repo_root"
    return
  fi
  target_release_dir "$repo_root" "$target"
}

platform_summary_json() {
  local host_os host_arch rust_target flavor
  host_os="$(detect_host_os)" || host_os="unknown"
  host_arch="$(detect_host_arch)" || host_arch="unknown"
  rust_target="$(rust_target_for_platform "$host_os" "$host_arch" 2>/dev/null || true)"
  flavor="$(package_flavor_for_target "$rust_target" 2>/dev/null || true)"
  python3 - "$host_os" "$host_arch" "$rust_target" "$flavor" <<'PY'
import json
import sys

print(json.dumps({
    "host_os": sys.argv[1],
    "host_arch": sys.argv[2],
    "rust_target": sys.argv[3],
    "package_flavor": sys.argv[4],
}, ensure_ascii=False))
PY
}
