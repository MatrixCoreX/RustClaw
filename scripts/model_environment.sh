#!/usr/bin/env bash

load_managed_model_environment() {
  local root="$1"
  local file="$root/.agent-runtime/credentials/models.env"
  local line name value assignment
  local -a assignments=()
  local -a names=()
  local previous
  for assignment in "$root/.agent-runtime" "$root/.agent-runtime/credentials" "$file"; do
    if [[ -L "$assignment" ]]; then
      echo "Managed model environment must not use symlinks." >&2
      return 1
    fi
  done
  [[ -e "$file" ]] || return 0
  if [[ ! -f "$file" || ! -r "$file" || "$(wc -c < "$file")" -gt 65536 ]]; then
    echo "Cannot read managed model environment." >&2
    return 1
  fi
  # Parse literal assignments, never source/eval user-entered credentials.
  while IFS= read -r line || [[ -n "$line" ]]; do
    case "$line" in ''|'#'*) continue ;; esac
    name="${line%%=*}"
    value="${line#*=}"
    case "$name" in
      OPENAI_API_KEY|GOOGLE_API_KEY|ANTHROPIC_API_KEY|GROK_API_KEY|DEEPSEEK_API_KEY|QWEN_API_KEY|MINIMAX_API_KEY|MIMO_API_KEY|CUSTOM_API_KEY) ;;
      *) echo "Invalid managed model environment name." >&2; return 1 ;;
    esac
    if [[ "$line" != *=* || -z "$value" || ${#value} -gt 4096 || "$value" == REPLACE_ME* || "$value" == *[[:cntrl:]]* ]]; then
      echo "Invalid managed model environment value." >&2
      return 1
    fi
    for previous in ${names[@]+"${names[@]}"}; do
      if [[ "$previous" == "$name" ]]; then
        echo "Duplicate managed model environment name." >&2
        return 1
      fi
    done
    names+=("$name")
    assignments+=("$name=$value")
  done < "$file"
  for assignment in ${assignments[@]+"${assignments[@]}"}; do
    export "$assignment"
  done
}
