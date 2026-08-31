#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

ARTIFACT="$TMP_ROOT/agent-runtime-test.tar.gz"
SBOM="$ARTIFACT.spdx.json"
MANIFEST="$ARTIFACT.manifest.json"
printf 'release fixture\n' > "$ARTIFACT"
python3 "$ROOT_DIR/scripts/security/generate_release_sbom.py" \
  --root "$ROOT_DIR" \
  --output "$SBOM" \
  --name agent-runtime \
  --version 0.1.8 \
  --commit 0123456789abcdef0123456789abcdef01234567
python3 "$ROOT_DIR/scripts/security/release_manifest.py" create \
  --artifact "$ARTIFACT" \
  --sbom "$SBOM" \
  --output "$MANIFEST" \
  --version 0.1.8 \
  --commit 0123456789abcdef0123456789abcdef01234567 \
  --target x86_64-unknown-linux-gnu \
  --package-root agent-runtime
python3 "$ROOT_DIR/scripts/security/release_manifest.py" verify \
  --artifact "$ARTIFACT" \
  --sbom "$SBOM" \
  --manifest "$MANIFEST" \
  --expected-target x86_64-unknown-linux-gnu \
  --expected-package-root agent-runtime \
  --expected-version 0.1.8 >/dev/null

python3 - "$SBOM" <<'PY'
import json
from pathlib import Path
import sys

document = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
assert document["spdxVersion"] == "SPDX-2.3"
assert document["dataLicense"] == "CC0-1.0"
assert len(document["packages"]) > 1
assert all(package["SPDXID"].startswith("SPDXRef-") for package in document["packages"])
PY

if python3 "$ROOT_DIR/scripts/security/release_manifest.py" verify \
  --artifact "$ARTIFACT" \
  --sbom "$SBOM" \
  --manifest "$MANIFEST" \
  --expected-target aarch64-unknown-linux-gnu \
  --expected-package-root agent-runtime >/dev/null 2>&1; then
  echo "wrong target unexpectedly verified" >&2
  exit 1
fi

cp "$SBOM" "$TMP_ROOT/tampered.spdx.json"
printf 'x' >> "$TMP_ROOT/tampered.spdx.json"
if python3 "$ROOT_DIR/scripts/security/release_manifest.py" verify \
  --artifact "$ARTIFACT" \
  --sbom "$TMP_ROOT/tampered.spdx.json" \
  --manifest "$MANIFEST" \
  --expected-target x86_64-unknown-linux-gnu \
  --expected-package-root agent-runtime >/dev/null 2>&1; then
  echo "tampered SBOM unexpectedly verified" >&2
  exit 1
fi

echo "RELEASE_SUPPLY_CHAIN_TESTS ok"
