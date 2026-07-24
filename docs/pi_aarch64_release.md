# Raspberry Pi aarch64 Release Package

RustClaw can publish a prebuilt Raspberry Pi package through GitHub Actions so
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

The workflow builds:

- Rust workspace binaries for `aarch64-unknown-linux-gnu`
- `UI/dist`
- a runtime archive named `RustClaw-pi-aarch64-<tag>.tar.gz`
  (`RustClaw-<tag>.tar.gz` when the tag already starts with `pi-aarch64-`)
- a matching `.sha256` checksum file

The archive is uploaded both as a workflow artifact and as a GitHub Release
asset.

After a successful publish, the workflow automatically keeps the newest
`pi-aarch64-*` Release and deletes older Pi Releases and their associated tags.
Ubuntu x86_64 Releases use a separate prefix and are not deleted.

## Pi Update Notes

On the Pi, keep runtime state from the existing install:

- `configs/`
- `data/`
- `logs/`
- `.pids/`

`data/` includes the main runtime database and every private skill database
under `data/skills/`; preserve the whole directory rather than selecting only
`rustclaw.db`.

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
the old service cannot kill its own restart process. If an existing RustClaw
nginx site is configured, packaged `UI/dist` is copied without a local UI
build. A local Pi install without nginx is not configured for nginx.
