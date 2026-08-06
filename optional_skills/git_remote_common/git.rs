use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wait_timeout::ChildExt as _;

use super::{optional_bool, optional_string, required_string, SkillError};

const MAX_GIT_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_REPO_PATH_BYTES: usize = 4096;
const MIN_FETCH_FREE_BYTES: u64 = 64 * 1024 * 1024;
const GITHUB_GIT_TOKEN_ENV: &str = "GITHUB_GIT_TOKEN";
const PUSH_RECEIPT_PREFIX: &str = "git-push-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemotePurpose {
    Fetch,
    Push,
}

#[derive(Debug, Clone)]
pub struct RepositoryContext {
    pub workspace_root: PathBuf,
    pub repository_root: PathBuf,
    pub repo_selector: String,
}

#[derive(Debug, Clone)]
pub struct RemoteTarget {
    pub canonical_url: String,
    pub url_digest: String,
    pub owner: String,
    pub repository: String,
    pub remote: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedPushReceipt {
    pub context: RepositoryContext,
    pub profile: claw_core::git_remote_config::GitConnectionProfile,
    pub target: RemoteTarget,
    pub receipt: PushReceiptProjection,
}

#[derive(Debug)]
struct GitOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct InvocationRuntime {
    root: PathBuf,
    askpass: Option<PathBuf>,
    token: Option<String>,
}

impl InvocationRuntime {
    fn new(authenticated: bool) -> Result<Self, SkillError> {
        let root =
            std::env::temp_dir().join(format!("agent-git-remote-{}", Uuid::new_v4().simple()));
        fs::create_dir(&root).map_err(|_| SkillError::new("git_runtime_temp_unavailable"))?;
        apply_private_directory_permissions(&root)?;
        let token = if authenticated {
            Some(
                claw_core::secrets::env_non_empty_resolved_or_none(GITHUB_GIT_TOKEN_ENV)
                    .ok_or_else(|| SkillError::new("git_credentials_missing"))?,
            )
        } else {
            None
        };
        let askpass = if authenticated {
            Some(create_askpass(&root)?)
        } else {
            None
        };
        Ok(Self {
            root,
            askpass,
            token,
        })
    }

