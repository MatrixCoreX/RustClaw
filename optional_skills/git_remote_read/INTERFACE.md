# git_remote_read Interface Spec

## Capability Summary

This optional bundled skill observes one administrator-approved GitHub HTTPS
remote. It can read one exact branch or fetch that branch into one exact local
remote-tracking ref. Public actions never receive credentials; authenticated
actions receive only `github_git_token` through the host credential broker.

The connection profile, repository allowlist, local remote URL and caller's
`expected_remote_url_digest` must all agree. SSH, file transports, URL
rewrites, proxies, credential helpers, redirects, tags, prune and multiple
refspecs are rejected.

## Actions

- `ls_remote_public`: observe one exact public branch.
- `ls_remote_authenticated`: observe one exact private/authenticated branch.
- `fetch_public`: fetch one public branch into
  `refs/remotes/<remote>/<remote_branch>`.
- `fetch_authenticated`: authenticated form of the same bounded fetch.

## Parameters

| Parameter | Required | Type | Description |
|---|---:|---|---|
| `action` | yes | string | One action listed above. |
| `repo` | no | string | Workspace-relative repository selector; default `.`. |
| `connection_id` | yes | string | Administrator-created connection profile. |
| `remote` | yes | string | Local Git remote selector. |
| `remote_branch` | yes | string | Exact branch, never a wildcard or refspec. |
| `expected_remote_url_digest` | yes | string | Approval-time SHA-256 digest of the canonical HTTPS remote URL. |

## Result and Error Contract

Success uses the common single-line JSON response and returns structured
`extra` with the selected connection, owner/repository, branch, URL digest,
observed SHA and timestamp. Fetch additionally returns before/after tracking
SHA, object-store size delta, free-disk evidence, and worktree evidence. It
does not mutate the remote. Fetch checks a fixed free-disk safety floor before
starting and returns `available_disk_bytes_before`.

Errors use `extra.{schema_version,source_skill,status,error_code,message_key,
retryable,failure_phase,side_effect_applied}`. Stable codes include
`git_connection_not_found`, `git_remote_repository_not_allowed`,
`git_repository_config_unsafe`, `git_credentials_missing`,
`git_remote_url_precondition_changed`, `git_fetch_failed`, and
`git_command_timeout`.

## Examples

```json
{"request_id":"read-1","args":{"action":"ls_remote_public","connection_id":"github-main","remote":"origin","remote_branch":"main","expected_remote_url_digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"context":null,"user_id":1,"chat_id":1}
```

```json
{"request_id":"read-1","status":"ok","text":"action=ls_remote_public status=ok","error_text":null,"extra":{"schema_version":1,"source_skill":"git_remote_read","status":"ok","action":"ls_remote_public","effect":"observe","remote_branch":"main","remote_sha":"0123456789abcdef0123456789abcdef01234567","authenticated":false}}
```

```json
{"request_id":"read-2","status":"error","text":"","error_text":"git_remote_url_precondition_changed","extra":{"schema_version":1,"source_skill":"git_remote_read","status":"error","error_code":"git_remote_url_precondition_changed","message_key":"skill.git_remote_read.git_remote_url_precondition_changed","retryable":false}}
```
