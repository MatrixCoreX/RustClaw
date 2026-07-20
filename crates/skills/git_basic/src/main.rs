use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Value as TomlValue;

static I18N: OnceLock<TextCatalog> = OnceLock::new();
const SKILL_NAME: &str = "git_basic";

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct GitBasicConfig {
    #[serde(default)]
    git_basic: GitBasicSection,
}

#[derive(Debug, Deserialize, Default)]
struct GitBasicSection {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

fn tr(key: &str) -> String {
    I18N.get()
        .and_then(|c| c.current.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn tr_with(key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = tr(key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn default_catalog(lang: &str) -> TextCatalog {
    let mut current = HashMap::new();
    let _ = lang;
    current.insert(
        "git_basic.err.invalid_input".to_string(),
        "invalid input: {error}".to_string(),
    );
    current.insert(
        "git_basic.err.args_object".to_string(),
        "args must be object".to_string(),
    );
    current.insert(
        "git_basic.msg.not_git_repo".to_string(),
        "current directory is not a git repository. Please use git_basic in a git repo."
            .to_string(),
    );
    current.insert(
        "git_basic.err.unsupported_action".to_string(),
        "unsupported action; use status|log|diff|branch|show|rev_parse|diff_cached|current_branch|remote|changed_files|show_file_at_rev".to_string(),
    );
    current.insert(
        "git_basic.err.run_git_failed".to_string(),
        "run git failed: {error}".to_string(),
    );
    TextCatalog { current }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: TomlValue = toml::from_str(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        if let Some(text) = v.as_str() {
            out.insert(k.to_string(), text.to_string());
        }
    }
    Some(out)
}

fn load_git_basic_config(workspace_root: &Path) -> GitBasicConfig {
    let path = workspace_root.join("configs/git_basic.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return GitBasicConfig::default(),
    };
    toml::from_str::<GitBasicConfig>(&raw).unwrap_or_default()
}

fn init_i18n(workspace_root: &Path) {
    let cfg = load_git_basic_config(workspace_root);
    let lang = cfg
        .git_basic
        .language
        .as_deref()
        .unwrap_or("zh-CN")
        .trim()
        .to_string();
    let mut catalog = default_catalog(&lang);
    let path = cfg
        .git_basic
        .i18n_path
        .as_deref()
        .map(|p| workspace_root.join(p))
        .unwrap_or_else(|| workspace_root.join(format!("configs/i18n/git_basic.{lang}.toml")));
    if let Some(overrides) = load_external_i18n(&path) {
        for (k, v) in overrides {
            catalog.current.insert(k, v);
        }
    }
    let _ = I18N.set(catalog);
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    init_i18n(&workspace_root);

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: Some(error_extra("execution_failed")),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(tr_with(
                    "git_basic.err.invalid_input",
                    &[("error", &err.to_string())],
                )),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execute(args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| tr("git_basic.err.args_object"))?;
    let raw_action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");
    let action = normalize_action(raw_action);
    let root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if !is_git_repo(&root) {
        return Err(tr("git_basic.msg.not_git_repo"));
    }

    let log_n = obj
        .get("n")
        .or_else(|| obj.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(100);

    let mut input_meta = Map::new();
    let (subcmd, mut extra): (&str, Vec<String>) = match action.as_str() {
        "status" => (
            "status",
            vec!["--short".to_string(), "--branch".to_string()],
        ),
        "log" => (
            "log",
            vec!["--oneline".to_string(), "-n".to_string(), log_n.to_string()],
        ),
        "diff" => ("diff", vec![]),
        "diff_cached" => ("diff", vec!["--cached".to_string()]),
        "branch" => ("branch", vec!["--all".to_string()]),
        "current_branch" => (
            "rev-parse",
            vec!["--abbrev-ref".to_string(), "HEAD".to_string()],
        ),
        "remote" => ("remote", vec!["-v".to_string()]),
        "changed_files" => ("diff", vec!["--name-only".to_string(), "HEAD".to_string()]),
        "show" => {
            let target = obj.get("target").and_then(|v| v.as_str()).unwrap_or("HEAD");
            input_meta.insert("target".to_string(), json!(target));
            ("show", vec!["--stat".to_string(), target.to_string()])
        }
        "show_file_at_rev" => {
            let target = obj.get("target").and_then(|v| v.as_str()).unwrap_or("HEAD");
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return Err("show_file_at_rev requires path".to_string());
            }
            input_meta.insert("target".to_string(), json!(target));
            input_meta.insert("revision".to_string(), json!(target));
            input_meta.insert("path".to_string(), json!(path));
            input_meta.insert("source".to_string(), json!("git_show_file_at_rev"));
            input_meta.insert("source_kind".to_string(), json!("git_revision_file"));
            ("show", vec![format!("{}:{}", target, path)])
        }
        "rev_parse" => ("rev-parse", vec!["HEAD".to_string()]),
        _ => {
            return Err(tr("git_basic.err.unsupported_action"));
        }
    };

    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .arg(subcmd)
        .args(extra.drain(..))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let out = cmd.output().map_err(|err| {
        tr_with(
            "git_basic.err.run_git_failed",
            &[("error", &err.to_string())],
        )
    })?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    const MAX_LEN: usize = 12000;
    let marker = "\n...(truncated)";
    let max_bytes = MAX_LEN.saturating_sub(marker.len());
    if text.len() > MAX_LEN {
        let mut boundary = 0usize;
        for (i, c) in text.char_indices() {
            if i + c.len_utf8() > max_bytes {
                break;
            }
            boundary = i + c.len_utf8();
        }
        text.truncate(boundary);
        text.push_str(marker);
    }

    let exit_code = out.status.code().unwrap_or(-1);
    if out.status.success() {
        let output = format!("exit={exit_code}\n{text}");
        let extra = git_success_extra(
            action.as_str(),
            raw_action,
            subcmd,
            exit_code,
            &text,
            &output,
            Some(&input_meta),
        );
        Ok((output.clone(), extra))
    } else {
        Err(format!("git command failed: exit={exit_code}\n{text}"))
    }
}

fn git_success_extra(
    action: &str,
    raw_action: &str,
    subcommand: &str,
    exit_code: i32,
    command_text: &str,
    output: &str,
    input_meta: Option<&Map<String, Value>>,
) -> Value {
    let mut root = Map::new();
    root.insert("schema_version".to_string(), json!(1));
    root.insert("action".to_string(), json!(action));
    root.insert("raw_action".to_string(), json!(raw_action));
    root.insert("subcommand".to_string(), json!(subcommand));
    root.insert("exit_code".to_string(), json!(exit_code));
    root.insert("output".to_string(), json!(output));
    let mut field_value = Map::new();
    field_value.insert("exit_code".to_string(), json!(exit_code));
    field_value.insert("action".to_string(), json!(action));
    if let Some(input_meta) = input_meta {
        for (key, value) in input_meta {
            root.insert(key.clone(), value.clone());
            field_value.insert(key.clone(), value.clone());
        }
    }

    match action {
        "status" => append_git_status_extra(command_text, &mut root, &mut field_value),
        "current_branch" => append_current_branch_extra(command_text, &mut root, &mut field_value),
        "changed_files" => append_changed_files_extra(command_text, &mut root, &mut field_value),
        "log" => append_git_log_extra(command_text, &mut root, &mut field_value),
        "rev_parse" => append_rev_parse_extra(command_text, &mut root, &mut field_value),
        "branch" => append_branch_list_extra(command_text, &mut root, &mut field_value),
        "remote" => append_remote_list_extra(command_text, &mut root, &mut field_value),
        "show_file_at_rev" => {
            append_show_file_at_rev_extra(command_text, &mut root, &mut field_value)
        }
        _ => {}
    }

    root.insert("field_value".to_string(), Value::Object(field_value));
    Value::Object(root)
}

fn append_git_status_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let summary = parse_git_status_summary(text);
    if let Some(branch) = summary.branch.as_deref() {
        root.insert("branch".to_string(), json!(branch));
        root.insert("current_branch".to_string(), json!(branch));
        field_value.insert("branch".to_string(), json!(branch));
        field_value.insert("current_branch".to_string(), json!(branch));
    }
    if let Some(upstream) = summary.upstream.as_deref() {
        root.insert("upstream".to_string(), json!(upstream));
        field_value.insert("upstream".to_string(), json!(upstream));
    }
    if let Some(ahead) = summary.ahead {
        root.insert("ahead".to_string(), json!(ahead));
        field_value.insert("ahead".to_string(), json!(ahead));
    }
    if let Some(behind) = summary.behind {
        root.insert("behind".to_string(), json!(behind));
        field_value.insert("behind".to_string(), json!(behind));
    }
    root.insert("clean".to_string(), json!(summary.clean));
    root.insert(
        "worktree_state".to_string(),
        json!(if summary.clean { "clean" } else { "dirty" }),
    );
    root.insert("changed_count".to_string(), json!(summary.changed_count));
    root.insert("staged_count".to_string(), json!(summary.staged_count));
    root.insert("unstaged_count".to_string(), json!(summary.unstaged_count));
    root.insert(
        "untracked_count".to_string(),
        json!(summary.untracked_count),
    );
    root.insert(
        "changed_files".to_string(),
        json!(summary.changed_files.clone()),
    );
    root.insert("paths".to_string(), json!(summary.changed_files.clone()));
    field_value.insert("clean".to_string(), json!(summary.clean));
    field_value.insert(
        "worktree_state".to_string(),
        json!(if summary.clean { "clean" } else { "dirty" }),
    );
    field_value.insert("changed_count".to_string(), json!(summary.changed_count));
    field_value.insert("staged_count".to_string(), json!(summary.staged_count));
    field_value.insert("unstaged_count".to_string(), json!(summary.unstaged_count));
    field_value.insert(
        "untracked_count".to_string(),
        json!(summary.untracked_count),
    );
    field_value.insert("paths".to_string(), json!(summary.changed_files.clone()));
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GitStatusSummary {
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    clean: bool,
    changed_count: usize,
    staged_count: usize,
    unstaged_count: usize,
    untracked_count: usize,
    changed_files: Vec<String>,
}

fn parse_git_status_summary(text: &str) -> GitStatusSummary {
    let mut summary = GitStatusSummary::default();
    for line in text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
    {
        if let Some(header) = line.strip_prefix("## ") {
            parse_git_status_branch_header(header, &mut summary);
            continue;
        }
        if line.len() < 3 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].trim();
        if path.is_empty() {
            continue;
        }
        summary.changed_count += 1;
        if code == "??" {
            summary.untracked_count += 1;
        } else {
            let bytes = code.as_bytes();
            if bytes.first().is_some_and(|ch| *ch != b' ') {
                summary.staged_count += 1;
            }
            if bytes.get(1).is_some_and(|ch| *ch != b' ') {
                summary.unstaged_count += 1;
            }
        }
        summary.changed_files.push(normalize_git_status_path(path));
    }
    summary.clean = summary.changed_count == 0;
    summary
}

fn parse_git_status_branch_header(header: &str, summary: &mut GitStatusSummary) {
    let branch_part = header
        .split_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(header)
        .trim();
    if let Some((branch, upstream)) = branch_part.split_once("...") {
        if !branch.trim().is_empty() {
            summary.branch = Some(branch.trim().to_string());
        }
        if !upstream.trim().is_empty() {
            summary.upstream = Some(upstream.trim().to_string());
        }
    } else if !branch_part.is_empty() {
        summary.branch = Some(branch_part.to_string());
    }
    if let Some((_, bracket_tail)) = header.split_once('[') {
        if let Some((bracket, _)) = bracket_tail.split_once(']') {
            parse_git_status_ahead_behind(bracket, summary);
        }
    }
}

fn parse_git_status_ahead_behind(bracket: &str, summary: &mut GitStatusSummary) {
    let parts = bracket
        .split([',', ' '])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    for pair in parts.windows(2) {
        match pair[0] {
            "ahead" => summary.ahead = pair[1].parse::<u64>().ok(),
            "behind" => summary.behind = pair[1].parse::<u64>().ok(),
            _ => {}
        }
    }
}

fn normalize_git_status_path(path: &str) -> String {
    path.split(" -> ")
        .last()
        .unwrap_or(path)
        .trim()
        .trim_matches('"')
        .to_string()
}

fn append_current_branch_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    if let Some(branch) = first_non_empty_line(text) {
        root.insert("branch".to_string(), json!(branch));
        root.insert("current_branch".to_string(), json!(branch));
        field_value.insert("branch".to_string(), json!(branch));
        field_value.insert("current_branch".to_string(), json!(branch));
    }
}

fn append_changed_files_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let files = non_empty_lines(text);
    root.insert("changed_files".to_string(), json!(files.clone()));
    root.insert("paths".to_string(), json!(files.clone()));
    root.insert("changed_count".to_string(), json!(files.len()));
    field_value.insert("changed_count".to_string(), json!(files.len()));
    field_value.insert("paths".to_string(), json!(files));
}

fn append_git_log_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let commits = parse_git_log_commits(text);
    let subjects = commits
        .iter()
        .filter_map(|commit| commit.get("subject").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    root.insert("commits".to_string(), json!(commits));
    root.insert("commit_count".to_string(), json!(subjects.len()));
    if let Some(subject) = subjects.first() {
        root.insert("subject".to_string(), json!(subject));
        field_value.insert("subject".to_string(), json!(subject));
    }
    root.insert("subjects".to_string(), json!(subjects));
    field_value.insert("commit_count".to_string(), json!(subjects.len()));
}

fn parse_git_log_commits(text: &str) -> Vec<Value> {
    non_empty_lines(text)
        .into_iter()
        .filter_map(|line| {
            let (sha, subject) = line.split_once(' ')?;
            Some(json!({
                "sha": sha,
                "subject": subject.trim(),
            }))
        })
        .collect()
}

fn append_rev_parse_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    if let Some(revision) = first_non_empty_line(text) {
        root.insert("revision".to_string(), json!(revision));
        field_value.insert("revision".to_string(), json!(revision));
    }
}

fn append_branch_list_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let branches = parse_branch_list(text);
    let current_branch = branches
        .iter()
        .find(|branch| branch.get("current").and_then(Value::as_bool) == Some(true))
        .and_then(|branch| branch.get("name").and_then(Value::as_str))
        .map(str::to_string);
    root.insert("branches".to_string(), json!(branches));
    root.insert("branch_count".to_string(), json!(branches.len()));
    if let Some(branch) = current_branch {
        root.insert("current_branch".to_string(), json!(branch));
        field_value.insert("current_branch".to_string(), json!(branch));
    }
    field_value.insert("branch_count".to_string(), json!(branches.len()));
}

