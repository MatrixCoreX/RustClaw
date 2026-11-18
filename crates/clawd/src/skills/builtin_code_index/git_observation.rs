use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use claw_core::config::{ToolSandboxBackend, ToolSandboxMode};

use super::{normalize_relative_path, CodeIndexError, HostFilesystemPolicy};

const MAX_METADATA_BYTES: u64 = 64 * 1024;
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ATTRIBUTE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_METADATA_BYTES: u64 = 128 * 1024 * 1024;
const MAX_REPOSITORIES: usize = 32;
const MAX_SUBMODULE_DEPTH: usize = 8;

struct ObservationBudget {
    metadata_bytes_left: u64,
    repositories_left: usize,
    visited: BTreeSet<PathBuf>,
}

struct RepositoryLayout {
    worktree: PathBuf,
    git_directory: PathBuf,
    common_directory: PathBuf,
}

impl RepositoryLayout {
    fn discover(
        root: &Path,
        filesystem: &HostFilesystemPolicy,
    ) -> Result<Option<Self>, CodeIndexError> {
        let selected = admitted_directory(filesystem, root)?;
        for ancestor in selected.ancestors() {
            let git = ancestor.join(".git");
            let directory = match filesystem.open_directory(Some(ancestor), &git) {
                Ok(directory) => directory.canonical_path().to_path_buf(),
                Err(error) if HostFilesystemPolicy::is_private_access_denied(&error) => {
                    return Err(metadata_error(error));
                }
                Err(_) => {
                    let Some(bytes) =
                        read_optional(filesystem, ancestor, &git, MAX_METADATA_BYTES)?
                    else {
                        continue;
                    };
                    let target = bytes
                        .strip_prefix(b"gitdir: ")
                        .ok_or_else(invalid_metadata)?;
                    admitted_directory(filesystem, &ancestor.join(metadata_path(target)?))?
                }
            };
            let common = match read_optional(
                filesystem,
                &directory,
                &directory.join("commondir"),
                MAX_METADATA_BYTES,
            )? {
                Some(bytes) => {
                    admitted_directory(filesystem, &directory.join(metadata_path(&bytes)?))?
                }
                None => directory.clone(),
            };
            return Ok(Some(Self {
                worktree: ancestor.to_path_buf(),
                git_directory: directory,
                common_directory: common,
            }));
        }
        Ok(None)
    }

    fn readable_directories(&self) -> Vec<PathBuf> {
        [
            self.worktree.clone(),
            self.git_directory.clone(),
            self.common_directory.clone(),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
    }
}

// Git still owns object/ref/index interpretation. Only its two documented
// directory pointer files are resolved here to admit the exact read-only roots.
fn metadata_path(bytes: &[u8]) -> Result<PathBuf, CodeIndexError> {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(invalid_metadata());
    }
    Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}

fn admitted_directory(
    filesystem: &HostFilesystemPolicy,
    path: &Path,
) -> Result<PathBuf, CodeIndexError> {
    filesystem
        .open_directory(None, path)
        .map(|directory| directory.canonical_path().to_path_buf())
        .map_err(metadata_error)
}

fn invalid_metadata() -> CodeIndexError {
    CodeIndexError::new("git_metadata_invalid", "code_index.git_metadata_invalid")
}

fn metadata_error(error: io::Error) -> CodeIndexError {
    if HostFilesystemPolicy::is_private_access_denied(&error) {
        CodeIndexError::new(
            "host_private_directory_access_denied",
            "code_index.private_git_metadata_denied",
        )
    } else {
        CodeIndexError::new(
            "git_metadata_read_failed",
            "code_index.git_metadata_read_failed",
        )
    }
}

fn read_optional(
    filesystem: &HostFilesystemPolicy,
    root: &Path,
    path: &Path,
    limit: u64,
) -> Result<Option<Vec<u8>>, CodeIndexError> {
    let file = match filesystem.open_file(Some(root), path) {
        Ok((file, _)) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(metadata_error(error)),
    };
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(metadata_error)?;
    if bytes.len() as u64 > limit {
        return Err(CodeIndexError::new(
            "git_metadata_too_large",
            "code_index.git_metadata_too_large",
        ));
    }
    Ok(Some(bytes))
}

