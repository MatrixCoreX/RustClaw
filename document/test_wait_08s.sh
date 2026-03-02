#!/usr/bin/env bash
set -euo pipefail

target_seconds=8
start_ts=$(date +%s)

while true; do
  now_ts=$(date +%s)
  elapsed=$((now_ts - start_ts))
  if [ "$elapsed" -ge "$target_seconds" ]; then
    echo "测试结束: target=${target_seconds}s elapsed=${elapsed}s"
    exit 0
  fi
  sleep 1
done
