#!/usr/bin/env bash
# git add 当前目录，排除 logs 与 db 目录

set -e
cd "$(git rev-parse --show-toplevel)"

# 使用 pathspec 排除 logs 和 db
git add -- . ':!logs' ':!data'

echo "已添加变更（已排除 logs/、data/）"