    fn redact(&self, text: &str, username: Option<&str>) -> String {
        let mut redacted = text.to_string();
        if let Some(token) = self.token.as_deref() {
            redacted = redacted.replace(token, "[REDACTED]");
            let encoded =
                url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>();
            if encoded != token {
                redacted = redacted.replace(&encoded, "[REDACTED]");
            }
            if let Some(username) = username {
                let basic =
                    base64::engine::general_purpose::STANDARD.encode(format!("{username}:{token}"));
                redacted = redacted.replace(&basic, "[REDACTED]");
            }
        }
        let userinfo = Regex::new(r"(?i)(https://)[^\s/@]+@").expect("userinfo regex");
        userinfo.replace_all(&redacted, "$1[REDACTED]@").to_string()
    }
}

impl Drop for InvocationRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushReceiptProjection {
    pub schema_version: u32,
    pub connection_id: String,
    pub repo_selector: String,
    pub remote: String,
    pub remote_url_digest: String,
    pub owner: String,
    pub repository: String,
    pub remote_branch: String,
    pub local_sha: String,
}

pub fn execute_remote_read(args: &Map<String, Value>) -> Result<Value, SkillError> {
    let action = required_string(args, "action", "git_action_missing")?;
    let authenticated = matches!(action, "ls_remote_authenticated" | "fetch_authenticated");
    if !matches!(
        action,
        "ls_remote_public" | "ls_remote_authenticated" | "fetch_public" | "fetch_authenticated"
    ) {
        return Err(SkillError::new("unsupported_action"));
    }
    let context = repository_context(args)?;
    let connection_id = required_string(args, "connection_id", "git_connection_id_missing")?;
    let profile = load_profile(&context, connection_id)?;
    let remote = required_string(args, "remote", "git_remote_missing")?;
    let remote_branch = validated_branch(
        &context.repository_root,
        required_string(args, "remote_branch", "git_remote_branch_missing")?,
    )?;
    let purpose = RemotePurpose::Fetch;
    let target = resolve_remote_target(&context, &profile, remote, purpose)?;
    require_url_digest(args, &target)?;
    let runtime = InvocationRuntime::new(authenticated)?;
    let username = authenticated.then_some(profile.git_username.as_str());

    match action {
        "ls_remote_public" | "ls_remote_authenticated" => {
            let remote_sha =
                ls_remote_branch(&context, &runtime, &target, &remote_branch, username)?;
            Ok(json!({
                "action": action,
                "effect": "observe",
                "connection_id": profile.id,
                "remote": target.remote,
                "owner": target.owner,
                "repository": target.repository,
                "remote_branch": remote_branch,
                "remote_sha": remote_sha,
                "remote_exists": remote_sha.is_some(),
                "remote_url_digest": target.url_digest,
                "authenticated": authenticated,
                "truncated": false,
                "observed_at": epoch_seconds(),
            }))
        }
        "fetch_public" | "fetch_authenticated" => {
            let available_disk_bytes = ensure_fetch_disk_available(&context.repository_root)?;
            let tracking_ref = format!("refs/remotes/{remote}/{remote_branch}");
            let before_sha = optional_ref_sha(&context.repository_root, &tracking_ref)?;
            let before_objects = object_store_kib(&context, &runtime)?;
            let output = run_git(
                &context,
                &runtime,
                &[
                    "fetch".to_string(),
                    "--no-tags".to_string(),
                    "--no-recurse-submodules".to_string(),
                    target.canonical_url.clone(),
                    format!("refs/heads/{remote_branch}:{tracking_ref}"),
                ],
                username,
            )?;
            if !output.status.success() {
                return Err(git_command_error(
                    "git_fetch_failed",
                    &runtime,
                    &output,
                    username,
                    "dispatch",
                    false,
                ));
            }
            let after_sha = optional_ref_sha(&context.repository_root, &tracking_ref)?
                .ok_or_else(|| SkillError::new("git_fetch_postcondition_failed"))?;
            let after_objects = object_store_kib(&context, &runtime)?;
            let worktree = worktree_projection(&context, &runtime)?;
            Ok(json!({
                "action": action,
                "effect": "mutate",
                "connection_id": profile.id,
                "remote": target.remote,
                "owner": target.owner,
                "repository": target.repository,
                "remote_branch": remote_branch,
                "tracking_ref": tracking_ref,
                "before_sha": before_sha,
                "after_sha": after_sha,
                "remote_url_digest": target.url_digest,
                "authenticated": authenticated,
                "object_store_kib_before": before_objects,
                "object_store_kib_after": after_objects,
                "object_store_kib_delta": after_objects.saturating_sub(before_objects),
                "available_disk_bytes_before": available_disk_bytes,
                "worktree_state": worktree["worktree_state"],
                "worktree_status_sha256": worktree["worktree_status_sha256"],
                "changed_count": worktree["changed_count"],
                "remote_mutation": false,
                "reversible": true,
                "observed_at": epoch_seconds(),
            }))
        }
        _ => unreachable!(),
    }
}

pub fn execute_remote_publish(args: &Map<String, Value>) -> Result<Value, SkillError> {
    let action = required_string(args, "action", "git_action_missing")?;
    if !matches!(action, "push" | "reconcile_push") {
        return Err(SkillError::new("unsupported_action"));
    }
    let context = repository_context(args)?;
    let connection_id = required_string(args, "connection_id", "git_connection_id_missing")?;
    let profile = load_profile(&context, connection_id)?;
    let remote = required_string(args, "remote", "git_remote_missing")?;
    let remote_branch = validated_branch(
        &context.repository_root,
        required_string(args, "remote_branch", "git_remote_branch_missing")?,
    )?;
    let target = resolve_remote_target(&context, &profile, remote, RemotePurpose::Push)?;
    require_url_digest(args, &target)?;
    let expected_local_sha = validated_sha(required_string(
        args,
        "expected_local_sha",
        "git_expected_local_sha_missing",
    )?)?;
    let runtime = InvocationRuntime::new(true)?;
    let username = Some(profile.git_username.as_str());
    if !args.contains_key("expected_remote_sha") {
        return Err(SkillError::new("git_expected_remote_sha_missing"));
    }

    if action == "reconcile_push" {
        let observed = ls_remote_branch(&context, &runtime, &target, &remote_branch, username)?;
        let expected_before = optional_sha_arg(args, "expected_remote_sha")?;
        let disposition = if observed.as_deref() == Some(expected_local_sha.as_str()) {
            "applied"
        } else if observed == expected_before {
            "not_applied"
        } else {
            "still_unknown"
        };
        let result_ref = if disposition == "applied" {
            Some(encode_push_receipt_ref(&PushReceiptProjection {
                schema_version: 1,
                connection_id: profile.id.clone(),
                repo_selector: context.repo_selector.clone(),
                remote: remote.to_string(),
                remote_url_digest: target.url_digest.clone(),
                owner: target.owner.clone(),
                repository: target.repository.clone(),
                remote_branch: remote_branch.clone(),
                local_sha: expected_local_sha.clone(),
            })?)
        } else {
            None
        };
        let operation_id = result_ref.as_deref().map(deterministic_operation_id);
        let evidence = json!({
            "expected_remote_sha": expected_before,
            "observed_remote_sha": observed,
            "remote_url_digest": target.url_digest,
        });
        return Ok(json!({
            "action": action,
            "effect": "observe",
            "disposition": disposition,
            "connection_id": profile.id,
            "remote": target.remote,
            "remote_branch": remote_branch,
            "expected_local_sha": expected_local_sha,
            "expected_remote_sha": expected_before,
            "observed_remote_sha": observed,
            "remote_url_digest": target.url_digest,
            "operation_id": operation_id,
            "action_ref": "git.push",
            "target_ref": format!("{}/{}#refs/heads/{remote_branch}", target.owner, target.repository),
            "before_version": expected_before,
            "after_version": observed,
            "result_ref": result_ref,
            "reversible": false,
            "evidence_digest": evidence_digest(&evidence),
            "observed_at": epoch_seconds(),
        }));
    }

    if !args.contains_key("set_upstream") {
        return Err(SkillError::new("git_set_upstream_missing"));
    }

    let local_branch = validated_branch(
        &context.repository_root,
        required_string(args, "local_branch", "git_local_branch_missing")?,
    )?;
    let local_ref = format!("refs/heads/{local_branch}");
    let observed_local_sha = required_ref_sha(&context.repository_root, &local_ref)?;
    if observed_local_sha != expected_local_sha {
        return Err(
            SkillError::new("git_push_precondition_changed").with_extra(json!({
                "precondition": "local_sha",
                "expected": expected_local_sha,
                "observed": observed_local_sha,
            })),
        );
    }
    let expected_remote_sha = optional_sha_arg(args, "expected_remote_sha")?;
    let remote_before = ls_remote_branch(&context, &runtime, &target, &remote_branch, username)?;
    if remote_before != expected_remote_sha {
        return Err(
            SkillError::new("git_push_precondition_changed").with_extra(json!({
                "precondition": "remote_sha",
                "expected": expected_remote_sha,
                "observed": remote_before,
            })),
        );
    }
    if let Some(remote_sha) = remote_before.as_deref() {
        ensure_fast_forward(&context, &runtime, remote_sha, &expected_local_sha)?;
    }
    let worktree = worktree_projection(&context, &runtime)?;
    let push = run_git(
        &context,
        &runtime,
        &[
            "push".to_string(),
            "--porcelain".to_string(),
            target.canonical_url.clone(),
            exact_push_refspec(&expected_local_sha, &remote_branch),
        ],
        username,
    )?;
    if !push.status.success() {
        return Err(git_command_error(
            "git_remote_rejected",
            &runtime,
            &push,
            username,
            "dispatch",
            true,
        ));
    }
    let remote_after = ls_remote_branch(&context, &runtime, &target, &remote_branch, username)?;
    if remote_after.as_deref() != Some(expected_local_sha.as_str()) {
        return Err(SkillError::new("git_push_postcondition_uncertain")
            .phase("postcondition")
            .applied(true)
            .retryable(true)
            .with_extra(json!({"observed_remote_sha": remote_after})));
    }

    let set_upstream = optional_bool(args, "set_upstream", false)?;
    let mut upstream_set = false;
    let mut result_status = "applied";
    if set_upstream {
        if update_tracking_and_upstream(
            &context,
            &runtime,
            remote,
            &remote_branch,
            &local_branch,
            &expected_local_sha,
        )
        .is_ok()
        {
            upstream_set = true;
        } else {
            result_status = "applied_with_local_followup_required";
        }
    }
    let receipt = PushReceiptProjection {
        schema_version: 1,
        connection_id: profile.id.clone(),
        repo_selector: context.repo_selector.clone(),
        remote: remote.to_string(),
        remote_url_digest: target.url_digest.clone(),
        owner: target.owner.clone(),
        repository: target.repository.clone(),
        remote_branch: remote_branch.clone(),
        local_sha: expected_local_sha.clone(),
    };
    let result_ref = encode_push_receipt_ref(&receipt)?;
    let operation_id = deterministic_operation_id(&result_ref);
    let evidence = json!({
        "remote_sha_before": remote_before,
        "remote_sha_after": remote_after,
        "remote_url_digest": target.url_digest,
    });
    let evidence_digest = evidence_digest(&evidence);
    Ok(json!({
        "action": action,
        "effect": "external",
        "status": result_status,
        "operation_id": operation_id,
        "action_ref": "git.push",
        "target_ref": format!("{}/{}#refs/heads/{remote_branch}", target.owner, target.repository),
        "before_version": remote_before,
        "after_version": remote_after,
        "connection_id": profile.id,
        "remote": target.remote,
        "owner": target.owner,
        "repository": target.repository,
        "local_branch": local_branch,
        "remote_branch": remote_branch,
        "expected_local_sha": expected_local_sha,
        "remote_sha_before": evidence["remote_sha_before"],
        "remote_sha_after": evidence["remote_sha_after"],
        "remote_url_digest": evidence["remote_url_digest"],
        "upstream_set": upstream_set,
        "forced": false,
        "worktree_state": worktree["worktree_state"],
        "worktree_status_sha256": worktree["worktree_status_sha256"],
        "changed_count": worktree["changed_count"],
        "result_ref": result_ref,
        "reversible": false,
        "evidence_digest": evidence_digest,
        "observed_at": epoch_seconds(),
    }))
}

pub fn repository_context(args: &Map<String, Value>) -> Result<RepositoryContext, SkillError> {
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|_| SkillError::new("workspace_canonicalize_failed"))?;
    let repo_selector = optional_string(args, "repo", "git_repository_path_invalid")?
        .unwrap_or(".")
        .to_string();
    if repo_selector.len() > MAX_REPO_PATH_BYTES {
        return Err(SkillError::new("git_repository_path_invalid"));
    }
    let selector_path = Path::new(&repo_selector);
    if selector_path.is_absolute()
        || selector_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(SkillError::new("git_repository_outside_workspace"));
    }
    let candidate = workspace_root.join(selector_path);
    let candidate = candidate
        .canonicalize()
        .map_err(|_| SkillError::new("git_repository_path_invalid"))?;
    if !candidate.starts_with(&workspace_root) {
        return Err(SkillError::new("git_repository_outside_workspace"));
    }
    let root = plain_git_output(&candidate, &["rev-parse", "--show-toplevel"])?;
    let repository_root = PathBuf::from(root.trim())
        .canonicalize()
        .map_err(|_| SkillError::new("not_git_repository"))?;
    if !repository_root.starts_with(&workspace_root) {
        return Err(SkillError::new("git_repository_outside_workspace"));
    }
    Ok(RepositoryContext {
        workspace_root,
        repository_root,
        repo_selector,
    })
}

