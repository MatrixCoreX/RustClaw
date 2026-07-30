#!/usr/bin/env bash

VERSION_INFO_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

app_version_from_root() {
  local root_dir="${1:-}"
  local cargo_toml="${root_dir%/}/Cargo.toml"
  local version_file="${root_dir%/}/VERSION"
  local candidate=""

  candidate="${APP_VERSION:-}"
  if [[ -z "$candidate" && -f "$version_file" ]]; then
    candidate="$(head -n 1 "$version_file" | tr -d '\r\n')"
  fi
  if [[ -n "$candidate" && "$candidate" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi

  if [[ -z "$root_dir" || ! -f "$cargo_toml" ]]; then
    printf '%s\n' "unknown"
    return 0
  fi
  awk '
    /^[[:space:]]*\[workspace\.package\][[:space:]]*$/ {
      in_workspace_package = 1
      next
    }
    in_workspace_package && /^[[:space:]]*\[/ {
      exit
    }
    in_workspace_package && /^[[:space:]]*version[[:space:]]*=/ {
      line = $0
      sub(/^[^=]*=[[:space:]]*/, "", line)
      sub(/[[:space:]]*#.*/, "", line)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
      gsub(/^"|"$/, "", line)
      print line
      found = 1
      exit
    }
    END {
      if (!found) {
        print "unknown"
      }
    }
  ' "$cargo_toml"
}

print_app_version() {
  local root_dir="${1:-}"
  if [[ "${APP_VERSION_PRINTED:-0}" == "1" ]]; then
    return 0
  fi
  if [[ -z "${APP_DISPLAY_NAME:-}" ]]; then
    # shellcheck source=/dev/null
    source "${VERSION_INFO_SCRIPT_DIR}/product_identity.sh"
  fi
  export APP_VERSION_PRINTED=1
  printf '%s version: %s\n' "$APP_DISPLAY_NAME" "$(app_version_from_root "$root_dir")"
}
