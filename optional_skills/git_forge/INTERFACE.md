# git_forge Interface Spec

## Capability Summary

This optional bundled GitHub v1 skill creates and observes pull requests. The
repository and head commit are not caller-selected URLs: they are derived from
a digest-bound `git_remote_publish` receipt and reverified against both the
current connection profile and remote branch before every API request.

The API host is fixed to `api.github.com`, redirects are disabled, responses
and pagination are bounded, and only `github_api_token` is sent to the API.
The Git receipt check separately uses `github_git_token`. Neither credential
is returned in output.

## Actions

- `create_pr`: create a PR after one-time confirmation; on GitHub 422, perform
  an exact structured lookup instead of parsing the response message.
- `list_prs`: bounded, truly paginated PR observation for the receipt branch.
- `pr_status`: PR state plus both check-runs and commit statuses.
- `reconcile_create_pr`: exact head/base/SHA lookup returning `applied`,
  `not_applied`, or `still_unknown`; never creates a PR.

## Parameters

| Parameter | Required | Type | Description |
|---|---:|---|---|
| `action` | yes | string | One action listed above. |
| `connection_id` | yes | string | Must match the push receipt. |
| `push_receipt_ref` | yes | string | Digest-bound verified-push result reference. |
| `expected_head_sha` | create/reconcile | string | Must equal the verified pushed SHA. |
| `head` | create/reconcile | string | Must equal the pushed remote branch. |
| `base` | create/reconcile | string | Exact base branch. |
| `title` | create | string | 1..256 characters; secret-scanned. |
| `body` | create | string | Up to 64 KiB; secret-scanned. |
| `draft` | create | boolean | GitHub draft state. |
| `state` | list | string | `open`, `closed`, or `all`. |
| `number` | status | integer | Positive PR number. |

## Result and Error Contract

Create returns the generic external mutation receipt/evidence fields plus a
bounded PR projection. Status returns `mergeable` (including normal null while
GitHub is computing it), a combined checks summary, and bounded source lists.
All results include rate-limit metadata without exposing response headers that
could contain unrelated data.

Errors use the canonical structured error envelope. Stable codes include
`forge_credentials_missing`, `forge_content_secret_detected`,
`forge_api_authentication_failed`, `forge_api_permission_denied`,
`forge_api_rate_limited`, `forge_api_redirect_rejected`,
`forge_api_validation_failed`, `forge_head_precondition_changed`, and
`forge_pr_reconciliation_ambiguous`.

## Examples

```json
{"request_id":"pr-1","args":{"action":"create_pr","connection_id":"github-main","push_receipt_ref":"git-push-v1:payload:digest","expected_head_sha":"0123456789abcdef0123456789abcdef01234567","head":"delivery","base":"main","title":"Delivery closure","body":"Validated locally.","draft":true},"context":null,"user_id":1,"chat_id":1}
```

```json
{"request_id":"pr-1","status":"ok","text":"action=create_pr status=applied","error_text":null,"extra":{"schema_version":1,"source_skill":"git_forge","status":"applied","action":"create_pr","effect":"external","action_ref":"forge.create_pr","reversible":false,"pull_request":{"number":7,"state":"open","head":"delivery","base":"main"}}}
```

```json
{"request_id":"pr-s1","args":{"action":"pr_status","connection_id":"github-main","push_receipt_ref":"git-push-v1:payload:digest","number":7},"context":null,"user_id":1,"chat_id":1}
```
