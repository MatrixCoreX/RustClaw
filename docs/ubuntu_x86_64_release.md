# Ubuntu x86_64 Release Package

RustClaw can publish a prebuilt Ubuntu x86_64 runtime package through GitHub
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

The workflow builds:

- Rust workspace binaries for `x86_64-unknown-linux-gnu`
- `UI/dist`
- a runtime archive named `RustClaw-ubuntu-x86_64-<tag>.tar.gz`
  (`RustClaw-<tag>.tar.gz` when the tag already starts with `ubuntu-x86_64-`)
- a matching `.sha256` checksum file

The archive is uploaded both as a workflow artifact and as a GitHub Release
asset.

After a successful publish, the workflow automatically keeps the newest
`ubuntu-x86_64-*` Release and deletes older Ubuntu Releases and their associated
tags. Raspberry Pi Releases use a separate prefix and are not deleted.

## Update Notes

When updating an existing install, keep runtime state:

- `configs/`
- `data/`
- `logs/`
- `.pids/`

Use the package as a source of updated binaries, scripts, prompts, migrations,
and `UI/dist`. Do not overwrite live secrets or channel settings with packaged
defaults.

The admin Release update path verifies the checksum, preserves runtime
directories, replaces prebuilt files, and restarts `clawd`. If the host already
has a RustClaw nginx site, it copies the packaged `UI/dist` into that site
without rebuilding. A local install with no nginx site remains nginx-free.
