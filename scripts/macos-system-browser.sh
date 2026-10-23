#!/bin/bash
set -euo pipefail

# Launch the signed application by its real path: a binary symlink breaks
# Chrome's relative Framework lookup. Never open a personal browser profile.
for browser in chrome edge; do
  case "$browser" in
    chrome)
      app_name='Google Chrome.app'
      executable_name='Google Chrome'
      bundle_id='com.google.Chrome'
      team='EQHXZ8M8AV'
      ;;
    edge)
      app_name='Microsoft Edge.app'
      executable_name='Microsoft Edge'
      bundle_id='com.microsoft.edgemac'
      team='UBF8T346G9'
      ;;
  esac
  for directory in "${HOME:?}/Applications" /Applications; do
    app="$directory/$app_name"
    executable="$app/Contents/MacOS/$executable_name"
    [[ -x "$executable" ]] || continue
    plist="$app/Contents/Info.plist"
    identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist" 2>/dev/null)" || continue
    [[ "$identifier" == "$bundle_id" ]] || continue
    version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist" 2>/dev/null)" || continue
    major="${version%%.*}"
    [[ "$major" =~ ^[0-9]+$ ]] && (( 10#$major >= 120 )) || continue
    # An unprefixed codesign requirement is interpreted as a filename.
    requirement="=anchor apple generic and identifier \"$bundle_id\" and certificate leaf[subject.OU] = \"$team\""
    /usr/bin/codesign --verify --strict -R "$requirement" "$app" 2>/dev/null || continue
    if [[ "${1:-}" == --resolve-executable ]]; then
      printf '%s\n' "$executable"
      exit 0
    fi
    exec "$executable" "$@"
  done
done
printf '%s\n' '{"error_code":"system_browser_unavailable","platform":"macos","supported_browsers":["google_chrome","microsoft_edge"]}' >&2
exit 127
