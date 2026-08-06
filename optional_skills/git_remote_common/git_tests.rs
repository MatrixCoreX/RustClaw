use super::*;
use fs2::FileExt as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::process::{Command as ProcessCommand, Stdio as ProcessStdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

struct SmartHttpFixture {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl SmartHttpFixture {
    fn start(project_root: PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind smart HTTP fixture");
        listener
            .set_nonblocking(true)
            .expect("nonblocking smart HTTP listener");
        let address = listener.local_addr().expect("smart HTTP address");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => serve_git_http_request(stream, &project_root),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            handle: Some(handle),
        }
    }

    fn repository_url(&self) -> String {
        format!("http://{}/runtime.git", self.address)
    }
}

impl Drop for SmartHttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve_git_http_request(mut stream: TcpStream, project_root: &Path) {
    // Apple Git/curl may pause briefly before sending the POST body while
    // several package test binaries run concurrently on an Intel Mac.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let (headers, body) = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("smart HTTP fixture request read failed: {error}");
            return;
        }
    };
    let headers = String::from_utf8_lossy(&headers);
    let mut lines = headers.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or("GET");
    let target = request_parts.next().unwrap_or("/");
    let (path_info, query_string) = target.split_once('?').unwrap_or((target, ""));
    let content_type = lines
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-type")
                    .then(|| value.trim().to_string())
            })
        })
        .unwrap_or_default();
    let mut child = match ProcessCommand::new("git")
        .arg("http-backend")
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .env("GIT_PROJECT_ROOT", project_root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", path_info)
        .env("QUERY_STRING", query_string)
        .env("CONTENT_TYPE", content_type)
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(ProcessStdio::piped())
        .stdout(ProcessStdio::piped())
        .stderr(ProcessStdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("smart HTTP fixture backend spawn failed: {error}");
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&body);
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("smart HTTP fixture backend wait failed: {error}");
            return;
        }
    };
    if !output.status.success() {
        eprintln!(
            "smart HTTP fixture backend failed: status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let delimiter = output
        .stdout
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            output
                .stdout
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        });
    let Some((cgi_header_end, delimiter_len)) = delimiter else {
        eprintln!(
            "smart HTTP fixture backend returned no CGI headers: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    };
    let cgi_headers = String::from_utf8_lossy(&output.stdout[..cgi_header_end]);
    let response_body = &output.stdout[cgi_header_end + delimiter_len..];
    let mut status = "200 OK".to_string();
    let mut forwarded = Vec::new();
    for line in cgi_headers.lines() {
        if let Some(value) = line.strip_prefix("Status:") {
            status = value.trim().to_string();
        } else if line.split_once(':').is_some() {
            forwarded.push(line.trim_end_matches('\r').to_string());
        }
    }
    let mut response = format!("HTTP/1.1 {status}\r\n");
    for header in forwarded {
        response.push_str(&header);
        response.push_str("\r\n");
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    ));
    if let Err(error) = stream.write_all(response.as_bytes()) {
        eprintln!("smart HTTP fixture response header write failed: {error}");
        return;
    }
    if let Err(error) = stream.write_all(response_body) {
        eprintln!("smart HTTP fixture response body write failed: {error}");
    }
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 8192];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "smart HTTP request headers ended early",
            ));
        }
        request.extend_from_slice(&chunk[..count]);
        if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = request[..header_end].to_vec();
    let header_text = String::from_utf8_lossy(&headers);
    let header_value = |wanted: &str| {
        header_text.lines().find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case(wanted)
                    .then(|| value.trim().to_string())
            })
        })
    };
    if header_value("expect").is_some_and(|value| value.eq_ignore_ascii_case("100-continue")) {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let mut encoded_body = request[header_end..].to_vec();
    if header_value("transfer-encoding").is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        loop {
            if let Some(body) = decode_chunked_body(&encoded_body)? {
                return Ok((headers, body));
            }
            read_more_http_body(stream, &mut encoded_body)?;
        }
    }
    let content_length = header_value("content-length")
        .map(|value| {
            value.parse::<usize>().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid smart HTTP content length",
                )
            })
        })
        .transpose()?
        .unwrap_or(0);
    while encoded_body.len() < content_length {
        read_more_http_body(stream, &mut encoded_body)?;
    }
    encoded_body.truncate(content_length);
    Ok((headers, encoded_body))
}

