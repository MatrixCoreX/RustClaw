#!/usr/bin/env bash
set -euo pipefail

RULE_PATH="/etc/polkit-1/rules.d/50-agent-small-screen-networkmanager.rules"
MODE="${1:-install}"

write_rule() {
  local output="$1"
  cat >"$output" <<'EOF'
polkit.addRule(function(action, subject) {
    var allowed = [
        "org.freedesktop.NetworkManager.network-control",
        "org.freedesktop.NetworkManager.settings.modify.own",
        "org.freedesktop.NetworkManager.settings.modify.system",
        "org.freedesktop.NetworkManager.wifi.scan"
    ];
    if (subject.isInGroup("netdev") && allowed.indexOf(action.id) >= 0) {
        return polkit.Result.YES;
    }
});
EOF
}

run_privileged() {
  if [[ "${EUID}" -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

case "$MODE" in
  install)
    tmp_file="$(mktemp)"
    trap 'rm -f "$tmp_file"' EXIT
    write_rule "$tmp_file"
    run_privileged install -o root -g root -m 0644 "$tmp_file" "$RULE_PATH"
    echo "Installed NetworkManager permission rule: $RULE_PATH"
    ;;
  check)
    tmp_file="$(mktemp)"
    trap 'rm -f "$tmp_file"' EXIT
    write_rule "$tmp_file"
    run_privileged cmp -s "$tmp_file" "$RULE_PATH"
    echo "NetworkManager permission rule is current: $RULE_PATH"
    ;;
  remove)
    run_privileged rm -f "$RULE_PATH"
    echo "Removed NetworkManager permission rule: $RULE_PATH"
    ;;
  *)
    echo "Usage: $0 [install|check|remove]" >&2
    exit 2
    ;;
esac
