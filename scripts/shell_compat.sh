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
    completed = subprocess.run(command, check=False)
except subprocess.TimeoutExpired:
    sys.exit(124)

sys.exit(completed.returncode)
PY
}
