#!/usr/bin/env bash
# Keep only the newest published GitHub Release for one RustClaw platform.
set -euo pipefail

usage() {
  echo "Usage: $0 <release-tag-prefix> [--dry-run]"
  echo "Example: $0 ubuntu-x86_64- --dry-run"
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage >&2
  exit 64
fi

prefix="$1"
dry_run=0
if [[ "${2:-}" == "--dry-run" ]]; then
  dry_run=1
elif [[ -n "${2:-}" ]]; then
  usage >&2
  exit 64
fi
case "$prefix" in
  ubuntu-x86_64-|pi-aarch64-)
    ;;
  *)
    echo "Unsupported release prefix: $prefix" >&2
    exit 64
    ;;
esac

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GH_REPO:?GH_REPO is required}"

release_rows="$(mktemp)"
trap 'rm -f "$release_rows"' EXIT

gh api --paginate "repos/${GH_REPO}/releases?per_page=100" \
  --jq ".[] | select(.draft == false and (.tag_name | startswith(\"${prefix}\"))) | [.created_at, .tag_name] | @tsv" \
  | LC_ALL=C sort -r > "$release_rows"

keep_tag="$(sed -n '1{s/^[^	]*	//;p;}' "$release_rows")"
if [[ -z "$keep_tag" ]]; then
  echo "No published ${prefix} release exists; skipping cleanup."
  exit 0
fi

echo "Keeping newest ${prefix} release: ${keep_tag}"
sed -n '2,$p' "$release_rows" | while IFS=$'\t' read -r _created_at old_tag; do
  [[ -n "$old_tag" ]] || continue
  if [[ "$dry_run" -eq 1 ]]; then
    echo "Would delete old release and tag: ${old_tag}"
  else
    echo "Deleting old release and tag: ${old_tag}"
    gh release delete "$old_tag" --cleanup-tag --yes
  fi
done
