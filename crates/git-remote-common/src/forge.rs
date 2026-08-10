use std::io::Read as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use reqwest::blocking::{Client, Response};
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER, USER_AGENT,
};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::git::{validated_branch, validated_sha, verify_push_receipt, VerifiedPushReceipt};
use super::{optional_bool, optional_string, required_string, SkillError};

const GITHUB_API_TOKEN_ENV: &str = "GITHUB_API_TOKEN";
const MAX_API_BODY_BYTES: usize = 1024 * 1024;
const MAX_TITLE_CHARS: usize = 256;
const MAX_PR_BODY_BYTES: usize = 64 * 1024;
const MAX_API_PAGES: usize = 4;
const API_PAGE_SIZE: usize = 100;

#[derive(Debug)]
struct GithubClient {
    client: Client,
    token: String,
    base_url: String,
}

#[derive(Debug)]
struct ApiDocument {
    value: Value,
    rate_limit: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PullRequestReceipt {
    schema_version: u32,
    connection_id: String,
    owner: String,
    repository: String,
    number: u64,
    head: String,
    base: String,
    head_sha: String,
    html_url: String,
}

pub fn execute_git_forge(args: &Map<String, Value>) -> Result<Value, SkillError> {
    let action = required_string(args, "action", "forge_action_missing")?;
    if !matches!(
        action,
        "create_pr" | "list_prs" | "pr_status" | "reconcile_create_pr"
    ) {
        return Err(SkillError::new("unsupported_action"));
    }
    let push_receipt_ref =
        required_string(args, "push_receipt_ref", "forge_push_receipt_ref_missing")?;
    let verified = verify_push_receipt(args, push_receipt_ref)?;
    let client = GithubClient::new()?;
    match action {
        "create_pr" => create_pr(args, &verified, &client),
        "list_prs" => list_prs(args, &verified, &client),
        "pr_status" => pr_status(args, &verified, &client),
        "reconcile_create_pr" => reconcile_create_pr(args, &verified, &client),
        _ => unreachable!(),
    }
}

impl GithubClient {
    fn new() -> Result<Self, SkillError> {
        let token = claw_core::secrets::env_non_empty_resolved_or_none(GITHUB_API_TOKEN_ENV)
            .ok_or_else(|| SkillError::new("forge_credentials_missing"))?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| SkillError::new("forge_client_unavailable"))?;
        Ok(Self {
            client,
            token,
            base_url: "https://api.github.com".to_string(),
        })
    }

    #[cfg(test)]
    fn new_for_test(base_url: String, token: &str) -> Result<Self, SkillError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| SkillError::new("forge_client_unavailable"))?;
        Ok(Self {
            client,
            token: token.to_string(),
            base_url,
        })
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        mutation: bool,
    ) -> Result<ApiDocument, SkillError> {
        if !path.starts_with('/') || path.contains("..") || path.chars().any(char::is_control) {
            return Err(SkillError::new("forge_api_path_invalid"));
        }
        let url = format!("{}{path}", self.base_url);
        let mut request = self
            .client
            .request(method, url)
            .header(USER_AGENT, "agent-runtime-git-forge/1")
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", self.token))
                    .map_err(|_| SkillError::new("forge_credentials_invalid"))?,
            )
            .query(query);
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").json(body);
        }
        let response = request.send().map_err(|_| {
            let error = SkillError::new("forge_api_request_failed")
                .phase("dispatch")
                .retryable(true);
            if mutation {
                error.applied(true)
            } else {
                error
            }
        })?;
        parse_response(response, mutation)
    }
}

