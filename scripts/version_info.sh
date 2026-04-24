#!/usr/bin/env bash

rustclaw_version_from_root() {
  local root_dir="${1:-}"
  local cargo_toml="${root_dir%/}/Cargo.toml"
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

print_rustclaw_version() {
  local root_dir="${1:-}"
  if [[ "${RUSTCLAW_VERSION_PRINTED:-0}" == "1" ]]; then
    return 0
  fi
  export RUSTCLAW_VERSION_PRINTED=1
  printf 'RustClaw version: %s\n' "$(rustclaw_version_from_root "$root_dir")"
}