fn read_more_http_body(stream: &mut TcpStream, body: &mut Vec<u8>) -> std::io::Result<()> {
    let mut chunk = [0_u8; 8192];
    let count = stream.read(&mut chunk)?;
    if count == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "smart HTTP request body ended early",
        ));
    }
    body.extend_from_slice(&chunk[..count]);
    Ok(())
}

fn decode_chunked_body(encoded: &[u8]) -> std::io::Result<Option<Vec<u8>>> {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    loop {
        let Some(line_end) = encoded[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|offset| cursor + offset)
        else {
            return Ok(None);
        };
        let size_text = std::str::from_utf8(&encoded[cursor..line_end])
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk size"))?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or(""), 16)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk size"))?;
        cursor = line_end + 2;
        if size == 0 {
            if encoded.len() < cursor + 2 {
                return Ok(None);
            }
            if &encoded[cursor..cursor + 2] == b"\r\n" {
                return Ok(Some(decoded));
            }
            return Ok(encoded[cursor..]
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|_| decoded));
        }
        let chunk_end = cursor.saturating_add(size);
        if encoded.len() < chunk_end + 2 {
            return Ok(None);
        }
        if &encoded[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "chunk delimiter",
            ));
        }
        decoded.extend_from_slice(&encoded[cursor..chunk_end]);
        cursor = chunk_end + 2;
    }
}

