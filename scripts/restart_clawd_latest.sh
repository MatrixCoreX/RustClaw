#!/usr/bin/env bash
set -euo pipefail

cd /home/guagua/rustclaw

pkill -f 'target/release/clawd|cargo run -p clawd' || true

setsid /home/guagua/rustclaw/target/release/clawd >/tmp/clawd.out 2>&1 </dev/null &

sleep 2

pgrep -n -f '^/home/guagua/rustclaw/target/release/clawd$|target/release/clawd' > /home/guagua/rustclaw/.pids/clawd.pid

cat /home/guagua/rustclaw/.pids/clawd.pid
echo '---'
pgrep -af '^/home/guagua/rustclaw/target/release/clawd$|target/release/clawd'
echo '---'
ss -lntp | rg '8787|clawd'