pub(super) fn prepare_query(
    root: &Path,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
    readonly: &[PathBuf],
    writable: &[PathBuf],
) -> Result<(Command, Vec<PathBuf>), CodeIndexError> {
    let private = filesystem.process_directories().map_err(|_| {
        CodeIndexError::new("sandbox_policy_changed", "code_index.host_policy_changed")
    })?;
    let prepared = crate::process_sandbox::prepare_readonly_process_command(
        "git",
        crate::process_sandbox::ProcessSandboxRequest {
            mode: ToolSandboxMode::ReadOnly,
            backend,
            workspace_root: root,
            execution_root: root,
            network: crate::process_sandbox::ProcessNetworkPolicy::Deny,
            additional_writable_paths: writable,
            host_private_directories: &private,
            browser_subprocess: false,
        },
        readonly,
    )
    .map_err(|code| CodeIndexError::new("sandbox_unavailable", code))?;
    let mut command = prepared.command.into_std();
    command
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "core.excludesFile=/dev/null",
        ])
        .current_dir(root)
        .env_clear()
        .envs(crate::skills::SkillRunnerEnvironment::capture().into_pairs())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::null());
    Ok((command, prepared.additional_writable_targets))
}

fn run_query(mut command: Command, args: &[&str]) -> Result<Output, CodeIndexError> {
    command
        .args(args)
        .output()
        .map_err(|_| CodeIndexError::new("git_launch_failed", "code_index.git_launch_failed"))
}

struct ObservationRepository {
    temporary: tempfile::TempDir,
    worktree: PathBuf,
    objects: PathBuf,
    git_directory: PathBuf,
}

impl ObservationRepository {
    fn create(
        root: &Path,
        filesystem: &HostFilesystemPolicy,
        backend: ToolSandboxBackend,
        nested: bool,
        budget: &mut ObservationBudget,
    ) -> Result<Option<Self>, CodeIndexError> {
        // Prepare even a non-repository query: an unavailable backend must fail
        // closed rather than becoming an apparently successful empty result.
        prepare_query(root, filesystem, backend, &[], &[])?;
        let Some(layout) = RepositoryLayout::discover(root, filesystem)? else {
            return Ok(None);
        };
        if nested && layout.worktree != admitted_directory(filesystem, root)? {
            return Ok(None);
        }
        let readable = layout.readable_directories();
        let discover = |args: &[&str]| -> Result<Output, CodeIndexError> {
            run_query(
                prepare_query(root, filesystem, backend, &readable, &[])?.0,
                args,
            )
        };
        let format = discover(&["rev-parse", "--show-object-format"])?;
        if !format.status.success() {
            return Err(invalid_metadata());
        }
        let format = format.stdout.strip_suffix(b"\n").unwrap_or(&format.stdout);
        if format != b"sha1" && format != b"sha256" {
            return Err(invalid_metadata());
        }
        let mut config = if format == b"sha256" {
            String::from("[core]\nrepositoryformatversion = 1\nbare = false\n[extensions]\nobjectformat = sha256\n")
        } else {
            String::from("[core]\nrepositoryformatversion = 0\nbare = false\n")
        };
        let settings = discover(&["config", "--null", "--get-regexp", "^core\\.(filemode|ignorecase|precomposeunicode|autocrlf|eol|symlinks|checkstat|trustctime)$"])?;
        if !settings.status.success() && settings.status.code() != Some(1) {
            return Err(invalid_metadata());
        }
        config.push_str(&safe_core_settings(&settings.stdout)?);
        let temporary = tempfile::Builder::new()
            .prefix("agent-index-git-")
            .tempdir()
            .map_err(|_| {
                CodeIndexError::new(
                    "git_scratch_unavailable",
                    "code_index.git_scratch_unavailable",
                )
            })?;
        std::fs::create_dir(temporary.path().join("refs")).map_err(metadata_error)?;
        std::fs::create_dir(temporary.path().join("info")).map_err(metadata_error)?;
        std::fs::create_dir(temporary.path().join("objects")).map_err(metadata_error)?;
        std::fs::write(temporary.path().join("config"), config).map_err(metadata_error)?;
        let head = discover(&["rev-parse", "--verify", "HEAD"])?;
        let head_bytes = if head.status.success() {
            let oid = head.stdout.strip_suffix(b"\n").unwrap_or(&head.stdout);
            if oid.len() != if format == b"sha256" { 64 } else { 40 }
                || !oid.iter().all(u8::is_ascii_hexdigit)
            {
                return Err(invalid_metadata());
            }
            head.stdout
        } else {
            b"ref: refs/heads/observation\n".to_vec()
        };
        std::fs::write(temporary.path().join("HEAD"), head_bytes).map_err(metadata_error)?;
        copy_optional(
            filesystem,
            &layout.git_directory,
            "index",
            temporary.path(),
            MAX_INDEX_BYTES,
            budget,
        )?;
        let shared = discover(&["rev-parse", "--shared-index-path"])?;
        if !shared.status.success() {
            return Err(invalid_metadata());
        }
        if shared.stdout != b"\n" && !shared.stdout.is_empty() {
            let path = metadata_path(&shared.stdout)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(invalid_metadata)?;
            let suffix = name
                .strip_prefix("sharedindex.")
                .ok_or_else(invalid_metadata)?;
            if !matches!(suffix.len(), 40 | 64)
                || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(invalid_metadata());
            }
            copy_optional(
                filesystem,
                &layout.git_directory,
                name,
                temporary.path(),
                MAX_INDEX_BYTES,
                budget,
            )?;
        }
        for name in ["info/exclude", "info/attributes"] {
            copy_optional(
                filesystem,
                &layout.common_directory,
                name,
                temporary.path(),
                MAX_ATTRIBUTE_BYTES,
                budget,
            )?;
        }
        let objects = admitted_directory(filesystem, &layout.common_directory.join("objects"))?;
        Ok(Some(Self {
            temporary,
            worktree: layout.worktree,
            objects,
            git_directory: layout.git_directory,
        }))
    }

    fn command(
        &self,
        root: &Path,
        filesystem: &HostFilesystemPolicy,
        backend: ToolSandboxBackend,
    ) -> Result<Command, CodeIndexError> {
        let (mut command, targets) = prepare_query(
            root,
            filesystem,
            backend,
            &[self.worktree.clone(), self.objects.clone()],
            &[self.temporary.path().to_path_buf()],
        )?;
        let directory = targets.first().ok_or_else(invalid_metadata)?;
        command
            .env("GIT_DIR", directory)
            .env("GIT_COMMON_DIR", directory)
            .env("GIT_WORK_TREE", &self.worktree)
            .env("GIT_OBJECT_DIRECTORY", &self.objects);
        Ok(command)
    }
}