fn parse_branch_list(text: &str) -> Vec<Value> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let current = line.trim_start().starts_with('*');
            let name = line.trim_start_matches(['*', ' ']).trim().to_string();
            json!({
                "name": name,
                "current": current,
            })
        })
        .collect()
}

fn append_remote_list_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let remotes = parse_remote_list(text);
    let remote_names = unique_remote_names(&remotes);
    let remote_urls = unique_remote_urls(&remotes);
    root.insert("remotes".to_string(), json!(remotes));
    root.insert("remote_names".to_string(), json!(remote_names.clone()));
    root.insert("remote_urls".to_string(), json!(remote_urls.clone()));
    root.insert("remote_count".to_string(), json!(remotes.len()));
    field_value.insert("remotes".to_string(), json!(remote_names.clone()));
    field_value.insert("remote_names".to_string(), json!(remote_names));
    field_value.insert("remote_urls".to_string(), json!(remote_urls));
    field_value.insert("remote_count".to_string(), json!(remotes.len()));
}

fn append_show_file_at_rev_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let content = text.trim_end_matches(['\r', '\n']);
    let content_excerpt = bounded_single_line_excerpt(content, 240);
    root.insert("content_excerpt".to_string(), json!(content_excerpt));
    root.insert(
        "content_line_count".to_string(),
        json!(content.lines().count()),
    );
    root.insert("content_bytes".to_string(), json!(content.len()));
    field_value.insert("content_excerpt".to_string(), json!(content_excerpt));
    field_value.insert(
        "content_line_count".to_string(),
        json!(content.lines().count()),
    );
    field_value.insert("content_bytes".to_string(), json!(content.len()));
}

