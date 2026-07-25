#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

mkdir -p "$TMP_ROOT/bin" "$TMP_ROOT/state"
touch "$TMP_ROOT/state/tag-ubuntu-x86_64-current"

cat > "$TMP_ROOT/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

args="$*"
state="${MOCK_GH_STATE:?}"

case "$args" in
  *"actions/runs?event=push&status=queued"*)
    printf '101\tubuntu-x86_64-orphan\n'
    printf '102\tubuntu-x86_64-current\n'
    printf '103\tunrelated-tag\n'
    ;;
  *"actions/runs?event=push&status=in_progress"*)
    printf '201\tpi-aarch64-orphan\n'
    ;;
  *"git/ref/tags/ubuntu-x86_64-current"*)
    test -f "$state/tag-ubuntu-x86_64-current"
    ;;
  *"git/ref/tags/ubuntu-x86_64-orphan"|*"git/ref/tags/pi-aarch64-orphan"*)
    exit 1
    ;;
  *"POST repos/owner/repo/actions/runs/101/cancel"*)
    touch "$state/canceled-101"
    ;;
  *"POST repos/owner/repo/actions/runs/201/cancel"*)
    touch "$state/canceled-201"
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
    bash "$SCRIPT_DIR/cancel-orphaned-platform-release-runs.sh"
)"

grep -Fq "Canceling orphaned Release workflow: run=101 tag=ubuntu-x86_64-orphan" <<<"$output"
grep -Fq "Canceling orphaned Release workflow: run=201 tag=pi-aarch64-orphan" <<<"$output"
grep -Fq "Keeping active Release workflow: run=102 tag=ubuntu-x86_64-current" <<<"$output"
test -f "$TMP_ROOT/state/canceled-101"
test -f "$TMP_ROOT/state/canceled-201"
test ! -e "$TMP_ROOT/state/canceled-102"
test ! -e "$TMP_ROOT/state/canceled-103"

echo "CANCEL_ORPHANED_PLATFORM_RELEASE_RUNS_TESTS ok"