pub fn load_profile(
    context: &RepositoryContext,
    connection_id: &str,
) -> Result<claw_core::git_remote_config::GitConnectionProfile, SkillError> {
    let path = claw_core::git_remote_config::git_connection_store_path(&context.workspace_root);
    claw_core::git_remote_config::find_git_connection(&path, connection_id)
        .map_err(|_| SkillError::new("git_connection_not_found"))
}

pub fn verify_push_receipt(
    args: &Map<String, Value>,
    receipt_ref: &str,
) -> Result<VerifiedPushReceipt, SkillError> {
    let receipt = decode_push_receipt_ref(receipt_ref)?;
    if receipt.schema_version != 1 {
        return Err(SkillError::new("git_push_receipt_schema_unsupported"));
    }
    if let Some(connection_id) =
        optional_string(args, "connection_id", "git_connection_id_invalid")?
    {
        if connection_id != receipt.connection_id {
            return Err(SkillError::new("git_push_receipt_connection_mismatch"));
        }
    }
    if let Some(expected_head_sha) = optional_string(args, "expected_head_sha", "git_sha_invalid")?
    {
        if validated_sha(expected_head_sha)? != receipt.local_sha {
            return Err(SkillError::new("git_push_receipt_head_mismatch"));
        }
    }
    let mut context_args = Map::new();
    context_args.insert(
        "repo".to_string(),
        Value::String(receipt.repo_selector.clone()),
    );
    let context = repository_context(&context_args)?;
    let profile = load_profile(&context, &receipt.connection_id)?;
    let remote_branch = validated_branch(&context.repository_root, &receipt.remote_branch)?;
    let target = resolve_remote_target(&context, &profile, &receipt.remote, RemotePurpose::Push)?;
    if target.url_digest != receipt.remote_url_digest
        || target.owner != receipt.owner
        || target.repository != receipt.repository
        || remote_branch != receipt.remote_branch
    {
        return Err(SkillError::new("git_push_receipt_target_changed"));
    }
    let runtime = InvocationRuntime::new(true)?;
    let observed = ls_remote_branch(
        &context,
        &runtime,
        &target,
        &remote_branch,
        Some(profile.git_username.as_str()),
    )?;
    if observed.as_deref() != Some(receipt.local_sha.as_str()) {
        return Err(
            SkillError::new("git_push_receipt_remote_unverified").with_extra(json!({
                "expected_head_sha": receipt.local_sha,
                "observed_head_sha": observed,
            })),
        );
    }
    Ok(VerifiedPushReceipt {
        context,
        profile,
        target,
        receipt,
    })
}