fn create_pr(
    args: &Map<String, Value>,
    verified: &VerifiedPushReceipt,
    client: &GithubClient,
) -> Result<Value, SkillError> {
    let expected_head_sha = validated_sha(required_string(
        args,
        "expected_head_sha",
        "forge_expected_head_sha_missing",
    )?)?;
    if expected_head_sha != verified.receipt.local_sha {
        return Err(SkillError::new("forge_head_precondition_changed"));
    }
    let head = validated_branch(
        &verified.context.repository_root,
        required_string(args, "head", "forge_head_missing")?,
    )?;
    if head != verified.receipt.remote_branch {
        return Err(SkillError::new("forge_head_receipt_mismatch"));
    }
    let base = validated_branch(
        &verified.context.repository_root,
        required_string(args, "base", "forge_base_missing")?,
    )?;
    let title = validated_title(required_string(args, "title", "forge_title_missing")?)?;
    let body = validated_body(required_string(args, "body", "forge_body_missing")?)?;
    reject_secret_content(title, &client.token)?;
    reject_secret_content(body, &client.token)?;
    if !args.contains_key("draft") {
        return Err(SkillError::new("forge_draft_missing"));
    }
    let draft = optional_bool(args, "draft", false)?;
    let payload = json!({"title": title, "body": body, "head": head, "base": base, "draft": draft});
    let path = repository_path(verified, "pulls");
    match client.request(Method::POST, &path, &[], Some(&payload), true) {
        Ok(document) => pr_mutation_result(
            "create_pr",
            "applied",
            verified,
            &head,
            &base,
            &expected_head_sha,
            &document.value,
            document.rate_limit,
        ),
        Err(error) if error.code == "forge_api_validation_failed" => {
            let matches = find_matching_prs(client, verified, &head, &base, "all")?;
            if matches.len() == 1 {
                pr_mutation_result(
                    "create_pr",
                    "already_applied",
                    verified,
                    &head,
                    &base,
                    &expected_head_sha,
                    &matches[0],
                    json!({}),
                )
            } else if matches.is_empty() {
                Err(error)
            } else {
                Err(SkillError::new("forge_pr_reconciliation_ambiguous")
                    .phase("postcondition")
                    .applied(true))
            }
        }
        Err(error) => Err(error),
    }
}

fn list_prs(
    args: &Map<String, Value>,
    verified: &VerifiedPushReceipt,
    client: &GithubClient,
) -> Result<Value, SkillError> {
    let state = optional_string(args, "state", "forge_state_invalid")?.unwrap_or("open");
    if !matches!(state, "open" | "closed" | "all") {
        return Err(SkillError::new("forge_state_invalid"));
    }
    let head = optional_string(args, "head", "forge_head_invalid")?;
    if let Some(head) = head {
        let head = validated_branch(&verified.context.repository_root, head)?;
        if head != verified.receipt.remote_branch {
            return Err(SkillError::new("forge_head_receipt_mismatch"));
        }
    }
    let (documents, truncated, rate_limit) = paginated_get(
        client,
        &repository_path(verified, "pulls"),
        vec![
            ("state", state.to_string()),
            (
                "head",
                format!(
                    "{}:{}",
                    verified.target.owner, verified.receipt.remote_branch
                ),
            ),
        ],
    )?;
    let pull_requests = documents
        .iter()
        .map(pr_projection)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "action": "list_prs",
        "effect": "observe",
        "connection_id": verified.profile.id,
        "owner": verified.target.owner,
        "repository": verified.target.repository,
        "pull_requests": pull_requests,
        "count": pull_requests.len(),
        "truncated": truncated,
        "rate_limit": rate_limit,
        "observed_at": epoch_seconds(),
    }))
}

