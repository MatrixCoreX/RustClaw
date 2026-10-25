# Application browser authentication

Distributed deliveries use task-scoped storage and delete task profiles, browser
caches, downloaded media, and results after acknowledgement. An application can
separately retain authentication only when the node owner grants the exact
application origin and capability in the node's host-owned work policy:

```json
{
  "browser_authentication_grants": [
    {"origin": "https://media.example", "capability": "media_download.download"}
  ]
}
```

This optional field belongs to `data/nni/work/policy.json`, or the policy selected
by `APP_NNI_WORK_POLICY_PATH`. Existing policy fields remain required. Both values
must already occur in `artifact_origins` and `capabilities`. Artifact cleanup
permission alone does not grant authentication storage. Deploy a runtime that
supports this field before adding it; older runtimes reject unknown policy fields.

After verifying the enrolled task against the host ledger, the storage resolver
adds `context.skill_storage.browser_authentication`, with schema version 1, an
absolute sandbox-mapped directory, and a scope identifier. The identifier is the
SHA-256 digest of principal ID, origin, and capability separated by newlines. The
directory is under the owning skill's private storage at
`application-authentication/<scope>`. Only this directory receives an additional
writable sandbox mount. Task arguments cannot request a directory or grant.
Removing the grant stops subsequent tasks from receiving it without deleting the
saved authentication. Ordinary Agent descriptors omit this field.

The media downloader consumes the descriptor only for application downloads with
`public_only=true`. Its wrapper removes inherited authentication-store variables
and supplies internal child variables solely from that host descriptor. For
WeChat Channels, Chromium restores and captures URL-scoped Yuanbao cookies using
CDP. The vault stores no browser profile, local storage, cache, media, screenshot,
or task history. Directory permissions are 0700 and files 0600; a process lock
serializes updates. Writes are bounded and atomic; symlink/hardlink reads and
scope/origin mismatches fail explicitly. Personal/ordinary Agent sessions are not
imported. A dedicated initial login may still be required.

Safe `extra.browser_authentication` metadata contains status, last verification
and check timestamps, earliest cookie expiry, and whether session cookies exist.
Times use Unix seconds. A successful platform parse verifies the login at that
time, then captures cookies. Expired cookies are never restored or extended.
Session cookies may have no known expiry; a cookie's date does not guarantee the
whole account session remains valid. Platform login rejection clears the saved
cookies and marks reauthentication required. Network timeouts do not clear them.
The first version covers Yuanbao authentication for Channels; other platform
profiles retain their existing behavior. No model call is involved.

The Pi companion also reads short-lived `login_challenge` records from validated
UUID task directories under `.agent-runtime/artifacts/task-storage`. It reads
only the existing challenge metadata and PNG, rejects symlinked task/skill paths,
and never lists task results or opens media. Public browser profiles publish
their challenge in their own task-scoped storage. The existing localized QR
overlay refreshes while login is pending and closes when the browser clears the
record, including success, cancellation, or timeout. Stale records are ignored
within the existing 15-second freshness window and removed by task cleanup. On
a Pi with a live companion, the Channels login browser renders headlessly behind
the screen; the official Yuanbao sign-in control is opened and only the square
WeChat QR region is captured. Other platforms and non-companion desktop browsers
keep their existing behavior. The localized prompt names Tencent Yuanbao so the
account authorization is clear.

Validation: the media downloader unit suite covers vault isolation, expiry,
revocation, secret-free metadata, and ordinary-call isolation; runtime tests cover
host grants, sandbox mapping, and cleanup preservation. The optional real-browser
probe runs with:

```sh
python3 optional_skills/media_download/tests/browser_authentication_chromium_probe.py --chrome /usr/bin/chromium
```

The probe uses synthetic cookies in fresh temporary browser profiles. It proves
CDP persistence across browser restarts without accessing a platform account; a
real account login and download remain separate integration checks.