fn resolve_remote_target(
    context: &RepositoryContext,
    profile: &claw_core::git_remote_config::GitConnectionProfile,
    remote: &str,
    purpose: RemotePurpose,
) -> Result<RemoteTarget, SkillError> {
    validate_machine_token(remote, "git_remote_invalid")?;
    reject_dangerous_local_git_config(context)?;
    let fetch_urls =
        local_config_values(&context.repository_root, &format!("remote.{remote}.url"))?;
    if fetch_urls.len() != 1 {
        return Err(SkillError::new("git_remote_url_count_invalid"));
    }
    let push_urls = local_config_values(
        &context.repository_root,
        &format!("remote.{remote}.pushurl"),
    )?;
    if push_urls.len() > 1 {
        return Err(SkillError::new("git_remote_pushurl_count_invalid"));
    }
    let fetch_target = canonicalize_remote_url(profile, remote, &fetch_urls[0])?;
    let push_target = push_urls
        .first()
        .map(|value| canonicalize_remote_url(profile, remote, value))
        .transpose()?;
    Ok(match purpose {
        RemotePurpose::Fetch => fetch_target,
        RemotePurpose::Push => push_target.unwrap_or(fetch_target),
    })
}

fn canonicalize_remote_url(
    profile: &claw_core::git_remote_config::GitConnectionProfile,
    remote: &str,
    raw: &str,
) -> Result<RemoteTarget, SkillError> {
    let canonical = claw_core::git_remote_config::canonical_github_remote_url(raw).map_err(
        |error| match error.to_string().as_str() {
            "git_remote_url_invalid" => SkillError::new("git_remote_url_invalid"),
            "git_remote_host_not_allowed" => SkillError::new("git_remote_host_not_allowed"),
            "git_remote_path_invalid" => SkillError::new("git_remote_path_invalid"),
            _ => SkillError::new("git_remote_url_not_allowed"),
        },
    )?;
    if canonical.host != profile.git_host {
        return Err(SkillError::new("git_remote_host_not_allowed"));
    }
    let owner = canonical.owner;
    let repository = canonical.repository;
    if !profile.allowed_owners.iter().any(|value| value == &owner)
        || !profile
            .allowed_repositories
            .iter()
            .any(|value| value == &repository)
    {
        return Err(SkillError::new("git_remote_repository_not_allowed"));
    }
    Ok(RemoteTarget {
        canonical_url: canonical.canonical_url,
        url_digest: canonical.url_digest,
        owner,
        repository,
        remote: remote.to_string(),
    })
}