#[test]
fn smart_http_fixture_decodes_chunked_request_bodies() {
    assert_eq!(
        decode_chunked_body(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").expect("decode"),
        Some(b"Wikipedia".to_vec())
    );
    assert_eq!(
        decode_chunked_body(b"4\r\nWi").expect("incomplete body"),
        None
    );
}

fn repository_fixture() -> (PathBuf, RepositoryContext) {
    let root = std::env::temp_dir().join(format!(
        "agent-git-security-fixture-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&root).expect("fixture root");
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .status()
        .expect("git init");
    assert!(status.success());
    let context = RepositoryContext {
        workspace_root: root.clone(),
        repository_root: root.clone(),
        repo_selector: ".".to_string(),
    };
    (root, context)
}

fn git_config(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("config")
        .args(args)
        .current_dir(root)
        .status()
        .expect("git config");
    assert!(status.success(), "git config {args:?}");
}

fn process_git(root: &Path, args: &[&str]) -> std::process::Output {
    ProcessCommand::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .output()
        .expect("run Git fixture command")
}

fn profile() -> claw_core::git_remote_config::GitConnectionProfile {
    claw_core::git_remote_config::GitConnectionProfile {
        id: "github-main".to_string(),
        forge_kind: "github".to_string(),
        git_host: "github.com".to_string(),
        api_host: "api.github.com".to_string(),
        allowed_owners: vec!["exampleowner".to_string()],
        allowed_repositories: vec!["runtime".to_string()],
        git_username: "x-access-token".to_string(),
        auth_scheme: "token".to_string(),
        git_credential_ref: "github_git_token".to_string(),
        api_credential_ref: "github_api_token".to_string(),
    }
}

#[test]
fn canonical_remote_is_exact_https_allowlisted_repository() {
    let target = canonicalize_remote_url(
        &profile(),
        "origin",
        "https://github.com/ExampleOwner/Runtime.git",
    )
    .expect("canonical remote");
    assert_eq!(target.owner, "exampleowner");
    assert_eq!(target.repository, "runtime");
    assert_eq!(
        target.canonical_url,
        "https://github.com/exampleowner/runtime.git"
    );
    assert!(target.url_digest.starts_with("sha256:"));
}

#[test]
fn unsafe_remote_forms_are_rejected() {
    for raw in [
        "ssh://git@github.com/ExampleOwner/Runtime.git",
        "git@github.com:ExampleOwner/Runtime.git",
        "https://token@github.com/ExampleOwner/Runtime.git",
        "https://github.com/Other/Runtime.git",
        "https://github.com/ExampleOwner/Other.git",
        "https://github.com/ExampleOwner/Runtime.git?x=1",
    ] {
        assert!(
            canonicalize_remote_url(&profile(), "origin", raw).is_err(),
            "{raw}"
        );
    }
}

#[test]
fn push_receipt_detects_tampering() {
    let receipt = PushReceiptProjection {
        schema_version: 1,
        connection_id: "github-main".to_string(),
        repo_selector: ".".to_string(),
        remote: "origin".to_string(),
        remote_url_digest: "sha256:abc".to_string(),
        owner: "exampleowner".to_string(),
        repository: "runtime".to_string(),
        remote_branch: "main".to_string(),
        local_sha: "0".repeat(40),
    };
    let encoded = encode_push_receipt_ref(&receipt).expect("encode");
    assert_eq!(decode_push_receipt_ref(&encoded).expect("decode"), receipt);
    let tampered = encoded.replacen('A', "B", 1);
    assert!(decode_push_receipt_ref(&tampered).is_err());
}

#[test]
fn redactor_covers_raw_encoded_basic_and_userinfo() {
    let runtime = InvocationRuntime {
        root: std::env::temp_dir().join("agent-git-redactor-test-do-not-create"),
        askpass: None,
        token: Some("synthetic-token+/=".to_string()),
    };
    let basic =
        base64::engine::general_purpose::STANDARD.encode("x-access-token:synthetic-token+/=");
    let encoded = url::form_urlencoded::byte_serialize(b"synthetic-token+/=").collect::<String>();
    let output = runtime.redact(
        &format!("synthetic-token+/= {encoded} {basic} https://secret@github.com/a/b"),
        Some("x-access-token"),
    );
    assert!(!output.contains("synthetic-token"));
    assert!(!output.contains(&basic));
    assert!(!output.contains("secret@"));
}

#[test]
fn fetch_disk_floor_fails_closed() {
    let error = enforce_fetch_disk_floor(63, 64).expect_err("insufficient disk rejected");
    assert_eq!(error.code, "git_disk_space_insufficient");
    assert!(enforce_fetch_disk_floor(64, 64).is_ok());
}

#[test]
fn smart_http_fixture_covers_public_fetch_exact_push_and_non_fast_forward() {
    // Cargo runs this shared test once per skill binary. Apple Git's bundled
    // HTTP stack intermittently resets independent CGI connections when those
    // binaries exercise receive-pack concurrently, so keep the transport
    // integration fixture deterministic across test processes.
    let fixture_lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(std::env::temp_dir().join("git-smart-http-fixture-v1.lock"))
        .expect("open smart HTTP fixture lock");
    fixture_lock
        .lock_exclusive()
        .expect("lock smart HTTP fixture");
    let fixture_root = std::env::temp_dir().join(format!(
        "agent-git-smart-http-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let project_root = fixture_root.join("projects");
    let bare = project_root.join("runtime.git");
    let source = fixture_root.join("source");
    let stale = fixture_root.join("stale");
    std::fs::create_dir_all(&project_root).expect("project root");
    std::fs::create_dir_all(&source).expect("source root");
    assert!(ProcessCommand::new("git")
        .args(["init", "--quiet", "--bare"])
        .arg(&bare)
        .status()
        .expect("init bare repository")
        .success());
    git_config(&bare, &["http.receivepack", "true"]);
    assert!(process_git(&source, &["init", "--quiet"]).status.success());
    git_config(&source, &["user.name", "Synthetic Test"]);
    git_config(&source, &["user.email", "synthetic@example.invalid"]);
    assert!(process_git(&source, &["branch", "-M", "main"])
        .status
        .success());
    std::fs::write(source.join("fixture.txt"), "initial\n").expect("initial fixture");
    assert!(process_git(&source, &["add", "fixture.txt"])
        .status
        .success());
    assert!(
        process_git(&source, &["commit", "--quiet", "-m", "initial"])
            .status
            .success()
    );
    assert!(ProcessCommand::new("git")
        .arg("push")
        .arg(&bare)
        .arg("HEAD:refs/heads/main")
        .current_dir(&source)
        .output()
        .expect("seed bare repository")
        .status
        .success());
    assert!(ProcessCommand::new("git")
        .args(["symbolic-ref", "HEAD", "refs/heads/main"])
        .current_dir(&bare)
        .status()
        .expect("set bare default branch")
        .success());

    let fixture = SmartHttpFixture::start(project_root);
    let repository_url = fixture.repository_url();
    let observed = process_git(
        &source,
        &["ls-remote", "--refs", &repository_url, "refs/heads/main"],
    );
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert!(String::from_utf8_lossy(&observed.stdout).contains("refs/heads/main"));
    assert!(ProcessCommand::new("git")
        .args(["clone", "--quiet"])
        .arg(&repository_url)
        .arg(&stale)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("clone over smart HTTP")
        .success());
    git_config(&stale, &["user.name", "Synthetic Test"]);
    git_config(&stale, &["user.email", "synthetic@example.invalid"]);

    std::fs::write(source.join("fixture.txt"), "approved\n").expect("approved fixture");
    assert!(process_git(&source, &["add", "fixture.txt"])
        .status
        .success());
    assert!(
        process_git(&source, &["commit", "--quiet", "-m", "approved"])
            .status
            .success()
    );
    let approved_sha = String::from_utf8(process_git(&source, &["rev-parse", "HEAD"]).stdout)
        .expect("approved SHA UTF-8")
        .trim()
        .to_string();
    let exact_refspec = exact_push_refspec(&approved_sha, "delivery");
    let pushed = process_git(
        &source,
        &["push", "--porcelain", &repository_url, &exact_refspec],
    );
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    let remote_delivery = String::from_utf8(
        ProcessCommand::new("git")
            .args(["rev-parse", "refs/heads/delivery"])
            .current_dir(&bare)
            .output()
            .expect("read bare delivery ref")
            .stdout,
    )
    .expect("remote SHA UTF-8")
    .trim()
    .to_string();
    assert_eq!(remote_delivery, approved_sha);

    std::fs::write(stale.join("fixture.txt"), "diverged\n").expect("diverged fixture");
    assert!(process_git(&stale, &["add", "fixture.txt"])
        .status
        .success());
    assert!(
        process_git(&stale, &["commit", "--quiet", "-m", "diverged"])
            .status
            .success()
    );
    let stale_sha = String::from_utf8(process_git(&stale, &["rev-parse", "HEAD"]).stdout)
        .expect("stale SHA UTF-8")
        .trim()
        .to_string();
    let rejected = process_git(
        &stale,
        &[
            "push",
            "--porcelain",
            &repository_url,
            &exact_push_refspec(&stale_sha, "delivery"),
        ],
    );
    assert!(!rejected.status.success());
    let remote_after_rejection = String::from_utf8(
        ProcessCommand::new("git")
            .args(["rev-parse", "refs/heads/delivery"])
            .current_dir(&bare)
            .output()
            .expect("read unchanged delivery ref")
            .stdout,
    )
    .expect("unchanged SHA UTF-8")
    .trim()
    .to_string();
    assert_eq!(remote_after_rejection, approved_sha);
    drop(fixture);
    let _ = std::fs::remove_dir_all(fixture_root);
    fs2::FileExt::unlock(&fixture_lock).expect("unlock smart HTTP fixture");
}

#[test]
fn smart_http_auth_and_network_failures_remain_machine_classified() {
    let (root, _) = repository_fixture();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind unauthorized fixture");
    let address = listener.local_addr().expect("unauthorized fixture address");
    let unauthorized = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept unauthorized request");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request);
        stream
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"fixture\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .expect("write unauthorized response");
    });
    let unauthorized_url = format!("http://{address}/runtime.git");
    let auth_output = process_git(
        &root,
        &["ls-remote", "--refs", &unauthorized_url, "refs/heads/main"],
    );
    unauthorized.join().expect("unauthorized fixture thread");
    assert!(!auth_output.status.success());

    let unavailable_listener = TcpListener::bind("127.0.0.1:0").expect("reserve unavailable port");
    let unavailable_address = unavailable_listener
        .local_addr()
        .expect("unavailable address");
    drop(unavailable_listener);
    let unavailable_url = format!("http://{unavailable_address}/runtime.git");
    let network_output = process_git(
        &root,
        &["ls-remote", "--refs", &unavailable_url, "refs/heads/main"],
    );
    assert!(!network_output.status.success());

    let runtime_root = std::env::temp_dir().join(format!(
        "agent-git-failure-runtime-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&runtime_root).expect("failure runtime root");
    let runtime = InvocationRuntime {
        root: runtime_root,
        askpass: None,
        token: None,
    };
    for output in [auth_output, network_output] {
        let classified = git_command_error(
            "git_remote_request_failed",
            &runtime,
            &GitOutput {
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            },
            None,
            "dispatch",
            false,
        );
        assert_eq!(classified.code, "git_remote_request_failed");
        assert_eq!(classified.failure_phase, "dispatch");
        assert!(!classified.side_effect_applied);
        assert!(classified.detail_extra.is_some());
    }
    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn exact_push_inputs_reject_revision_refspec_and_option_injection() {
    let (root, _) = repository_fixture();
    for branch in [
        "-force",
        "+main",
        "main~1",
        "main:other",
        "refs/heads/*",
        "main\nother",
    ] {
        assert_eq!(
            validated_branch(&root, branch)
                .expect_err("unsafe branch must fail")
                .code,
            "git_branch_invalid",
            "{branch:?}"
        );
    }
    for revision in [
        "HEAD".to_string(),
        "a".repeat(39),
        format!("{}~1", "a".repeat(40)),
    ] {
        assert_eq!(
            validated_sha(&revision)
                .expect_err("revision expression must fail")
                .code,
            "git_sha_invalid"
        );
    }
    let sha = "a".repeat(40);
    let refspec = exact_push_refspec(&sha, "delivery/reviewed");
    assert_eq!(refspec, format!("{sha}:refs/heads/delivery/reviewed"));
    assert!(!refspec.starts_with('+'));
    assert!(!refspec.contains('*'));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remote_url_digest_change_fails_before_transport() {
    let target = canonicalize_remote_url(
        &profile(),
        "origin",
        "https://github.com/ExampleOwner/Runtime.git",
    )
    .expect("canonical target");
    let args = serde_json::json!({
        "expected_remote_url_digest": "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    });
    let error = require_url_digest(args.as_object().expect("arguments"), &target)
        .expect_err("changed target digest");
    assert_eq!(error.code, "git_remote_url_precondition_changed");
    assert_eq!(
        error
            .detail_extra
            .as_ref()
            .and_then(|value| value.get("observed")),
        Some(&serde_json::Value::String(target.url_digest))
    );
}

#[test]
fn dirty_worktree_is_evidence_and_does_not_change_fixed_commit() {
    let (root, context) = repository_fixture();
    git_config(&root, &["user.name", "Synthetic Test"]);
    git_config(&root, &["user.email", "synthetic@example.invalid"]);
    std::fs::write(root.join("tracked.txt"), "committed\n").expect("tracked file");
    assert!(std::process::Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(&root)
        .status()
        .expect("git add")
        .success());
    assert!(std::process::Command::new("git")
        .args(["commit", "--quiet", "-m", "fixture"])
        .current_dir(&root)
        .status()
        .expect("git commit")
        .success());
    let approved_sha = plain_git_output(&root, &["rev-parse", "HEAD"])
        .expect("approved commit")
        .trim()
        .to_string();
    std::fs::write(root.join("tracked.txt"), "uncommitted\n").expect("dirty tracked file");
    let runtime_root = std::env::temp_dir().join(format!(
        "agent-git-dirty-runtime-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&runtime_root).expect("runtime root");
    let runtime = InvocationRuntime {
        root: runtime_root,
        askpass: None,
        token: None,
    };
    let evidence = worktree_projection(&context, &runtime).expect("worktree evidence");
    assert_eq!(evidence["worktree_state"], "dirty");
    assert_eq!(evidence["changed_count"], 1);
    let current_sha = plain_git_output(&root, &["rev-parse", "HEAD"])
        .expect("current commit")
        .trim()
        .to_string();
    assert_eq!(current_sha, approved_sha);
    assert_eq!(
        exact_push_refspec(&approved_sha, "delivery"),
        format!("{approved_sha}:refs/heads/delivery")
    );
    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repository_remote_rejects_multiple_urls_and_untrusted_push_target() {
    let (root, context) = repository_fixture();
    git_config(
        &root,
        &[
            "remote.origin.url",
            "https://github.com/ExampleOwner/Runtime.git",
        ],
    );
    git_config(
        &root,
        &[
            "--add",
            "remote.origin.pushurl",
            "https://github.com/ExampleOwner/Runtime.git",
        ],
    );
    git_config(
        &root,
        &[
            "--add",
            "remote.origin.pushurl",
            "https://github.com/ExampleOwner/Runtime.git",
        ],
    );
    assert_eq!(
        resolve_remote_target(&context, &profile(), "origin", RemotePurpose::Push)
            .expect_err("multiple push URLs")
            .code,
        "git_remote_pushurl_count_invalid"
    );

    git_config(&root, &["--unset-all", "remote.origin.pushurl"]);
    git_config(
        &root,
        &[
            "remote.origin.pushurl",
            "https://example.com/attacker/repository.git",
        ],
    );
    assert!(resolve_remote_target(&context, &profile(), "origin", RemotePurpose::Fetch).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repository_remote_rejects_rewrite_proxy_and_credential_helpers() {
    for (key, value) in [
        ("url.https://example.com/.insteadOf", "https://github.com/"),
        ("http.proxy", "http://127.0.0.1:8080"),
        ("credential.helper", "!synthetic-helper"),
        ("core.fsmonitor", "synthetic-monitor"),
    ] {
        let (root, context) = repository_fixture();
        git_config(
            &root,
            &[
                "remote.origin.url",
                "https://github.com/ExampleOwner/Runtime.git",
            ],
        );
        git_config(&root, &[key, value]);
        let error = resolve_remote_target(&context, &profile(), "origin", RemotePurpose::Fetch)
            .expect_err(key);
        assert_eq!(error.code, "git_repository_config_unsafe", "{key}");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(unix)]
#[test]
fn git_transport_process_is_hardened_and_redacted() {
    use std::os::unix::fs::PermissionsExt as _;

    let (root, context) = repository_fixture();
    let capture = root.join("captured-argv.txt");
    let program = root.join("synthetic-git");
    let token = "synthetic-token+/=";
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' \"$GITHUB_GIT_TOKEN\" >&2\nexit 1\n",
            capture.display()
        ),
    )
    .expect("write synthetic Git");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
        .expect("synthetic Git mode");
    let runtime_root = std::env::temp_dir().join(format!(
        "agent-git-hardening-runtime-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&runtime_root).expect("runtime root");
    let runtime = InvocationRuntime {
        root: runtime_root,
        askpass: Some(root.join("synthetic-askpass")),
        token: Some(token.to_string()),
    };
    let args = vec![
        "push".to_string(),
        "--porcelain".to_string(),
        "https://github.com/exampleowner/runtime.git".to_string(),
        format!("{}:refs/heads/delivery", "a".repeat(40)),
    ];
    let output = run_git_with_program(&context, &runtime, &args, Some("x-access-token"), &program)
        .expect("synthetic Git output");
    assert!(!output.status.success());
    assert!(!output.stderr.contains(token));
    assert!(output.stderr.contains("[REDACTED]"));
    let captured = std::fs::read_to_string(&capture).expect("captured argv");
    assert!(captured.contains("core.hooksPath=/dev/null"));
    assert!(captured.contains("credential.helper="));
    assert!(captured.contains("http.followRedirects=false"));
    assert!(captured.contains(&format!("{}:refs/heads/delivery", "a".repeat(40))));
    assert!(!captured.contains(token));
    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn git_transport_timeout_terminates_the_child() {
    use std::os::unix::fs::PermissionsExt as _;

    let (root, context) = repository_fixture();
    let program = root.join("sleeping-git");
    std::fs::write(&program, "#!/bin/sh\nwhile :; do :; done\n")
        .expect("write sleeping Git fixture");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
        .expect("sleeping Git mode");
    let runtime_root = std::env::temp_dir().join(format!(
        "agent-git-timeout-runtime-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&runtime_root).expect("runtime root");
    let runtime = InvocationRuntime {
        root: runtime_root,
        askpass: None,
        token: None,
    };
    let started = std::time::Instant::now();
    let error = run_git_with_program_and_timeout(
        &context,
        &runtime,
        &["ls-remote".to_string()],
        None,
        &program,
        Duration::from_millis(50),
    )
    .expect_err("timeout must fail closed");
    assert_eq!(error.code, "git_command_timeout");
    assert!(error.retryable);
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(runtime);
    let _ = std::fs::remove_dir_all(root);
}
