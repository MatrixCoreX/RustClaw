# Base Skill Response Contract

This document defines the response-shape conventions for RustClaw base skills that are intended to be machine-readable.

## Goals

- Preserve backward compatibility through `text`.
- Give new consumers stable structured access through `extra`.
- Keep field names consistent across base skills where semantics overlap.

## Success Response Rules

For covered base skills:

- `status` must be `ok`
- `text` must remain populated for backward compatibility
- `extra` should be non-null
- `extra.action` should be present whenever the skill has action-style dispatch

## Error Response Rules

- `status` must be `error`
- `error_text` must carry the readable failure message
- `extra` should be `null` unless there is a strong reason to expose structured failure metadata

## Preferred `extra` Field Names

Use these names when the semantics exist:

- `action`: canonical action name handled by the skill
- `output`: primary human-readable success payload when the skill is command-like
- `exit_code`: subprocess exit code for shell/CLI-backed skills
- `status_code`: HTTP status code for HTTP-like skills
- `body_preview`: bounded response body preview for HTTP-like skills
- `command`: flattened command string when there is one authoritative command
- `docker_args`: raw docker argv slice for docker-backed skills
- `manager`: detected or selected package manager
- `packages`: package list for package-management skills
- `locale`: selected UI/message locale tag for skills with i18n (e.g. `weather`)
- `mode`: coarse result mode for multi-mode skills (e.g. `weather`: `current` vs `daily`)
- `forecast_days_requested`: user-requested forecast horizon (e.g. `weather`)
- `forecast_days_applied`: forecast horizon actually queried after caps (e.g. `weather`)
- `forecast_days_capped`: `true` when the request exceeded the API cap and was clamped (e.g. `weather`)

## Covered Skills

Current contract-gated base skills:

- `system_basic`
- `fs_search`
- `health_check`
- `process_basic`
- `git_basic`
- `package_manager`
- `archive_basic`

`docker_basic` already follows the same shape on successful responses, but it is not yet gated by the automated contract script because some environments do not have the Docker CLI available.
