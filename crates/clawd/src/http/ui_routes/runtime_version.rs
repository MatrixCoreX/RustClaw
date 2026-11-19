#[derive(Debug, Clone, Serialize)]
struct RuntimeVersionSummary {
    package_version: &'static str,
    build_revision: Option<String>,
    source_revision: Option<String>,
    source_match: &'static str,
}
impl Default for RuntimeVersionSummary {
    fn default() -> Self {
        Self::from_revisions(option_env!("APP_BUILD_REVISION").unwrap_or_default(), None)
    }
}
impl RuntimeVersionSummary {
    fn from_revisions(build: &str, source: Option<String>) -> Self {
        let build_revision = valid_build_revision(build).then(|| build.to_string());
        let source_revision = source.filter(|v| valid_build_revision(v));
        let source_match = match (&build_revision, &source_revision) {
            (Some(a), Some(b)) if a == b => "matched",
            (Some(_), Some(_)) => "different",
            _ => "unavailable",
        };
        Self {
            package_version: env!("CARGO_PKG_VERSION"),
            build_revision,
            source_revision,
            source_match,
        }
    }
}
fn valid_build_revision(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
async fn collect_runtime_version(root: &Path) -> RuntimeVersionSummary {
    let source = run_workspace_update_command("git", &["rev-parse", "--verify", "HEAD"], root, 2)
        .await
        .ok()
        .filter(|out| out.exit_code == Some(0))
        .map(|out| out.stdout_tail.trim().to_string());
    RuntimeVersionSummary::from_revisions(
        option_env!("APP_BUILD_REVISION").unwrap_or_default(),
        source,
    )
}