fn reject_dangerous_local_git_config(context: &RepositoryContext) -> Result<(), SkillError> {
    let output = Command::new("git")
        .current_dir(&context.repository_root)
        .args(["config", "--local", "--name-only", "--get-regexp", ".*"])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| SkillError::new("git_spawn_failed"))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(SkillError::new("git_config_read_failed"));
    }
    let dangerous = Regex::new(
        r"(?i)^(url\..*\.(insteadof|pushinsteadof)|http\..*|remote\..*\.proxy|credential\..*|include(if)?\..*|core\.(sshcommand|fsmonitor)|diff\..*\.(command|textconv))$",
    )
    .expect("dangerous git config regex");
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| dangerous.is_match(line.trim()))
    {
        return Err(SkillError::new("git_repository_config_unsafe"));
    }
    Ok(())
}

fn local_config_values(repository: &Path, key: &str) -> Result<Vec<String>, SkillError> {
    let output = Command::new("git")
        .current_dir(repository)
        .args(["config", "--local", "--get-all", key])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| SkillError::new("git_spawn_failed"))?;
    if !output.status.success() {
        return if output.status.code() == Some(1) {
            Ok(Vec::new())
        } else {
            Err(SkillError::new("git_config_read_failed"))
        };
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn ls_remote_branch(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
    target: &RemoteTarget,
    branch: &str,
    username: Option<&str>,
) -> Result<Option<String>, SkillError> {
    let output = run_git(
        context,
        runtime,
        &[
            "ls-remote".to_string(),
            "--refs".to_string(),
            target.canonical_url.clone(),
            format!("refs/heads/{branch}"),
        ],
        username,
    )?;
    if !output.status.success() {
        return Err(git_command_error(
            "git_remote_request_failed",
            runtime,
            &output,
            username,
            "dispatch",
            false,
        ));
    }
    let mut lines = output.stdout.lines().filter(|line| !line.trim().is_empty());
    let first = lines.next();
    if lines.next().is_some() {
        return Err(SkillError::new("git_remote_response_ambiguous"));
    }
    first
        .map(|line| {
            let mut fields = line.split_whitespace();
            let sha = fields
                .next()
                .ok_or_else(|| SkillError::new("git_remote_response_invalid"))?;
            let reference = fields
                .next()
                .ok_or_else(|| SkillError::new("git_remote_response_invalid"))?;
            if fields.next().is_some() || reference != format!("refs/heads/{branch}") {
                return Err(SkillError::new("git_remote_response_invalid"));
            }
            validated_sha(sha)
        })
        .transpose()
}

fn run_git(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
    args: &[String],
    username: Option<&str>,
) -> Result<GitOutput, SkillError> {
    run_git_with_program(context, runtime, args, username, Path::new("git"))
}

fn run_git_with_program(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
    args: &[String],
    username: Option<&str>,
    program: &Path,
) -> Result<GitOutput, SkillError> {
    run_git_with_program_and_timeout(context, runtime, args, username, program, command_timeout())
}

fn run_git_with_program_and_timeout(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
    args: &[String],
    username: Option<&str>,
    program: &Path,
    timeout: Duration,
) -> Result<GitOutput, SkillError> {
    let stdout_path = runtime
        .root
        .join(format!("stdout-{}.log", Uuid::new_v4().simple()));
    let stderr_path = runtime
        .root
        .join(format!("stderr-{}.log", Uuid::new_v4().simple()));
    let stdout_file = fs::File::create(&stdout_path)
        .map_err(|_| SkillError::new("git_runtime_output_unavailable"))?;
    let stderr_file = fs::File::create(&stderr_path)
        .map_err(|_| SkillError::new("git_runtime_output_unavailable"))?;
    let mut command = Command::new(program);
    command
        .current_dir(&context.repository_root)
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        )
        .env("HOME", &runtime.root)
        .env("XDG_CONFIG_HOME", &runtime.root)
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS_REQUIRE", "force")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "maintenance.auto=false",
            "-c",
            "credential.helper=",
            "-c",
            "credential.useHttpPath=true",
            "-c",
            "http.followRedirects=false",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    for name in ["SSL_CERT_FILE", "SSL_CERT_DIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let (Some(askpass), Some(token), Some(username)) = (
        runtime.askpass.as_deref(),
        runtime.token.as_deref(),
        username,
    ) {
        command
            .env("GIT_ASKPASS", askpass)
            .env(GITHUB_GIT_TOKEN_ENV, token)
            .env("GITHUB_GIT_USERNAME", username);
    } else {
        command.env("GIT_ASKPASS", "/bin/false");
    }
    let mut child = command
        .spawn()
        .map_err(|_| SkillError::new("git_spawn_failed"))?;
    let status = match child
        .wait_timeout(timeout)
        .map_err(|_| SkillError::new("git_wait_failed"))?
    {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SkillError::new("git_command_timeout").retryable(true));
        }
    };
    let stdout = read_bounded(&stdout_path)?;
    let stderr = read_bounded(&stderr_path)?;
    Ok(GitOutput {
        status,
        stdout: runtime.redact(&stdout, username),
        stderr: runtime.redact(&stderr, username),
    })
}

