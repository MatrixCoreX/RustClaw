# git_remote_publish Interface Spec

## Capability Summary

This built-in skill publishes exactly one approved local commit object to
exactly one allowlisted GitHub branch over HTTPS. It never derives the source
from the branch at execution time and does not support force, delete, tags,
mirror, wildcard or multiple refspecs. Every push requires one-time
confirmation and a separately scoped `github_git_token`.

## Actions

- `push`: compare all approval-time preconditions, push
  `<expected_local_sha>:refs/heads/<remote_branch>`, and read the remote SHA
  back before reporting success.
- `reconcile_push`: read the same remote branch and report `applied`,
  `not_applied`, or `still_unknown`; it never pushes.

## Parameters

| Parameter | Required | Type | Description |
|---|---:|---|---|
| `action` | yes | string | `push` or `reconcile_push`. |
| `repo` | no | string | Workspace-relative repository selector. |
| `connection_id` | yes | string | Approved connection profile. |
| `remote` | yes | string | Local remote selector. |
| `local_branch` | push | string | Exact local branch checked against the approved SHA. |
| `remote_branch` | yes | string | Exact target branch. |
| `expected_local_sha` | yes | string | Full 40/64-hex object ID fixed by approval. |
| `expected_remote_sha` | push/reconcile | string or null | Remote CAS value; null explicitly means absent. |
| `expected_remote_url_digest` | yes | string | Digest of the canonical, credential-free HTTPS URL. |
| `set_upstream` | push | boolean | Update local tracking only after remote verification. |

## Result and Error Contract

A verified push returns the generic mutation evidence fields
`operation_id/action_ref/target_ref/result_ref/status/reversible/evidence_digest`
plus before/after remote SHAs, worktree digest, `forced=false`, and upstream
status. `result_ref` is a digest-bound push receipt for `git_forge`.

Errors use the canonical structured error envelope. Stable codes include
`git_push_precondition_changed`, `git_non_fast_forward`,
`git_remote_rejected`, `git_push_postcondition_uncertain`,
`git_repository_config_unsafe`, and `git_command_timeout`. A failure after
dispatch is conservatively marked for reconciliation and is never blindly
replayed.

## Examples

```json
{"request_id":"push-1","args":{"action":"push","connection_id":"github-main","remote":"origin","local_branch":"delivery","remote_branch":"delivery","expected_local_sha":"0123456789abcdef0123456789abcdef01234567","expected_remote_sha":null,"expected_remote_url_digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","set_upstream":true},"context":null,"user_id":1,"chat_id":1}
```

```json
{"request_id":"push-1","status":"ok","text":"action=push status=applied","error_text":null,"extra":{"schema_version":1,"source_skill":"git_remote_publish","status":"applied","action":"push","effect":"external","expected_local_sha":"0123456789abcdef0123456789abcdef01234567","remote_sha_after":"0123456789abcdef0123456789abcdef01234567","forced":false,"reversible":false}}
```

```json
{"request_id":"push-r1","args":{"action":"reconcile_push","connection_id":"github-main","remote":"origin","remote_branch":"delivery","expected_local_sha":"0123456789abcdef0123456789abcdef01234567","expected_remote_sha":null,"expected_remote_url_digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"context":null,"user_id":1,"chat_id":1}
```