fn copy_optional(
    filesystem: &HostFilesystemPolicy,
    source: &Path,
    name: &str,
    target: &Path,
    limit: u64,
    budget: &mut ObservationBudget,
) -> Result<(), CodeIndexError> {
    let file = match filesystem.open_file(Some(source), &source.join(name)) {
        Ok((file, _)) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(metadata_error(error)),
    };
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target.join(name))
        .map_err(metadata_error)?;
    let allowed = limit.min(budget.metadata_bytes_left);
    let copied = io::copy(&mut file.take(allowed + 1), &mut target).map_err(metadata_error)?;
    if copied > allowed {
        return Err(CodeIndexError::new(
            "git_metadata_budget_exceeded",
            "code_index.git_metadata_budget_exceeded",
        ));
    }
    budget.metadata_bytes_left -= copied;
    Ok(())
}

fn safe_core_settings(bytes: &[u8]) -> Result<String, CodeIndexError> {
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(invalid_metadata());
    }
    let mut config = String::from("[core]\n");
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let entry = std::str::from_utf8(entry).map_err(|_| invalid_metadata())?;
        let (key, value) = entry.split_once('\n').unwrap_or((entry, "true"));
        let key = key.to_ascii_lowercase();
        let name = key.strip_prefix("core.").ok_or_else(invalid_metadata)?;
        let mut value = value.trim().to_ascii_lowercase();
        let boolean = match value.as_str() {
            "true" | "yes" | "on" | "1" => Some("true"),
            "" | "false" | "no" | "off" | "0" => Some("false"),
            _ => None,
        };
        let safe = match name {
            "filemode" | "ignorecase" | "precomposeunicode" | "symlinks" | "trustctime" => {
                if let Some(boolean) = boolean {
                    value = boolean.to_string();
                    true
                } else {
                    false
                }
            }
            "autocrlf" => {
                if let Some(boolean) = boolean {
                    value = boolean.to_string();
                    true
                } else {
                    value == "input"
                }
            }
            "eol" => matches!(value.as_str(), "lf" | "crlf" | "native"),
            "checkstat" => matches!(value.as_str(), "default" | "minimal"),
            _ => false,
        };
        if !safe {
            return Err(invalid_metadata());
        }
        config.push_str(&format!("{name} = {value}\n"));
    }
    Ok(config)
}

pub(super) struct ChangedPaths {
    pub(super) paths: Vec<String>,
    pub(super) excluded_paths: usize,
    pub(super) query_failures: usize,
    pub(super) uninspected_submodules: usize,
}

fn uninspected_repository() -> ChangedPaths {
    ChangedPaths {
        paths: Vec::new(),
        excluded_paths: 0,
        query_failures: 0,
        uninspected_submodules: 1,
    }
}

