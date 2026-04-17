#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKILL_DIR="$ROOT_DIR/crates/skills/browser_web"
SERVICE_NAME="${1:-rustclaw.service}"

echo "[info] root=$ROOT_DIR"
echo "[info] skill_dir=$SKILL_DIR"
echo

echo "[1/6] Platform"
uname -a || true
echo "---"
cat /etc/os-release || true
echo "---"
dpkg --print-architecture || true
echo

echo "[2/6] Node / npm"
node -v || true
npm -v || true
echo

echo "[3/6] Chromium"
if command -v chromium >/dev/null 2>&1; then
  echo "[ok] chromium: $(command -v chromium)"
  chromium --version || true
elif command -v chromium-browser >/dev/null 2>&1; then
  echo "[ok] chromium-browser: $(command -v chromium-browser)"
  chromium-browser --version || true
else
  echo "[warn] system chromium not found"
fi
echo

echo "[4/6] Playwright package"
if [[ -f "$SKILL_DIR/package.json" ]]; then
  (cd "$SKILL_DIR" && npm ls --depth=0) || true
else
  echo "[warn] missing $SKILL_DIR/package.json"
fi
echo

echo "[5/6] Runtime restriction signals (/proc/self/status)"
grep -E 'NoNewPrivs|Seccomp' /proc/self/status || true
echo

echo "[6/6] Minimal Playwright launch test"
LAUNCH_EXIT=0
(cd "$SKILL_DIR" && node - <<'EOF') || LAUNCH_EXIT=$?
const fs = require('fs');
const path = require('path');

function pickChromium() {
  const candidates = ['/usr/bin/chromium', '/usr/bin/chromium-browser'];
  for (const p of candidates) {
    if (fs.existsSync(p)) return p;
  }
  return null;
}

(async () => {
  try {
    const { chromium } = require('playwright');
    const executablePath = pickChromium();
    const browser = await chromium.launch({
      executablePath: executablePath || undefined,
      headless: true,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
    });
    const page = await browser.newPage();
    await page.goto('https://example.com', { waitUntil: 'domcontentloaded', timeout: 15000 });
    console.log('[ok] launch succeeded, title=', await page.title());
    await browser.close();
    process.exit(0);
  } catch (e) {
    console.error('[fail] launch failed:', e.message || String(e));
    process.exit(2);
  }
})();
EOF
echo

echo "[extra] Service hardening scan: $SERVICE_NAME"
if command -v systemctl >/dev/null 2>&1; then
  systemctl cat "$SERVICE_NAME" 2>/dev/null | rg -n "NoNewPrivileges|SystemCallFilter|PrivateTmp|ProtectSystem|RestrictAddressFamilies|MemoryDenyWriteExecute|CapabilityBoundingSet" -n || true
else
  echo "[warn] systemctl not found"
fi
echo
if [[ "$LAUNCH_EXIT" -ne 0 ]]; then
  echo "[result] launch_check=FAIL (exit=$LAUNCH_EXIT)"
else
  echo "[result] launch_check=PASS"
fi
echo "[done] If launch failed with Operation not permitted + Seccomp/NoNewPrivs, run this script outside restricted sandbox/session."
exit "$LAUNCH_EXIT"
