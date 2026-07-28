#[derive(Debug)]
enum WorkspaceUpdateUpstreamError {
    Command(WorkspaceUpdateCommandOutput),
    Runtime(String),
}

async fn resolve_workspace_update_remote_commit(
    workspace_root: &Path,
) -> Result<Option<String>, WorkspaceUpdateUpstreamError> {
    let first_attempt = run_workspace_update_command(
        "git",
        &["rev-parse", "--short", "@{upstream}"],
        workspace_root,
        30,
    )
    .await
    .map_err(WorkspaceUpdateUpstreamError::Runtime)?;
    if first_attempt.exit_code == Some(0) {
        return Ok(first_output_line(&first_attempt.stdout_tail));
    }

    let branch = run_workspace_update_command(
        "git",
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        workspace_root,
        30,
    )
    .await
    .map_err(WorkspaceUpdateUpstreamError::Runtime)?;
    if branch.exit_code != Some(0) {
        return Err(WorkspaceUpdateUpstreamError::Command(first_attempt));
    }
    let Some(branch) = first_output_line(&branch.stdout_tail) else {
        return Err(WorkspaceUpdateUpstreamError::Command(first_attempt));
    };

    let remote_refs = run_workspace_update_command(
        "git",
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/remotes/",
        ],
        workspace_root,
        30,
    )
    .await
    .map_err(WorkspaceUpdateUpstreamError::Runtime)?;
    if remote_refs.exit_code != Some(0) {
        return Err(WorkspaceUpdateUpstreamError::Command(remote_refs));
    }
    let Some(upstream) = workspace_update_upstream_candidate(&branch, &remote_refs.stdout_tail)
    else {
        return Err(WorkspaceUpdateUpstreamError::Command(first_attempt));
    };

    let args = vec![
        "branch".to_string(),
        format!("--set-upstream-to={upstream}"),
        branch,
    ];
    let configured = run_workspace_update_command_args("git", &args, workspace_root, 30)
        .await
        .map_err(WorkspaceUpdateUpstreamError::Runtime)?;
    if configured.exit_code != Some(0) {
        return Err(WorkspaceUpdateUpstreamError::Command(configured));
    }

    let resolved = run_workspace_update_command(
        "git",
        &["rev-parse", "--short", "@{upstream}"],
        workspace_root,
        30,
    )
    .await
    .map_err(WorkspaceUpdateUpstreamError::Runtime)?;
    if resolved.exit_code == Some(0) {
        Ok(first_output_line(&resolved.stdout_tail))
    } else {
        Err(WorkspaceUpdateUpstreamError::Command(resolved))
    }
}

fn workspace_update_upstream_candidate(branch: &str, remote_refs: &str) -> Option<String> {
    let suffix = format!("/{branch}");
    let mut candidates = remote_refs
        .lines()
        .map(str::trim)
        .filter(|reference| reference.ends_with(&suffix))
        .map(str::to_string)
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();

    let origin = format!("origin/{branch}");
    if candidates.iter().any(|candidate| candidate == &origin) {
        return Some(origin);
    }
    (candidates.len() == 1).then(|| candidates.remove(0))
}
