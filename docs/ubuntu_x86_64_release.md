# Ubuntu x86_64 Release Package

Install from the signed prebuilt package using the [Release installation guide](release_installation.md). Deployment agents must not compile on the destination. The build/publish steps below are for maintainers only.

Agent Runtime can publish a prebuilt Ubuntu x86_64 runtime package through GitHub
Actions. This package is for regular 64-bit Ubuntu servers and PCs.

## Build And Publish

Preferred release command:

```bash
./release-latest.sh --platform ubuntu
```

It creates and pushes the next `ubuntu-x86_64-YYYYMMDD[-N]` tag. The tag
triggers `Build Ubuntu x86_64 Release` and publishes a normal GitHub Release.
The workflow can also be started manually; set `prerelease=true` only for a
package that must not become the normal update source.

For a signed acceptance candidate without creating or updating a GitHub Release:

```bash
gh workflow run ubuntu-x86_64-release.yml -f publish_release=false
```

Record the returned run's source SHA and download its `agent-runtime-ubuntu-x86_64`
Actions artifact after the complete run succeeds. This mode keeps the same build,
tests, signing, SBOM and provenance checks; it does not deploy, create tags, publish
or clean up Releases. `prerelease=true` alone still publishes a Release and is not
an artifact-only mode. Manual runs default to publishing; explicit tag pushes
also publish as usual.

The workflow builds:

- Rust workspace binaries for `x86_64-unknown-linux-gnu`
- `UI/dist`
- a runtime archive whose name uses the selected product identity's
  `release_artifact_id`, platform and tag
- matching `.sha256`, `.spdx.json`, `.manifest.json` and `.manifest.json.sig` files

The archive is uploaded as a workflow artifact and, when publishing is enabled,
as a GitHub Release asset.

Old Releases are retained unless a maintainer explicitly runs the separate
cleanup workflow. Verify a replacement before removing any platform's older
Release; artifact-only candidate builds never invoke that cleanup.

## Update Notes

When updating an existing install, keep runtime state:

- `configs/`
- `data/`
- `logs/`
- `.pids/`

`data/` includes the main runtime database and every private skill database
under `data/skills/`; preserve the whole directory rather than selecting only
`agent-runtime.db`.

Use the package as a source of updated binaries, scripts, prompts, migrations,
and `UI/dist`. Do not overwrite live secrets or channel settings with packaged
defaults.

The admin Release update path verifies the checksum, preserves runtime
directories, atomically replaces each prebuilt binary so the running `clawd`
executable can be upgraded, and then restarts `clawd`. A systemd-managed Linux
installation schedules that restart in a separate transient unit so stopping
the old service cannot kill its own restart process. If the host already has a
Agent Runtime nginx site, it copies the packaged `UI/dist` into that site without
rebuilding. A local install with no nginx site remains nginx-free.
