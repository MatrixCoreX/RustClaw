#!/usr/bin/env bash

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

append_to_array() {
  local array_name="$1"
  local value="$2"
  local length=0
  eval "length=\${#${array_name}[@]}"
  printf -v "${array_name}[${length}]" '%s' "$value"
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

cargo_jobs_for_small_host() {
  local host_os host_arch mem_kb cpu_count
  host_os="$(detect_host_os 2>/dev/null || printf '%s' "unknown")"
  host_arch="$(detect_host_arch 2>/dev/null || printf '%s' "unknown")"
  mem_kb="$(awk '/MemTotal:/ {print $2; exit}' /proc/meminfo 2>/dev/null || printf '%s' "0")"
  cpu_count="$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '%s' "1")"

  case "$cpu_count" in
    ''|*[!0-9]*) cpu_count=1 ;;
  esac
  case "$mem_kb" in
    ''|*[!0-9]*) mem_kb=0 ;;
  esac

  if [[ "$host_os" != "linux" ]]; then
    return 1
  fi

  if [[ "$host_arch" == "aarch64" || "$host_arch" == "armv7" || ( "$mem_kb" -gt 0 && "$mem_kb" -le 4194304 ) ]]; then
    if [[ "$mem_kb" -gt 0 && "$mem_kb" -le 3145728 ]]; then
      printf '%s\n' "1"
      return 0
    fi
    if [[ "$cpu_count" -le 1 ]]; then
      printf '%s\n' "1"
    else
      printf '%s\n' "2"
    fi
    return 0
  fi

  return 1
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
  echo "CARGO_BUILD_JOBS not set; using $CARGO_BUILD_JOBS on this small/ARM host to reduce build memory pressure."
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
