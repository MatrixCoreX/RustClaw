#!/usr/bin/env bash
# 先拉取远程并变基，再提示推送。解决 "rejected (non-fast-forward)" / "Updates were rejected"
set -e

BRANCH=$(git rev-parse --abbrev-ref HEAD)
REMOTE="${1:-origin}"

echo "当前分支: $BRANCH  远程: $REMOTE/$BRANCH"

STASHED=0
if ! git diff --quiet || ! git diff --cached --quiet || [ -n "$(git status -u --porcelain)" ]; then
  echo "暂存本地未提交修改..."
  git stash push -u -m "sync-push: $(date +%Y%m%d-%H%M%S)"
  STASHED=1
fi

echo "拉取并变基: git pull --rebase $REMOTE $BRANCH"
git pull --rebase "$REMOTE" "$BRANCH"

if [ "$STASHED" -eq 1 ]; then
  echo "恢复暂存..."
  git stash pop
fi

echo ""
echo "已与远程同步。若要推送，执行: git push $REMOTE $BRANCH"
read -r -p "是否现在推送? [y/N] " ans
if [[ "$ans" =~ ^[yY] ]]; then
  git push "$REMOTE" "$BRANCH"
  echo "推送完成。"
fi