fn exact_push_refspec(expected_local_sha: &str, remote_branch: &str) -> String {
    format!("{expected_local_sha}:refs/heads/{remote_branch}")
}

fn git_command_error(
    code: &'static str,
    runtime: &InvocationRuntime,
    output: &GitOutput,
    username: Option<&str>,
    phase: &'static str,
    applied: bool,
) -> SkillError {
    let diagnostic = runtime.redact(&format!("{}\n{}", output.stdout, output.stderr), username);
    SkillError::new(code)
        .phase(phase)
        .applied(applied)
        .retryable(code == "git_remote_request_failed")
        .with_extra(json!({
            "exit_code": output.status.code().unwrap_or(-1),
            "diagnostic": truncate(&diagnostic, 4096),
        }))
}

fn create_askpass(root: &Path) -> Result<PathBuf, SkillError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let path = root.join("git-askpass.sh");
        fs::write(
            &path,
            "#!/bin/sh\ncase \"$1\" in\n  *Username*) printf '%s\\n' \"$GITHUB_GIT_USERNAME\" ;;\n  *) printf '%s\\n' \"$GITHUB_GIT_TOKEN\" ;;\nesac\n",
        )
        .map_err(|_| SkillError::new("git_askpass_unavailable"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| SkillError::new("git_askpass_unavailable"))?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err(SkillError::new("git_platform_unsupported"))
    }
}

