use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

fn mock_api(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock API");
    let address = listener.local_addr().expect("mock address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept mock request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout");
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap_or_default();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            captured
                .lock()
                .expect("capture request")
                .push(String::from_utf8_lossy(&bytes).to_string());
            stream
                .write_all(response.as_bytes())
                .expect("mock response");
        }
    });
    (format!("http://{address}"), requests, handle)
}

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut value = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, header_value) in headers {
        value.push_str(&format!("{name}: {header_value}\r\n"));
    }
    value.push_str("\r\n");
    value.push_str(body);
    value
}

fn verified_fixture() -> (std::path::PathBuf, VerifiedPushReceipt) {
    let root = std::env::temp_dir().join(format!(
        "agent-forge-fixture-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&root).expect("fixture root");
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .status()
        .expect("git init");
    assert!(status.success());
    let connection_id = "github-main".to_string();
    let owner = "exampleowner".to_string();
    let repository = "runtime".to_string();
    let head_sha = "a".repeat(40);
    let remote_url_digest = "sha256:synthetic".to_string();
    let context = super::super::git::RepositoryContext {
        workspace_root: root.clone(),
        repository_root: root.clone(),
        repo_selector: ".".to_string(),
    };
    let profile = claw_core::git_remote_config::GitConnectionProfile {
        id: connection_id.clone(),
        forge_kind: "github".to_string(),
        git_host: "github.com".to_string(),
        api_host: "api.github.com".to_string(),
        allowed_owners: vec![owner.clone()],
        allowed_repositories: vec![repository.clone()],
        git_username: "x-access-token".to_string(),
        auth_scheme: "token".to_string(),
        git_credential_ref: "github_git_token".to_string(),
        api_credential_ref: "github_api_token".to_string(),
    };
    let target = super::super::git::RemoteTarget {
        canonical_url: "https://github.com/exampleowner/runtime.git".to_string(),
        url_digest: remote_url_digest.clone(),
        owner: owner.clone(),
        repository: repository.clone(),
        remote: "origin".to_string(),
    };
    let receipt = super::super::git::PushReceiptProjection {
        schema_version: 1,
        connection_id,
        repo_selector: ".".to_string(),
        remote: "origin".to_string(),
        remote_url_digest,
        owner,
        repository,
        remote_branch: "delivery".to_string(),
        local_sha: head_sha,
    };
    (
        root,
        VerifiedPushReceipt {
            context,
            profile,
            target,
            receipt,
        },
    )
}

fn pull_request_json(number: u64) -> Value {
    json!({
        "number": number,
        "state": "open",
        "title": "Delivery closure",
        "draft": false,
        "mergeable": null,
        "head": {"ref": "delivery", "sha": "a".repeat(40)},
        "base": {"ref": "main"},
        "html_url": format!("https://github.com/exampleowner/runtime/pull/{number}"),
    })
}

#[test]
fn pr_content_secret_scan_is_fail_closed() {
    let token = "synthetic-github-token";
    for value in [
        "contains synthetic-github-token",
        "contains github_pat_example",
        "contains ghp_example",
        "Authorization: Bearer example",
        "https://user@example.test/path",
    ] {
        assert!(reject_secret_content(value, token).is_err(), "{value}");
    }
    assert!(reject_secret_content("ordinary release notes", token).is_ok());
}

#[test]
fn check_summary_combines_both_github_sources() {
    let checks = vec![
        json!({"conclusion": "success"}),
        json!({"conclusion": null}),
    ];
    let statuses = vec![json!({"state": "failure"})];
    let summary = summarize_checks(&checks, &statuses);
    assert_eq!(summary["total"], 3);
    assert_eq!(summary["success"], 1);
    assert_eq!(summary["pending"], 1);
    assert_eq!(summary["failure"], 1);
    assert_eq!(summary["overall"], "failure");
}

#[test]
fn github_pr_url_must_match_verified_repository_and_number() {
    assert!(validate_github_html_url(
        "https://github.com/exampleowner/runtime/pull/7",
        "exampleowner",
        "runtime",
        7,
    )
    .is_ok());
    assert!(validate_github_html_url(
        "https://example.test/exampleowner/runtime/pull/7",
        "exampleowner",
        "runtime",
        7,
    )
    .is_err());
}

#[test]
fn pr_text_bounds_keep_newlines_but_reject_controls() {
    assert!(validated_title("release closure").is_ok());
    assert!(validated_body("line one\nline two").is_ok());
    assert!(validated_title("bad\u{0000}").is_err());
    assert!(validated_body("bad\u{0007}").is_err());
}

#[test]
fn github_api_client_sets_fixed_headers_and_refuses_redirects() {
    let (base, requests, handle) = mock_api(vec![response(
        "302 Found",
        &[("Location", "https://example.invalid/leak")],
        "{}",
    )]);
    let client = GithubClient::new_for_test(base, "synthetic-token").expect("client");
    let error = client
        .request(Method::GET, "/repos/o/r/pulls", &[], None, false)
        .expect_err("redirect rejected");
    assert_eq!(error.code, "forge_api_redirect_rejected");
    handle.join().expect("mock join");
    let request = requests.lock().expect("requests").join("\n");
    assert!(request.starts_with("GET /repos/o/r/pulls HTTP/1.1"));
    assert!(request.contains("x-github-api-version: 2022-11-28"));
    assert!(request.contains("authorization: Bearer synthetic-token"));
    assert!(!request.contains("example.invalid"));
}

#[test]
fn github_api_statuses_use_machine_contract_without_message_parsing() {
    let cases = [
        (
            "401 Unauthorized",
            Vec::new(),
            "forge_api_authentication_failed",
            false,
        ),
        (
            "403 Forbidden",
            Vec::new(),
            "forge_api_permission_denied",
            false,
        ),
        ("404 Not Found", Vec::new(), "forge_api_not_found", false),
        (
            "422 Unprocessable Entity",
            Vec::new(),
            "forge_api_validation_failed",
            false,
        ),
        (
            "403 Forbidden",
            vec![("Retry-After", "7")],
            "forge_api_rate_limited",
            true,
        ),
        (
            "500 Internal Server Error",
            Vec::new(),
            "forge_api_unavailable",
            true,
        ),
    ];
    for (status, headers, expected, retryable) in cases {
        let (base, _, handle) = mock_api(vec![response(
            status,
            &headers,
            r#"{"message":"arbitrary prose"}"#,
        )]);
        let client = GithubClient::new_for_test(base, "synthetic-token").expect("client");
        let error = client
            .request(Method::GET, "/repos/o/r/pulls", &[], None, false)
            .expect_err(status);
        assert_eq!(error.code, expected, "{status}");
        assert_eq!(error.retryable, retryable, "{status}");
        handle.join().expect("mock join");
    }
}

#[test]
fn github_api_pagination_collects_all_pages_and_rate_projection() {
    let first =
        Value::Array((1..=100).map(|number| json!({"number": number})).collect()).to_string();
    let second = json!([{"number": 101}]).to_string();
    let (base, requests, handle) = mock_api(vec![
        response("200 OK", &[("X-RateLimit-Remaining", "9")], &first),
        response("200 OK", &[("X-RateLimit-Remaining", "8")], &second),
    ]);
    let client = GithubClient::new_for_test(base, "synthetic-token").expect("client");
    let (values, truncated, rate) =
        paginated_get(&client, "/repos/o/r/pulls", Vec::new()).expect("pagination");
    handle.join().expect("mock join");
    assert_eq!(values.len(), 101);
    assert!(!truncated);
    assert_eq!(rate["remaining"], "8");
    let requests = requests.lock().expect("requests");
    assert!(requests[0].contains("page=1"));
    assert!(requests[1].contains("page=2"));
}

#[test]
fn create_pr_and_validation_race_return_one_digest_bound_receipt() {
    let (root, verified) = verified_fixture();
    let created = pull_request_json(7).to_string();
    let (base, requests, handle) = mock_api(vec![response("201 Created", &[], &created)]);
    let client = GithubClient::new_for_test(base, "synthetic-token").expect("client");
    let args = json!({
        "expected_head_sha": "a".repeat(40),
        "head": "delivery",
        "base": "main",
        "title": "Delivery closure",
        "body": "Verified change",
        "draft": false,
    });
    let result = create_pr(args.as_object().expect("args"), &verified, &client).expect("create PR");
    handle.join().expect("mock join");
    assert_eq!(result["status"], "applied");
    assert_eq!(result["pull_request"]["mergeable"], Value::Null);
    assert!(result["result_ref"]
        .as_str()
        .is_some_and(|value| value.starts_with("github-pr-v1:")));
    assert!(requests.lock().expect("requests")[0].starts_with("POST "));

    let matching = json!([pull_request_json(8)]).to_string();
    let (base, _, handle) = mock_api(vec![
        response(
            "422 Unprocessable Entity",
            &[],
            r#"{"message":"changed prose"}"#,
        ),
        response("200 OK", &[], &matching),
    ]);
    let client = GithubClient::new_for_test(base, "synthetic-token").expect("client");
    let result = create_pr(args.as_object().expect("args"), &verified, &client)
        .expect("requery duplicate race");
    handle.join().expect("mock join");
    assert_eq!(result["status"], "already_applied");
    assert_eq!(result["pull_request"]["number"], 8);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reconcile_pr_is_observe_only_and_returns_authoritative_receipt() {
    let (root, verified) = verified_fixture();
    let matching = json!([pull_request_json(9)]).to_string();
    let (base, _, handle) = mock_api(vec![response("200 OK", &[], &matching)]);
    let client = GithubClient::new_for_test(base, "synthetic-token").expect("client");
    let args = json!({
        "expected_head_sha": "a".repeat(40),
        "head": "delivery",
        "base": "main",
    });
    let result = reconcile_create_pr(args.as_object().expect("args"), &verified, &client)
        .expect("reconcile PR");
    handle.join().expect("mock join");
    assert_eq!(result["effect"], "observe");
    assert_eq!(result["disposition"], "applied");
    assert_eq!(result["action_ref"], "forge.create_pr");
    assert!(result["result_ref"].as_str().is_some());
    let _ = std::fs::remove_dir_all(root);
}
