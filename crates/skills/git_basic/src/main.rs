use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml::Value as TomlValue;

static I18N: OnceLock<TextCatalog> = OnceLock::new();

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
    current.insert("git_basic.err.invalid_input".to_string(), "invalid input: {error}".to_string());
    current.insert("git_basic.err.args_object".to_string(), "args must be object".to_string());
    current.insert(
        "git_basic.msg.not_git_repo".to_string(),
        "current directory is not a git repository. Please use git_basic in a git repo.".to_string(),
    );
    current.insert(
        "git_basic.err.unsupported_action".to_string(),
        "unsupported action; use status|log|diff|branch|show|rev_parse|diff_cached|current_branch|remote|changed_files|show_file_at_rev".to_string(),
    );
    current.insert("git_basic.err.run_git_failed".to_string(), "run git failed: {error}".to_string());
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
                Ok(text) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(tr_with("git_basic.err.invalid_input", &[("error", &err.to_string())])),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| tr("git_basic.err.args_object"))?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");
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

    let (subcmd, mut extra): (&str, Vec<String>) = match action {
        "status" => ("status", vec!["--short".to_string(), "--branch".to_string()]),
        "log" => (
            "log",
            vec![
                "--oneline".to_string(),
                "-n".to_string(),
                log_n.to_string(),
            ],
        ),
        "diff" => ("diff", vec![]),
        "diff_cached" => ("diff", vec!["--cached".to_string()]),
        "branch" => ("branch", vec!["--all".to_string()]),
        "current_branch" => ("rev-parse", vec!["--abbrev-ref".to_string(), "HEAD".to_string()]),
        "remote" => ("remote", vec!["-v".to_string()]),
        "changed_files" => (
            "diff",
            vec!["--name-only".to_string(), "HEAD".to_string()],
        ),
        "show" => {
            let target = obj
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("HEAD");
            ("show", vec!["--stat".to_string(), target.to_string()])
        }
        "show_file_at_rev" => {
            let target = obj
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("HEAD");
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return Err("show_file_at_rev requires path".to_string());
            }
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

    let out = cmd
        .output()
        .map_err(|err| tr_with("git_basic.err.run_git_failed", &[("error", &err.to_string())]))?;

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

    Ok(format!("exit={}\n{}", out.status.code().unwrap_or(-1), text))
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