fn update_tracking_and_upstream(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
    remote: &str,
    remote_branch: &str,
    local_branch: &str,
    sha: &str,
) -> Result<(), SkillError> {
    let tracking_ref = format!("refs/remotes/{remote}/{remote_branch}");
    let update = run_git(
        context,
        runtime,
        &[
            "update-ref".to_string(),
            tracking_ref.clone(),
            sha.to_string(),
        ],
        None,
    )?;
    if !update.status.success() {
        return Err(SkillError::new("git_tracking_ref_update_failed"));
    }
    let upstream = run_git(
        context,
        runtime,
        &[
            "branch".to_string(),
            "--set-upstream-to".to_string(),
            format!("{remote}/{remote_branch}"),
            local_branch.to_string(),
        ],
        None,
    )?;
    if upstream.status.success() {
        Ok(())
    } else {
        Err(SkillError::new("git_upstream_update_failed"))
    }
}

fn ensure_fast_forward(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
    remote_sha: &str,
    local_sha: &str,
) -> Result<(), SkillError> {
    if optional_ref_sha(&context.repository_root, remote_sha)?.is_none() {
        return Err(SkillError::new("git_remote_object_missing_local"));
    }
    let output = run_git(
        context,
        runtime,
        &[
            "merge-base".to_string(),
            "--is-ancestor".to_string(),
            remote_sha.to_string(),
            local_sha.to_string(),
        ],
        None,
    )?;
    if output.status.success() {
        Ok(())
    } else if output.status.code() == Some(1) {
        Err(SkillError::new("git_non_fast_forward"))
    } else {
        Err(SkillError::new("git_ancestry_check_failed"))
    }
}

fn worktree_projection(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
) -> Result<Value, SkillError> {
    let output = run_git(
        context,
        runtime,
        &["status".to_string(), "--porcelain=v1".to_string()],
        None,
    )?;
    if !output.status.success() {
        return Err(SkillError::new("git_worktree_observation_failed"));
    }
    let changed_count = output
        .stdout
        .lines()
        .filter(|line| !line.is_empty())
        .count();
    Ok(json!({
        "worktree_state": if changed_count == 0 { "clean" } else { "dirty" },
        "changed_count": changed_count,
        "worktree_status_sha256": format!("sha256:{:x}", Sha256::digest(output.stdout.as_bytes())),
    }))
}

fn object_store_kib(
    context: &RepositoryContext,
    runtime: &InvocationRuntime,
) -> Result<u64, SkillError> {
    let output = run_git(
        context,
        runtime,
        &["count-objects".to_string(), "-v".to_string()],
        None,
    )?;
    if !output.status.success() {
        return Err(SkillError::new("git_object_store_observation_failed"));
    }
    let mut total = 0_u64;
    for line in output.stdout.lines() {
        if let Some(value) = line.strip_prefix("size: ") {
            total = total.saturating_add(value.trim().parse::<u64>().unwrap_or(0));
        } else if let Some(value) = line.strip_prefix("size-pack: ") {
            total = total.saturating_add(value.trim().parse::<u64>().unwrap_or(0));
        }
    }
    Ok(total)
}

fn require_url_digest(args: &Map<String, Value>, target: &RemoteTarget) -> Result<(), SkillError> {
    let expected = required_string(
        args,
        "expected_remote_url_digest",
        "git_expected_remote_url_digest_missing",
    )?;
    if expected != target.url_digest {
        return Err(
            SkillError::new("git_remote_url_precondition_changed").with_extra(json!({
                "expected": expected,
                "observed": target.url_digest,
            })),
        );
    }
    Ok(())
}

pub fn validated_branch(repository: &Path, branch: &str) -> Result<String, SkillError> {
    if branch.len() > 255
        || branch.starts_with('-')
        || branch.starts_with('+')
        || branch.chars().any(char::is_control)
    {
        return Err(SkillError::new("git_branch_invalid"));
    }
    let output = Command::new("git")
        .current_dir(repository)
        .args(["check-ref-format", "--branch", branch])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| SkillError::new("git_spawn_failed"))?;
    if output.status.success() {
        Ok(branch.to_string())
    } else {
        Err(SkillError::new("git_branch_invalid"))
    }
}

pub fn validated_sha(value: &str) -> Result<String, SkillError> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(SkillError::new("git_sha_invalid"))
    }
}

