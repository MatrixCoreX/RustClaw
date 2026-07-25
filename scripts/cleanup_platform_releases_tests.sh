#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

mkdir -p "$TMP_ROOT/bin" "$TMP_ROOT/state"
touch "$TMP_ROOT/state/old_release" "$TMP_ROOT/state/old_tag" "$TMP_ROOT/state/orphan_tag"

cat > "$TMP_ROOT/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

args="$*"
state="${MOCK_GH_STATE:?}"

if [[ "$args" == *"releases?per_page=100"* ]]; then
  if [[ "$args" == *"@tsv"* ]]; then
    printf '2026-07-25T08:00:00Z\t200\tubuntu-x86_64-new\n'
    if [[ -f "$state/old_release" ]]; then
      printf '2026-07-24T08:00:00Z\t100\tubuntu-x86_64-old\n'
    fi
  else
    printf 'ubuntu-x86_64-new\n'
    if [[ -f "$state/old_release" ]]; then
      printf 'ubuntu-x86_64-old\n'
    fi
  fi
  exit 0
fi

if [[ "$args" == *"matching-refs/tags/ubuntu-x86_64-"* ]]; then
  printf 'refs/tags/ubuntu-x86_64-new\n'
  [[ ! -f "$state/old_tag" ]] || printf 'refs/tags/ubuntu-x86_64-old\n'
  [[ ! -f "$state/orphan_tag" ]] || printf 'refs/tags/ubuntu-x86_64-orphan\n'
  exit 0
fi

case "$args" in
  *"DELETE repos/owner/repo/releases/100"*)
    rm -f "$state/old_release"
    ;;
  *"DELETE repos/owner/repo/git/refs/tags/ubuntu-x86_64-old"*)
    rm -f "$state/old_tag"
    ;;
  *"DELETE repos/owner/repo/git/refs/tags/ubuntu-x86_64-orphan"*)
    rm -f "$state/orphan_tag"
    ;;
  *)
    echo "Unexpected mock gh call: $args" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$TMP_ROOT/bin/gh"

output="$(
  GH_TOKEN=test \
  GH_REPO=owner/repo \
  MOCK_GH_STATE="$TMP_ROOT/state" \
  PATH="$TMP_ROOT/bin:$PATH" \
    bash "$SCRIPT_DIR/cleanup-platform-releases.sh" \
      "ubuntu-x86_64-" \
      --keep-tag "ubuntu-x86_64-new"
)"

grep -Fq "Keeping newest ubuntu-x86_64- release: ubuntu-x86_64-new" <<<"$output"
grep -Fq "Deleting old release and tag: ubuntu-x86_64-old" <<<"$output"
grep -Fq "Deleting orphaned old tag: ubuntu-x86_64-orphan" <<<"$output"
grep -Fq "Cleanup verified: release=ubuntu-x86_64-new" <<<"$output"

for stale in old_release old_tag orphan_tag; do
  if [[ -e "$TMP_ROOT/state/$stale" ]]; then
    echo "Cleanup left stale mock state: $stale" >&2
    exit 1
  fi
done

echo "CLEANUP_PLATFORM_RELEASES_TESTS ok"