pub(super) fn changed_paths(
    root: &Path,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
) -> Result<ChangedPaths, CodeIndexError> {
    observe_repository(
        root,
        filesystem,
        backend,
        0,
        &mut ObservationBudget {
            metadata_bytes_left: MAX_TOTAL_METADATA_BYTES,
            repositories_left: MAX_REPOSITORIES,
            visited: BTreeSet::new(),
        },
    )
}

fn observe_repository(
    root: &Path,
    filesystem: &HostFilesystemPolicy,
    backend: ToolSandboxBackend,
    depth: usize,
    budget: &mut ObservationBudget,
) -> Result<ChangedPaths, CodeIndexError> {
    if depth > MAX_SUBMODULE_DEPTH || budget.repositories_left == 0 {
        return Ok(uninspected_repository());
    }
    budget.repositories_left -= 1;
    let Some(repository) =
        ObservationRepository::create(root, filesystem, backend, depth > 0, budget)?
    else {
        if depth > 0 {
            return Ok(uninspected_repository());
        }
        return Ok(ChangedPaths {
            paths: Vec::new(),
            excluded_paths: 0,
            query_failures: 2,
            uninspected_submodules: 0,
        });
    };
    if !budget.visited.insert(repository.git_directory.clone()) {
        return Ok(uninspected_repository());
    }
    let mut paths = BTreeSet::new();
    let mut failures = 0;
    for args in [
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=dirty",
            "--name-only",
            "-z",
            "--relative",
            "HEAD",
        ],
        vec!["ls-files", "-z", "--others", "--exclude-standard"],
    ] {
        let output = run_query(repository.command(root, filesystem, backend)?, &args)?;
        if !output.status.success() {
            failures += 1;
            continue;
        }
        for raw in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|raw| !raw.is_empty())
        {
            let path = std::str::from_utf8(raw)
                .ok()
                .and_then(|path| normalize_relative_path(path).ok());
            match path {
                Some(path) if !path.is_empty() => {
                    paths.insert(path);
                }
                _ => failures += 1,
            }
        }
    }
    // Git itself must not run nested status commands. Each populated gitlink
    // receives the same isolated observer and the shared traversal budget.
    let submodules = run_query(
        repository.command(root, filesystem, backend)?,
        &["ls-files", "--stage", "-z"],
    )?;
    let mut excluded = 0;
    let mut uninspected_submodules = 0;
    if submodules.status.success() {
        for entry in submodules
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| entry.starts_with(b"160000 "))
        {
            let path = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .and_then(|tab| std::str::from_utf8(&entry[tab + 1..]).ok())
                .and_then(|path| normalize_relative_path(path).ok());
            let Some(path) = path.filter(|path| !path.is_empty()) else {
                failures += 1;
                uninspected_submodules += 1;
                continue;
            };
            let child_root = root.join(&path);
            match filesystem.open_directory(Some(root), &child_root) {
                Ok(_) => {}
                Err(error) => {
                    if HostFilesystemPolicy::is_private_access_denied(&error) {
                        excluded += 1;
                    }
                    uninspected_submodules += 1;
                    continue;
                }
            }
            match observe_repository(&child_root, filesystem, backend, depth + 1, budget) {
                Ok(child) => {
                    if !child.paths.is_empty() {
                        paths.insert(path.clone());
                    }
                    for changed in child.paths {
                        paths.insert(format!("{path}/{changed}"));
                    }
                    excluded += child.excluded_paths;
                    failures += child.query_failures;
                    uninspected_submodules += child.uninspected_submodules;
                }
                Err(error)
                    if matches!(error.code, "sandbox_unavailable" | "sandbox_policy_changed") =>
                {
                    return Err(error)
                }
                Err(error) => {
                    if error.code == "host_private_directory_access_denied" {
                        excluded += 1;
                    } else {
                        failures += 1;
                    }
                    uninspected_submodules += 1;
                }
            }
        }
    } else {
        failures += 1;
    }
    paths.retain(|path| {
        let denied = filesystem
            .open_file(Some(root), &root.join(path))
            .err()
            .is_some_and(|error| HostFilesystemPolicy::is_private_access_denied(&error));
        if denied {
            excluded += 1;
        }
        !denied
    });
    Ok(ChangedPaths {
        paths: paths.into_iter().collect(),
        excluded_paths: excluded,
        query_failures: failures,
        uninspected_submodules,
    })
}

#[cfg(test)]
#[path = "git_observation_tests.rs"]
mod tests;
