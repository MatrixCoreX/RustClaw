# Raspberry Pi aarch64 Release Package

Install from the signed prebuilt package using the [Release installation guide](release_installation.md). Do not compile on the Pi during installation. The build/publish steps below are for maintainers only.

Agent Runtime can publish a prebuilt Raspberry Pi package through GitHub Actions so
the Pi does not need to run `cargo build`.

## Build And Publish

Preferred release command:

```bash
./release-latest.sh --platform pi
```

It creates and pushes the next `pi-aarch64-YYYYMMDD[-N]` tag. The tag triggers
`Build Pi aarch64 Release` and publishes a normal GitHub Release. The workflow
can also be started manually; set `prerelease=true` only for a package that
must not become the normal update source.

For a signed acceptance candidate without creating or updating a GitHub Release:

```bash
gh workflow run pi-aarch64-release.yml -f publish_release=false
```

Record the returned run's source SHA and download its `agent-runtime-pi-aarch64`
Actions artifact after the complete run succeeds. All cross-build, dependency,
receipt, signing, SBOM and provenance checks still run. Candidate mode does not
deploy, create tags, publish or clean up Releases. `prerelease=true` alone still
publishes a Release. Manual runs default to publishing; explicit tag pushes
also publish as usual.

The workflow builds:

- Rust workspace binaries for `aarch64-unknown-linux-gnu`
- `UI/dist`
- a runtime archive whose name uses the selected product identity's
  `release_artifact_id`, platform and tag
- matching `.sha256`, `.spdx.json`, `.manifest.json` and `.manifest.json.sig` files

The archive is uploaded as a workflow artifact and, when publishing is enabled,
as a GitHub Release asset.

Old Releases are retained unless a maintainer explicitly runs the separate
cleanup workflow. Verify a replacement before removing any platform's older
Release; artifact-only candidate builds never invoke that cleanup.

## Pi Update Notes

On the Pi, keep runtime state from the existing install:

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

After replacing files, restart the needed services. For a backend-only update,
restart `clawd`; for a full runtime update, restart the selected channel
adapters too.

The admin Release update path verifies the checksum, preserves runtime
directories, atomically replaces each prebuilt binary so the running `clawd`
executable can be upgraded, and then restarts `clawd`. A systemd-managed Linux
installation schedules that restart in a separate transient unit so stopping
the old service cannot kill its own restart process. If an existing Agent Runtime
nginx site is configured, packaged `UI/dist` is copied without a local UI
build. A local Pi install without nginx is not configured for nginx.