fn bounded_single_line_excerpt(text: &str, limit: usize) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(limit)
        .collect()
}

fn parse_remote_list(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let url = parts.next()?;
            let direction = parts
                .next()
                .map(|value| value.trim_matches(['(', ')']))
                .unwrap_or("");
            Some(json!({
                "name": name,
                "url": url,
                "direction": direction,
            }))
        })
        .collect()
}

fn unique_remote_names(remotes: &[Value]) -> Vec<String> {
    let mut names = Vec::new();
    for remote in remotes {
        let Some(name) = remote
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    names
}

fn unique_remote_urls(remotes: &[Value]) -> Vec<String> {
    let mut urls = Vec::new();
    for remote in remotes {
        let Some(url) = remote
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        if !urls.iter().any(|existing| existing == url) {
            urls.push(url.to_string());
        }
    }
    urls
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn non_empty_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_action(raw: &str) -> String {
    let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "branches" | "list_branches" | "all_branches" => "branch",
        "current_branch_name" | "branch_current" | "get_current_branch" => "current_branch",
        "cached_diff" | "staged_diff" => "diff_cached",
        "changed_file" | "changed_file_names" => "changed_files",
        "revparse" | "head" => "rev_parse",
        _ => normalized.as_str(),
    }
    .to_string()
}

/// 使用 `git rev-parse --is-inside-work-tree` 可靠识别仓库根、子目录与 worktree。
fn is_git_repo(root: &PathBuf) -> bool {
    let out = Command::new("git")
        .arg("-C")
        .arg(root.as_os_str())
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = out else {
        return false;
    };
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    out.status.success() && s == "true"
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
