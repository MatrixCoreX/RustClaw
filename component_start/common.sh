#!/usr/bin/env bash

component_start_init() {
  local root_dir="$1"
  local requested_profile="$2"
  local entrypoint="$3"

  COMPONENT_ROOT="$(cd "$root_dir" && pwd)"
  COMPONENT_PROFILE="${requested_profile:-${RUSTCLAW_START_PROFILE:-release}}"
  COMPONENT_ENTRYPOINT="$entrypoint"

  case "$COMPONENT_PROFILE" in
    release) ;;
    *)
      echo "Usage: $COMPONENT_ENTRYPOINT [release]" >&2
      return 2
      ;;
  esac

  cd "$COMPONENT_ROOT"
  # shellcheck source=/dev/null
  source "$COMPONENT_ROOT/scripts/shell_compat.sh"
  # shellcheck source=/dev/null
  source "$COMPONENT_ROOT/scripts/version_info.sh"
  print_rustclaw_version "$COMPONENT_ROOT"

  if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
  fi

  local runtime_env_script
  runtime_env_script="${RUSTCLAW_RUNTIME_ENV_SCRIPT:-$HOME/runtime_env_filled.sh}"
  if [[ -f "$runtime_env_script" ]]; then
    # shellcheck source=/dev/null
    source "$runtime_env_script"
  fi

  COMPONENT_CONFIG_PATH="${RUSTCLAW_CONFIG_PATH:-$COMPONENT_ROOT/configs/config.toml}"
  case "$COMPONENT_CONFIG_PATH" in
    /*) ;;
    *) COMPONENT_CONFIG_PATH="$COMPONENT_ROOT/$COMPONENT_CONFIG_PATH" ;;
  esac
  export RUSTCLAW_CONFIG_PATH="$COMPONENT_CONFIG_PATH"

  COMPONENT_CHANNEL_CONFIG_DIR="${RUSTCLAW_CHANNEL_CONFIG_DIR:-$COMPONENT_ROOT/configs/channels}"
  case "$COMPONENT_CHANNEL_CONFIG_DIR" in
    /*) ;;
    *) COMPONENT_CHANNEL_CONFIG_DIR="$COMPONENT_ROOT/$COMPONENT_CHANNEL_CONFIG_DIR" ;;
  esac
  export RUSTCLAW_CHANNEL_CONFIG_DIR="$COMPONENT_CHANNEL_CONFIG_DIR"

  if [[ -t 1 && -z "${RUSTCLAW_LOG_COLOR:-}" ]]; then
    export RUSTCLAW_LOG_COLOR=1
  fi

  COMPONENT_PID_DIR="$COMPONENT_ROOT/.pids"
  mkdir -p "$COMPONENT_PID_DIR"
}

component_binary_path() {
  local binary_name="$1"
  printf '%s\n' "$COMPONENT_ROOT/target/$COMPONENT_PROFILE/$binary_name"
}

component_require_binary() {
  local binary_name="$1"
  local binary_path
  binary_path="$(component_binary_path "$binary_name")"
  if [[ ! -x "$binary_path" ]]; then
    echo "Binary missing or not executable: $binary_path" >&2
    echo "Build it first: cargo build -p $binary_name --release" >&2
    return 1
  fi
  printf '%s\n' "$binary_path"
}

component_process_command() {
  local pid="$1"
  local cmdline_file="/proc/$pid/cmdline"
  if [[ -r "$cmdline_file" ]]; then
    tr '\0' ' ' < "$cmdline_file" 2>/dev/null || true
    return
  fi
  ps -p "$pid" -o command= 2>/dev/null || true
}

component_process_is_running() {
  local pid="$1"
  local state=""
  state="$(ps -p "$pid" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
  [[ -n "$state" && "$state" != Z* ]]
}

component_process_matches() {
  local pid="$1"
  local expected_command="$2"
  local cmdline_file="/proc/$pid/cmdline"
  local argument=""
  if [[ -r "$cmdline_file" ]]; then
    while IFS= read -r -d '' argument; do
      if [[ "$argument" == "$expected_command" ]]; then
        return 0
      fi
    done < "$cmdline_file"
    return 1
  fi
  local command=""
  command="$(component_process_command "$pid")"
  [[ -n "$command" && "$command" == *"$expected_command"* ]]
}

component_find_matching_pid() {
  local expected_command="$1"
  local search_token=""
  local candidates=""
  local pid=""
  search_token="$(basename "$expected_command")"

  if command -v pgrep >/dev/null 2>&1; then
    candidates="$(pgrep -f "$search_token" 2>/dev/null || true)"
  else
    candidates="$(
      while read -r candidate command; do
        case "$candidate" in
          ""|*[!0-9]*) continue ;;
        esac
        if [[ "$command" == *"$expected_command"* ]]; then
          printf '%s\n' "$candidate"
        fi
      done < <(ps -ax -o pid= -o command= 2>/dev/null || true)
    )"
  fi

  for pid in $candidates; do
    if [[ "$pid" != "$$" ]] && component_process_is_running "$pid" &&
      component_process_matches "$pid" "$expected_command"; then
      printf '%s\n' "$pid"
      return 0
    fi
  done
  return 1
}

component_pid_is_running() {
  local component="$1"
  local expected_command="${2:-}"
  local pid_file="$COMPONENT_PID_DIR/$component.pid"
  local pid=""
  if [[ -f "$pid_file" ]]; then
    pid="$(tr -d '[:space:]' < "$pid_file" 2>/dev/null || true)"
    case "$pid" in
      ""|*[!0-9]*)
        rm -f "$pid_file"
        pid=""
        ;;
    esac
    if [[ -n "$pid" ]] && ! component_process_is_running "$pid"; then
      rm -f "$pid_file"
      pid=""
    fi
    if [[ -n "$pid" && -n "$expected_command" ]] &&
      ! component_process_matches "$pid" "$expected_command"; then
      echo "Ignoring stale PID file for $component: PID $pid belongs to another process." >&2
      rm -f "$pid_file"
      pid=""
    fi
    if [[ -n "$pid" ]]; then
      return 0
    fi
  fi

  if [[ -n "$expected_command" ]]; then
    pid="$(component_find_matching_pid "$expected_command" || true)"
    if [[ -n "$pid" ]]; then
      component_write_pid_file "$component" "$pid"
      return 0
    fi
  fi
  if [[ -f "$pid_file" ]]; then
    rm -f "$pid_file"
  fi
  return 1
}

component_write_pid_file() {
  local component="$1"
  local pid="$2"
  local pid_file="$COMPONENT_PID_DIR/$component.pid"
  local temporary="${pid_file}.tmp.$$"
  printf '%s\n' "$pid" > "$temporary"
  mv -f "$temporary" "$pid_file"
}

component_exec_binary() {
  local component="$1"
  local binary_path="$2"
  if component_pid_is_running "$component" "$binary_path"; then
    echo "Component is already running: $component" >&2
    return 1
  fi
  component_write_pid_file "$component" "$$"
  exec "$binary_path"
}
