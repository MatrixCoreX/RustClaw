#!/usr/bin/env bash
# Keep only the newest published GitHub Release for one RustClaw platform.
set -euo pipefail

usage() {
  echo "Usage: $0 <release-tag-prefix> [--keep-tag TAG] [--dry-run]"
  echo "Example: $0 ubuntu-x86_64- --keep-tag ubuntu-x86_64-20260725"
}

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 64
fi

prefix="$1"
shift
dry_run=0
keep_tag=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep-tag)
      keep_tag="${2:-}"
      shift 2
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    *)
      usage >&2
      exit 64
      ;;
  esac
done
case "$prefix" in
  ubuntu-x86_64-|pi-aarch64-)
    ;;
  *)
    echo "Unsupported release prefix: $prefix" >&2
    exit 64
    ;;
esac
if [[ -n "$keep_tag" && "$keep_tag" != "$prefix"* ]]; then
  echo "Keep tag does not match release prefix: $keep_tag" >&2
  exit 64
fi

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GH_REPO:?GH_REPO is required}"

visibility_retries="${RELEASE_CLEANUP_VISIBILITY_RETRIES:-12}"
retry_seconds="${RELEASE_CLEANUP_RETRY_SECONDS:-5}"
case "$visibility_retries:$retry_seconds" in
  *[!0-9:]*|:*|*:)
    echo "Release cleanup retry settings must be non-negative integers." >&2
    exit 64
    ;;
esac

release_rows="$(mktemp)"
tag_rows="$(mktemp)"
trap 'rm -f "$release_rows" "$tag_rows"' EXIT

fetch_release_rows() {
  gh api --paginate "repos/${GH_REPO}/releases?per_page=100" \
    --jq ".[] | select(.draft == false and (.tag_name | startswith(\"${prefix}\"))) | [.published_at, (.id | tostring), .tag_name] | @tsv" \
    | LC_ALL=C sort -r > "$release_rows"
}

attempt=0
while true; do
  fetch_release_rows
  if [[ -z "$keep_tag" ]] ||
    awk -F $'\t' -v keep="$keep_tag" '$3 == keep { found=1 } END { exit !found }' "$release_rows"; then
    break
  fi
  if ((attempt >= visibility_retries)); then
    echo "New release did not become visible before cleanup: $keep_tag" >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  echo "Waiting for published release visibility: $keep_tag (attempt $attempt/$visibility_retries)"
  sleep "$retry_seconds"
done

if [[ -z "$keep_tag" ]]; then
  keep_tag="$(sed -n '1{s/^[^	]*	[^	]*	//;p;}' "$release_rows")"
fi
if [[ -z "$keep_tag" ]]; then
  echo "No published ${prefix} release exists; skipping cleanup."
  exit 0
fi

echo "Keeping newest ${prefix} release: ${keep_tag}"
while IFS=$'\t' read -r _published_at release_id old_tag; do
  [[ -n "$release_id" && -n "$old_tag" ]] || continue
  [[ "$old_tag" != "$keep_tag" ]] || continue
  if [[ "$dry_run" -eq 1 ]]; then
    echo "Would delete old release and tag: ${old_tag}"
  else
    echo "Deleting old release and tag: ${old_tag}"
    gh api --method DELETE "repos/${GH_REPO}/releases/${release_id}"
    gh api --method DELETE "repos/${GH_REPO}/git/refs/tags/${old_tag}"
  fi
done < "$release_rows"

gh api --paginate "repos/${GH_REPO}/git/matching-refs/tags/${prefix}" \
  --jq '.[].ref' | LC_ALL=C sort -u > "$tag_rows"
while IFS= read -r tag_ref; do
  [[ -n "$tag_ref" ]] || continue
  tag_name="${tag_ref#refs/tags/}"
  [[ "$tag_name" != "$keep_tag" ]] || continue
  if [[ "$dry_run" -eq 1 ]]; then
    echo "Would delete orphaned old tag: ${tag_name}"
  else
    echo "Deleting orphaned old tag: ${tag_name}"
    gh api --method DELETE "repos/${GH_REPO}/git/refs/tags/${tag_name}"
  fi
done < "$tag_rows"

if [[ "$dry_run" -eq 1 ]]; then
  exit 0
fi

attempt=0
while true; do
  remaining_releases="$(
    gh api --paginate "repos/${GH_REPO}/releases?per_page=100" \
      --jq ".[] | select(.draft == false and (.tag_name | startswith(\"${prefix}\"))) | .tag_name"
  )"
  remaining_release_count="$(printf '%s\n' "$remaining_releases" | awk 'NF { count++ } END { print count + 0 }')"
  remaining_tags="$(
    gh api --paginate "repos/${GH_REPO}/git/matching-refs/tags/${prefix}" \
      --jq '.[].ref'
  )"
  remaining_tag_count="$(printf '%s\n' "$remaining_tags" | awk 'NF { count++ } END { print count + 0 }')"
  if [[ "$remaining_release_count" == "1" && "$remaining_releases" == "$keep_tag" &&
    "$remaining_tag_count" == "1" && "$remaining_tags" == "refs/tags/$keep_tag" ]]; then
    break
  fi
  if ((attempt >= visibility_retries)); then
    echo "Release cleanup verification failed for ${prefix}: releases=${remaining_releases}, tags=${remaining_tags}" >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  echo "Waiting for GitHub cleanup consistency (attempt $attempt/$visibility_retries)"
  sleep "$retry_seconds"
done

echo "Cleanup verified: release=${keep_tag}, matching releases=1, matching tags=1"
