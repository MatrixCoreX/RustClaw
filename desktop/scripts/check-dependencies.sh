#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != Linux ]]; then
  echo 'desktop_ubuntu_dependency_check_unsupported_platform' >&2
  exit 2
fi
for command in cargo rustc node npm pkg-config; do
  command -v "$command" >/dev/null || { echo "Missing development dependency: $command" >&2; exit 1; }
done
pkg-config --modversion gtk+-3.0 webkit2gtk-4.1
echo 'Desktop build dependencies available. End users only need the packaged runtime dependencies.'
