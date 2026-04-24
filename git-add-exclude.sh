#!/usr/bin/env bash
# git add 当前目录，排除 logs 与 db 目录

set -e
cd "$(git rev-parse --show-toplevel)"

# 使用 pathspec 排除 logs 和 db
git add -- . ':!logs' ':!data'

# zh: 告知维护者已执行 git add，并明确排除了日志和数据目录。
echo "Changes added (excluding logs/ and data/)."