fn pr_status(
    args: &Map<String, Value>,
    verified: &VerifiedPushReceipt,
    client: &GithubClient,
) -> Result<Value, SkillError> {
    let number = required_u64(args, "number", "forge_pr_number_missing")?;
    let pr_path = repository_path(verified, &format!("pulls/{number}"));
    let pr = client.request(Method::GET, &pr_path, &[], None, false)?;
    let projection = pr_projection(&pr.value)?;
    let head_sha = value_string(&pr.value, "/head/sha", "forge_api_response_invalid")?;
    let head_sha = validated_sha(head_sha)?;
    if head_sha != verified.receipt.local_sha {
        return Err(SkillError::new("forge_head_precondition_changed"));
    }
    let (check_runs, checks_truncated, checks_rate) = paginated_get(
        client,
        &repository_path(verified, &format!("commits/{head_sha}/check-runs")),
        Vec::new(),
    )?;
    let (statuses, statuses_truncated, statuses_rate) = paginated_get(
        client,
        &repository_path(verified, &format!("commits/{head_sha}/statuses")),
        Vec::new(),
    )?;
    let check_summary = summarize_checks(&check_runs, &statuses);
    Ok(json!({
        "action": "pr_status",
        "effect": "observe",
        "connection_id": verified.profile.id,
        "owner": verified.target.owner,
        "repository": verified.target.repository,
        "pull_request": projection,
        "head_sha": head_sha,
        "checks": check_summary,
        "check_runs": check_runs.iter().map(check_run_projection).collect::<Vec<_>>(),
        "commit_statuses": statuses.iter().map(status_projection).collect::<Vec<_>>(),
        "truncated": checks_truncated || statuses_truncated,
        "rate_limit": {"pull_request": pr.rate_limit, "checks": checks_rate, "statuses": statuses_rate},
        "observed_at": epoch_seconds(),
    }))
}

fn reconcile_create_pr(
    args: &Map<String, Value>,
    verified: &VerifiedPushReceipt,
    client: &GithubClient,
) -> Result<Value, SkillError> {
    let expected_head_sha = validated_sha(required_string(
        args,
        "expected_head_sha",
        "forge_expected_head_sha_missing",
    )?)?;
    if expected_head_sha != verified.receipt.local_sha {
        return Err(SkillError::new("forge_head_precondition_changed"));
    }
    let head = validated_branch(
        &verified.context.repository_root,
        required_string(args, "head", "forge_head_missing")?,
    )?;
    if head != verified.receipt.remote_branch {
        return Err(SkillError::new("forge_head_receipt_mismatch"));
    }
    let base = validated_branch(
        &verified.context.repository_root,
        required_string(args, "base", "forge_base_missing")?,
    )?;
    let matches = find_matching_prs(client, verified, &head, &base, "all")?;
    let exact = matches
        .iter()
        .filter(|value| {
            value.pointer("/head/sha").and_then(Value::as_str) == Some(expected_head_sha.as_str())
        })
        .collect::<Vec<_>>();
    let disposition = match exact.len() {
        1 => "applied",
        0 if matches.is_empty() => "not_applied",
        _ => "still_unknown",
    };
    let (pull_request, result_ref, operation_id, target_ref, after_version) =
        if let [value] = exact.as_slice() {
            let number = value_u64(value, "/number", "forge_api_response_invalid")?;
            let html_url = value_string(value, "/html_url", "forge_api_response_invalid")?;
            validate_github_html_url(
                html_url,
                &verified.target.owner,
                &verified.target.repository,
                number,
            )?;
            let receipt = PullRequestReceipt {
                schema_version: 1,
                connection_id: verified.profile.id.clone(),
                owner: verified.target.owner.clone(),
                repository: verified.target.repository.clone(),
                number,
                head: head.clone(),
                base: base.clone(),
                head_sha: expected_head_sha.clone(),
                html_url: html_url.to_string(),
            };
            let result_ref = encode_pr_receipt(&receipt)?;
            (
                pr_projection(value)?,
                Some(result_ref.clone()),
                Some(deterministic_operation_id(&result_ref)),
                Some(format!(
                    "{}/{}#pull/{number}",
                    verified.target.owner, verified.target.repository
                )),
                Some(expected_head_sha.clone()),
            )
        } else {
            (Value::Null, None, None, None, None)
        };
    let evidence = json!({
        "disposition": disposition,
        "head": head,
        "base": base,
        "expected_head_sha": expected_head_sha,
        "observed_count": matches.len(),
    });
    Ok(json!({
        "action": "reconcile_create_pr",
        "effect": "observe",
        "disposition": disposition,
        "connection_id": verified.profile.id,
        "owner": verified.target.owner,
        "repository": verified.target.repository,
        "head": head,
        "base": base,
        "expected_head_sha": expected_head_sha,
        "pull_request": pull_request,
        "observed_count": matches.len(),
        "operation_id": operation_id,
        "action_ref": "forge.create_pr",
        "target_ref": target_ref,
        "before_version": Value::Null,
        "after_version": after_version,
        "result_ref": result_ref,
        "reversible": false,
        "evidence_digest": evidence_digest(&evidence),
        "observed_at": epoch_seconds(),
    }))
}

