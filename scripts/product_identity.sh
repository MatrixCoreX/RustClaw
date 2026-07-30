#!/usr/bin/env bash

# Central product identity projection for shell code.

PRODUCT_IDENTITY_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_PRODUCT_IDENTITY_CONFIG="${APP_PRODUCT_IDENTITY_CONFIG:-$PRODUCT_IDENTITY_SCRIPT_DIR/../configs/product_identity.toml}"

if [[ ! -f "$APP_PRODUCT_IDENTITY_CONFIG" ]]; then
  echo "Product identity config not found: $APP_PRODUCT_IDENTITY_CONFIG" >&2
  return 1 2>/dev/null || exit 1
fi

product_identity_config_value() {
  local key="$1"
  [[ -f "$APP_PRODUCT_IDENTITY_CONFIG" ]] || return 0
  awk -F= -v wanted="$key" '
    $1 ~ "^[[:space:]]*" wanted "[[:space:]]*$" {
      value = substr($0, index($0, "=") + 1)
      sub(/^[[:space:]]+/, "", value)
      sub(/[[:space:]]+#.*$/, "", value)
      sub(/[[:space:]]+$/, "", value)
      if (value ~ /^".*"$/) {
        value = substr(value, 2, length(value) - 2)
      }
      print value
      exit
    }
  ' "$APP_PRODUCT_IDENTITY_CONFIG"
}

product_identity_config_multiline_value() {
  local key="$1"
  [[ -f "$APP_PRODUCT_IDENTITY_CONFIG" ]] || return 0
  awk -v wanted="$key" '
    !reading && $0 ~ "^[[:space:]]*" wanted "[[:space:]]*=[[:space:]]*\\047\\047\\047[[:space:]]*$" {
      reading = 1
      next
    }
    reading && $0 ~ "^[[:space:]]*\\047\\047\\047[[:space:]]*$" { exit }
    reading { print }
  ' "$APP_PRODUCT_IDENTITY_CONFIG"
}

valid_product_slug() {
  local value="$1"
  [[ -n "$value" && ${#value} -le 64 && "$value" != -* && "$value" != *- && "$value" != *[!a-z0-9-]* ]]
}

valid_release_repository() {
  local value="$1"
  [[ "$value" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]
}

valid_safe_filename() {
  local value="$1"
  [[ -n "$value" && "$value" != "." && "$value" != ".." && "$value" != */* && "$value" != *\\* ]]
}

CONFIG_DISPLAY_NAME="$(product_identity_config_value display_name)"
CONFIG_RELEASE_ARTIFACT_ID="$(product_identity_config_value release_artifact_id)"
CONFIG_TERMINAL_BANNER="$(product_identity_config_multiline_value terminal_banner)"
CONFIG_RELEASE_REPOSITORY="$(product_identity_config_value release_repository)"
CONFIG_SMALL_SCREEN_SPLASH_IMAGE="$(product_identity_config_value small_screen_splash_image)"
CONFIG_SCHEMA_VERSION="$(product_identity_config_value schema_version)"

if [[ "$CONFIG_SCHEMA_VERSION" != "1" ]]; then
  echo "Unsupported product identity schema: ${CONFIG_SCHEMA_VERSION:-missing}" >&2
  return 1 2>/dev/null || exit 1
fi
if [[ -z "$CONFIG_DISPLAY_NAME" || -z "$CONFIG_TERMINAL_BANNER" ]]; then
  echo "Product identity display_name and terminal_banner must not be empty." >&2
  return 1 2>/dev/null || exit 1
fi
if ! valid_product_slug "$CONFIG_RELEASE_ARTIFACT_ID"; then
  echo "Product identity release_artifact_id must use 1-64 lowercase letters, digits, or hyphens." >&2
  return 1 2>/dev/null || exit 1
fi
if ! valid_release_repository "$CONFIG_RELEASE_REPOSITORY"; then
  echo "Product identity release_repository must use owner/repository syntax." >&2
  return 1 2>/dev/null || exit 1
fi
if ! valid_safe_filename "$CONFIG_SMALL_SCREEN_SPLASH_IMAGE"; then
  echo "Product identity small_screen_splash_image must be a safe file name." >&2
  return 1 2>/dev/null || exit 1
fi

# Brand fields are defined only by the selected TOML file. These exported
# values are projections for existing shell/build consumers, not override
# inputs and therefore intentionally replace any inherited values.
APP_DISPLAY_NAME="$CONFIG_DISPLAY_NAME"
APP_RELEASE_ARTIFACT_ID="$CONFIG_RELEASE_ARTIFACT_ID"
APP_SERVICE_NAME="agent-runtime"
APP_DATA_NAMESPACE="agent-runtime"
APP_TERMINAL_BANNER="$CONFIG_TERMINAL_BANNER"
APP_RELEASE_REPOSITORY="$CONFIG_RELEASE_REPOSITORY"
APP_SMALL_SCREEN_SPLASH_IMAGE="$CONFIG_SMALL_SCREEN_SPLASH_IMAGE"

export APP_PRODUCT_IDENTITY_CONFIG APP_DISPLAY_NAME APP_RELEASE_ARTIFACT_ID APP_SERVICE_NAME APP_DATA_NAMESPACE APP_TERMINAL_BANNER
export APP_RELEASE_REPOSITORY APP_SMALL_SCREEN_SPLASH_IMAGE
