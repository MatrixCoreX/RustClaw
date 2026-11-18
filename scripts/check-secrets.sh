#!/usr/bin/env bash
# Lightweight configuration guard; publication also requires a source secret scan.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/check_staged_secrets.py" "$@"