fn pr_mutation_result(
    action: &str,
    status: &str,
    verified: &VerifiedPushReceipt,
    head: &str,
    base: &str,
    head_sha: &str,
    value: &Value,
    rate_limit: Value,
) -> Result<Value, SkillError> {
    let number = value_u64(value, "/number", "forge_api_response_invalid")?;
    let html_url = value_string(value, "/html_url", "forge_api_response_invalid")?;
    validate_github_html_url(
        html_url,
        &verified.target.owner,
        &verified.target.repository,
        number,
    )?;
    let observed_head_sha = validated_sha(value_string(
        value,
        "/head/sha",
        "forge_api_response_invalid",
    )?)?;
    if observed_head_sha != head_sha {
        return Err(SkillError::new("forge_pr_postcondition_uncertain")
            .phase("postcondition")
            .applied(true));
    }
    let receipt = PullRequestReceipt {
        schema_version: 1,
        connection_id: verified.profile.id.clone(),
        owner: verified.target.owner.clone(),
        repository: verified.target.repository.clone(),
        number,
        head: head.to_string(),
        base: base.to_string(),
        head_sha: head_sha.to_string(),
        html_url: html_url.to_string(),
    };
    let result_ref = encode_pr_receipt(&receipt)?;
    let evidence = json!({
        "number": number,
        "head": head,
        "base": base,
        "head_sha": head_sha,
        "html_url": html_url,
    });
    Ok(json!({
        "action": action,
        "effect": "external",
        "status": status,
        "operation_id": deterministic_operation_id(&result_ref),
        "action_ref": "forge.create_pr",
        "target_ref": format!("{}/{}#pull/{number}", verified.target.owner, verified.target.repository),
        "before_version": Value::Null,
        "after_version": head_sha,
        "result_ref": result_ref,
        "reversible": false,
        "evidence_digest": evidence_digest(&evidence),
        "connection_id": verified.profile.id,
        "owner": verified.target.owner,
        "repository": verified.target.repository,
        "pull_request": pr_projection(value)?,
        "rate_limit": rate_limit,
        "observed_at": epoch_seconds(),
    }))
}

fn find_matching_prs(
    client: &GithubClient,
    verified: &VerifiedPushReceipt,
    head: &str,
    base: &str,
    state: &str,
) -> Result<Vec<Value>, SkillError> {
    let (documents, truncated, _) = paginated_get(
        client,
        &repository_path(verified, "pulls"),
        vec![
            ("state", state.to_string()),
            ("head", format!("{}:{head}", verified.target.owner)),
            ("base", base.to_string()),
        ],
    )?;
    if truncated {
        return Err(SkillError::new("forge_pr_reconciliation_truncated"));
    }
    Ok(documents
        .into_iter()
        .filter(|value| value.pointer("/head/ref").and_then(Value::as_str) == Some(head))
        .filter(|value| value.pointer("/base/ref").and_then(Value::as_str) == Some(base))
        .collect())
}

