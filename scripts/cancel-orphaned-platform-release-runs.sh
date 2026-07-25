#!/usr/bin/env bash
# Cancel queued/running platform Release workflows whose trigger tag was deleted.
set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GH_REPO:?GH_REPO is required}"
command -v gh >/dev/null 2>&1 || {
  echo "gh CLI is required" >&2
  exit 1
}

for status in queued in_progress; do
  while IFS=$'\t' read -r run_id head_branch; do
    [[ "$run_id" =~ ^[0-9]+$ ]] || continue
    case "$head_branch" in
      ubuntu-x86_64-*|pi-aarch64-*)
        ;;
      *)
        continue
        ;;
    esac

    if gh api "repos/${GH_REPO}/git/ref/tags/${head_branch}" >/dev/null 2>&1; then
      echo "Keeping active Release workflow: run=${run_id} tag=${head_branch}"
      continue
    fi

    echo "Canceling orphaned Release workflow: run=${run_id} tag=${head_branch}"
    gh api --method POST "repos/${GH_REPO}/actions/runs/${run_id}/cancel" >/dev/null
  done < <(
    gh api --paginate \
      "repos/${GH_REPO}/actions/runs?event=push&status=${status}&per_page=100" \
      --jq '.workflow_runs[] | [(.id | tostring), .head_branch] | @tsv'
  )
done
