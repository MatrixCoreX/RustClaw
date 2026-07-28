#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKILL_DIR="$ROOT_DIR/crates/skills/browser_web"
SERVICE_NAME="${1:-rustclaw.service}"
if [[ "$(uname -s)" == "Darwin" ]]; then
  export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"
else
  export PATH="/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin:${PATH:-}"
fi

echo "[info] root=$ROOT_DIR"
echo "[info] skill_dir=$SKILL_DIR"
echo

echo "[1/6] Platform"
uname -a || true
echo "---"
if [[ "$(uname -s)" == "Darwin" ]]; then
  sw_vers || true
  uname -m || true
else
  [[ -r /etc/os-release ]] && cat /etc/os-release
  if command -v dpkg >/dev/null 2>&1; then
    dpkg --print-architecture || true
  else
    uname -m || true
  fi
fi
echo

echo "[2/6] Node / npm"
node -v || true
npm -v || true
echo

echo "[3/6] Chromium"
CHROMIUM_PATH=""
for candidate in \
  "$(command -v chromium 2>/dev/null || true)" \
  "$(command -v chromium-browser 2>/dev/null || true)" \
  "$(command -v google-chrome 2>/dev/null || true)" \
  "/Applications/Chromium.app/Contents/MacOS/Chromium" \
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
  "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"; do
  if [[ -n "$candidate" && -x "$candidate" ]]; then
    CHROMIUM_PATH="$candidate"
    break
  fi
done
if [[ -n "$CHROMIUM_PATH" ]]; then
  echo "[ok] system browser: $CHROMIUM_PATH"
  if [[ "$(uname -s)" != "Darwin" ]]; then
    "$CHROMIUM_PATH" --version || true
  fi
else
  echo "[info] no system Chromium browser; checking Playwright-managed runtime"
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
if [[ -r /proc/self/status ]]; then
  grep -E 'NoNewPrivs|Seccomp' /proc/self/status || true
else
  echo "[info] Linux /proc restriction signals are not used on $(uname -s)"
fi
echo

echo "[6/6] Minimal Playwright launch test"
LAUNCH_EXIT=0
(cd "$SKILL_DIR" && node - <<'EOF') || LAUNCH_EXIT=$?
const fs = require('fs');
const path = require('path');

function pickChromium(chromium) {
  const candidates = [
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
    '/usr/bin/google-chrome',
    '/Applications/Chromium.app/Contents/MacOS/Chromium',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
    '/Applications/Brave Browser.app/Contents/MacOS/Brave Browser',
  ];
  for (const p of candidates) {
    if (fs.existsSync(p)) return p;
  }
  const managed = chromium.executablePath();
  if (managed && fs.existsSync(managed)) return managed;
  return null;
}

(async () => {
  try {
    const { chromium } = require('playwright');
    const executablePath = pickChromium(chromium);
    if (!executablePath) {
      throw new Error('Playwright browser runtime is not installed; install Chromium from the dependency center');
    }
    const browser = await chromium.launch({
      executablePath,
      headless: true,
      args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'],
    });
    const page = await browser.newPage();
    await page.goto('data:text/html,<title>runtime-ok</title>', {
      waitUntil: 'domcontentloaded',
      timeout: 15000,
    });
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
  echo "[info] systemd service hardening scan is not applicable on $(uname -s)"
fi
echo
if [[ "$LAUNCH_EXIT" -ne 0 ]]; then
  echo "[result] launch_check=FAIL (exit=$LAUNCH_EXIT)"
else
  echo "[result] launch_check=PASS"
fi
echo "[done] If launch failed with Operation not permitted + Seccomp/NoNewPrivs, run this script outside restricted sandbox/session."
exit "$LAUNCH_EXIT"