fn paginated_get(
    client: &GithubClient,
    path: &str,
    base_query: Vec<(&str, String)>,
) -> Result<(Vec<Value>, bool, Value), SkillError> {
    let mut output = Vec::new();
    let mut rate_limit = json!({});
    for page in 1..=MAX_API_PAGES {
        let mut query = base_query.clone();
        query.push(("per_page", API_PAGE_SIZE.to_string()));
        query.push(("page", page.to_string()));
        let document = client.request(Method::GET, path, &query, None, false)?;
        rate_limit = document.rate_limit;
        let values = if let Some(values) = document.value.as_array() {
            values.clone()
        } else if let Some(values) = document.value.get("check_runs").and_then(Value::as_array) {
            values.clone()
        } else {
            return Err(SkillError::new("forge_api_response_invalid"));
        };
        let page_len = values.len();
        output.extend(values);
        if page_len < API_PAGE_SIZE {
            return Ok((output, false, rate_limit));
        }
    }
    Ok((output, true, rate_limit))
}

fn parse_response(response: Response, mutation: bool) -> Result<ApiDocument, SkillError> {
    let status = response.status();
    let headers = response.headers().clone();
    if status.is_redirection() {
        return Err(SkillError::new("forge_api_redirect_rejected"));
    }
    let mut bytes = Vec::new();
    response
        .take((MAX_API_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| SkillError::new("forge_api_response_read_failed"))?;
    if bytes.len() > MAX_API_BODY_BYTES {
        return Err(SkillError::new("forge_api_response_too_large"));
    }
    let rate_limit = rate_limit_projection(&headers);
    if !status.is_success() {
        let rate_limited = status == StatusCode::TOO_MANY_REQUESTS
            || status == StatusCode::FORBIDDEN
                && (headers.contains_key(RETRY_AFTER)
                    || headers
                        .get("x-ratelimit-remaining")
                        .and_then(|value| value.to_str().ok())
                        == Some("0"));
        let code = match status {
            StatusCode::UNAUTHORIZED => "forge_api_authentication_failed",
            _ if rate_limited => "forge_api_rate_limited",
            StatusCode::FORBIDDEN => "forge_api_permission_denied",
            StatusCode::NOT_FOUND => "forge_api_not_found",
            StatusCode::UNPROCESSABLE_ENTITY => "forge_api_validation_failed",
            _ if status.is_server_error() => "forge_api_unavailable",
            _ => "forge_api_request_rejected",
        };
        let mut error = SkillError::new(code)
            .retryable(rate_limited || status.is_server_error())
            .with_extra(json!({"status_code": status.as_u16(), "rate_limit": rate_limit}));
        if mutation && status.is_server_error() {
            error = error.phase("dispatch").applied(true);
        }
        return Err(error);
    }
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| SkillError::new("forge_api_response_invalid"))?;
    Ok(ApiDocument { value, rate_limit })
}

fn rate_limit_projection(headers: &HeaderMap) -> Value {
    let text = |name: &'static str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    json!({
        "remaining": text("x-ratelimit-remaining"),
        "reset_unix": text("x-ratelimit-reset"),
        "retry_after_seconds": headers.get(RETRY_AFTER).and_then(|value| value.to_str().ok()),
    })
}

fn repository_path(verified: &VerifiedPushReceipt, suffix: &str) -> String {
    format!(
        "/repos/{}/{}/{}",
        verified.target.owner, verified.target.repository, suffix
    )
}

fn validated_title(value: &str) -> Result<&str, SkillError> {
    if value.chars().count() > MAX_TITLE_CHARS || value.chars().any(is_forbidden_control) {
        return Err(SkillError::new("forge_title_invalid"));
    }
    Ok(value)
}

fn validated_body(value: &str) -> Result<&str, SkillError> {
    if value.len() > MAX_PR_BODY_BYTES || value.chars().any(is_forbidden_control) {
        return Err(SkillError::new("forge_body_invalid"));
    }
    Ok(value)
}

fn is_forbidden_control(value: char) -> bool {
    value.is_control() && !matches!(value, '\n' | '\r' | '\t')
}

