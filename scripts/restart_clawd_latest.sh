#!/usr/bin/env bash
set -euo pipefail

cd /home/guagua/git_upload

pkill -f 'target/(debug|release)/clawd|cargo run -p clawd' || true

setsid /home/guagua/git_upload/target/release/clawd >/tmp/clawd.out 2>&1 </dev/null &

sleep 2

pgrep -n -f '^/home/guagua/git_upload/target/release/clawd$|target/release/clawd' > /home/guagua/git_upload/.pids/clawd.pid

cat /home/guagua/git_upload/.pids/clawd.pid
echo '---'
pgrep -af '^/home/guagua/git_upload/target/release/clawd$|target/release/clawd'
echo '---'
ss -lntp | rg '8787|clawd'