fn optional_sha_arg(args: &Map<String, Value>, key: &str) -> Result<Option<String>, SkillError> {
    optional_string(args, key, "git_sha_invalid")?
        .map(validated_sha)
        .transpose()
}

fn required_ref_sha(repository: &Path, reference: &str) -> Result<String, SkillError> {
    optional_ref_sha(repository, reference)?.ok_or_else(|| SkillError::new("git_ref_not_found"))
}

fn optional_ref_sha(repository: &Path, reference: &str) -> Result<Option<String>, SkillError> {
    if reference.starts_with('-') || reference.chars().any(char::is_control) {
        return Err(SkillError::new("git_ref_invalid"));
    }
    let output = Command::new("git")
        .current_dir(repository)
        .args(["rev-parse", "--verify", "--end-of-options", reference])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| SkillError::new("git_spawn_failed"))?;
    if !output.status.success() {
        return Ok(None);
    }
    validated_sha(String::from_utf8_lossy(&output.stdout).trim()).map(Some)
}

fn validate_machine_token(value: &str, error_code: &'static str) -> Result<(), SkillError> {
    if value.is_empty()
        || value.len() > 96
        || value.starts_with('.')
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(SkillError::new(error_code))
    } else {
        Ok(())
    }
}

pub fn encode_push_receipt_ref(receipt: &PushReceiptProjection) -> Result<String, SkillError> {
    let payload = serde_json::to_vec(receipt)
        .map_err(|_| SkillError::new("git_push_receipt_serialize_failed"))?;
    let digest = Sha256::digest(&payload);
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
    let digest = format!("{digest:x}");
    Ok([PUSH_RECEIPT_PREFIX, &encoded, &digest].join(":"))
}

pub fn decode_push_receipt_ref(value: &str) -> Result<PushReceiptProjection, SkillError> {
    let mut parts = value.split(':');
    if parts.next() != Some(PUSH_RECEIPT_PREFIX) {
        return Err(SkillError::new("git_push_receipt_ref_invalid"));
    }
    let encoded = parts
        .next()
        .ok_or_else(|| SkillError::new("git_push_receipt_ref_invalid"))?;
    let claimed = parts
        .next()
        .ok_or_else(|| SkillError::new("git_push_receipt_ref_invalid"))?;
    if parts.next().is_some() {
        return Err(SkillError::new("git_push_receipt_ref_invalid"));
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| SkillError::new("git_push_receipt_ref_invalid"))?;
    let observed = format!("{:x}", Sha256::digest(&payload));
    if observed != claimed {
        return Err(SkillError::new("git_push_receipt_digest_invalid"));
    }
    serde_json::from_slice(&payload).map_err(|_| SkillError::new("git_push_receipt_ref_invalid"))
}

fn deterministic_operation_id(value: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!("git-push-{}", &digest[..24])
}

fn evidence_digest(value: &Value) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}

fn plain_git_output(repository: &Path, args: &[&str]) -> Result<String, SkillError> {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| SkillError::new("git_spawn_failed"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(SkillError::new("not_git_repository"))
    }
}

fn read_bounded(path: &Path) -> Result<String, SkillError> {
    let file =
        fs::File::open(path).map_err(|_| SkillError::new("git_runtime_output_unavailable"))?;
    let mut bytes = Vec::new();
    file.take((MAX_GIT_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| SkillError::new("git_runtime_output_unavailable"))?;
    if bytes.len() > MAX_GIT_OUTPUT_BYTES {
        bytes.truncate(MAX_GIT_OUTPUT_BYTES);
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn ensure_fetch_disk_available(repository: &Path) -> Result<u64, SkillError> {
    let available = fs2::available_space(repository)
        .map_err(|_| SkillError::new("git_disk_status_unavailable"))?;
    enforce_fetch_disk_floor(available, MIN_FETCH_FREE_BYTES)?;
    Ok(available)
}

fn enforce_fetch_disk_floor(available: u64, required: u64) -> Result<(), SkillError> {
    if available < required {
        Err(
            SkillError::new("git_disk_space_insufficient").with_extra(json!({
                "available_bytes": available,
                "required_bytes": required,
            })),
        )
    } else {
        Ok(())
    }
}

fn command_timeout() -> Duration {
    let seconds = std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60)
        .clamp(5, 300);
    Duration::from_secs(seconds)
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(unix)]
fn apply_private_directory_permissions(path: &Path) -> Result<(), SkillError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| SkillError::new("git_runtime_temp_unavailable"))
}

#[cfg(not(unix))]
fn apply_private_directory_permissions(_path: &Path) -> Result<(), SkillError> {
    Err(SkillError::new("git_platform_unsupported"))
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