fn reject_secret_content(value: &str, token: &str) -> Result<(), SkillError> {
    let lower = value.to_ascii_lowercase();
    let encoded = url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>();
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    if value.contains(token)
        || (!encoded.is_empty() && value.contains(&encoded))
        || value.contains(&basic)
        || lower.contains("github_pat_")
        || lower.contains("ghp_")
        || lower.contains("authorization: bearer")
        || lower.contains("https://") && lower.contains('@')
    {
        return Err(SkillError::new("forge_content_secret_detected"));
    }
    Ok(())
}

fn pr_projection(value: &Value) -> Result<Value, SkillError> {
    Ok(json!({
        "number": value_u64(value, "/number", "forge_api_response_invalid")?,
        "state": value_string(value, "/state", "forge_api_response_invalid")?,
        "title": value_string(value, "/title", "forge_api_response_invalid")?,
        "draft": value.pointer("/draft").and_then(Value::as_bool).unwrap_or(false),
        "mergeable": value.pointer("/mergeable").cloned().unwrap_or(Value::Null),
        "head": value_string(value, "/head/ref", "forge_api_response_invalid")?,
        "head_sha": value_string(value, "/head/sha", "forge_api_response_invalid")?,
        "base": value_string(value, "/base/ref", "forge_api_response_invalid")?,
        "html_url": value_string(value, "/html_url", "forge_api_response_invalid")?,
    }))
}

fn check_run_projection(value: &Value) -> Value {
    json!({
        "name": value.get("name").and_then(Value::as_str),
        "status": value.get("status").and_then(Value::as_str),
        "conclusion": value.get("conclusion").and_then(Value::as_str),
    })
}

fn status_projection(value: &Value) -> Value {
    json!({
        "context": value.get("context").and_then(Value::as_str),
        "state": value.get("state").and_then(Value::as_str),
    })
}

fn summarize_checks(check_runs: &[Value], statuses: &[Value]) -> Value {
    let mut pending = 0_u64;
    let mut success = 0_u64;
    let mut failure = 0_u64;
    for value in check_runs {
        match value.get("conclusion").and_then(Value::as_str) {
            Some("success" | "neutral" | "skipped") => success += 1,
            Some(_) => failure += 1,
            None => pending += 1,
        }
    }
    for value in statuses {
        match value.get("state").and_then(Value::as_str) {
            Some("success") => success += 1,
            Some("pending") | None => pending += 1,
            Some(_) => failure += 1,
        }
    }
    json!({
        "total": check_runs.len() + statuses.len(),
        "success": success,
        "pending": pending,
        "failure": failure,
        "overall": if failure > 0 { "failure" } else if pending > 0 { "pending" } else { "success" },
    })
}

fn validate_github_html_url(
    raw: &str,
    owner: &str,
    repository: &str,
    number: u64,
) -> Result<(), SkillError> {
    let expected = format!("https://github.com/{owner}/{repository}/pull/{number}");
    if raw == expected {
        Ok(())
    } else {
        Err(SkillError::new("forge_pr_url_invalid"))
    }
}

fn value_string<'a>(
    value: &'a Value,
    pointer: &str,
    error: &'static str,
) -> Result<&'a str, SkillError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| SkillError::new(error))
}

fn value_u64(value: &Value, pointer: &str, error: &'static str) -> Result<u64, SkillError> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| SkillError::new(error))
}

fn required_u64(
    args: &Map<String, Value>,
    key: &str,
    error: &'static str,
) -> Result<u64, SkillError> {
    args.get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| SkillError::new(error))
}

fn encode_pr_receipt(receipt: &PullRequestReceipt) -> Result<String, SkillError> {
    let payload = serde_json::to_vec(receipt)
        .map_err(|_| SkillError::new("forge_pr_receipt_serialize_failed"))?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&payload);
    let digest = format!("{:x}", Sha256::digest(&payload));
    Ok(format!("github-pr-v1:{encoded}:{digest}"))
}

fn deterministic_operation_id(value: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!("forge-pr-{}", &digest[..24])
}

fn evidence_digest(value: &Value) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "forge_tests.rs"]
mod tests;
