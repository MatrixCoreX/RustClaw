use std::fs;

use serde_json::json;

use super::*;

struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("rustclaw-code-index-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(path.join("src")).expect("create temp source");
        fs::write(
            path.join("src/lib.rs"),
            r#"
pub struct Worker;

impl Worker {
    pub fn execute(&self) -> usize {
        helper()
    }
}

fn helper() -> usize {
    7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_executes() {
        assert_eq!(Worker.execute(), helper());
    }
}
"#,
        )
        .expect("write source");
        Self { path }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn execute_json(repo: &TempRepo, args: Value) -> Value {
    let text = execute(&repo.path, &args).expect("execute code index");
    serde_json::from_str(&text).expect("machine json")
}

#[test]
fn refresh_is_incremental_and_indexes_rust_symbols_references_and_tests() {
    let repo = TempRepo::new();
    let first = execute_json(&repo, json!({"action": "refresh"}));
    assert_eq!(first["summary"]["file_count"], 1);
    assert!(first["summary"]["symbol_count"].as_u64().unwrap_or(0) >= 4);
    assert_eq!(first["summary"]["test_count"], 1);
    assert_eq!(first["summary"]["parsed_files"], 1);

    let second = execute_json(&repo, json!({"action": "refresh"}));
    assert_eq!(second["summary"]["reused_files"], 1);
    assert_eq!(second["summary"]["parsed_files"], 0);
    assert!(repo
        .path
        .join(".rustclaw/index/repository-v1.json")
        .is_file());
}

#[test]
fn definitions_and_references_return_machine_range_handles() {
    let repo = TempRepo::new();
    let definitions = execute_json(
        &repo,
        json!({"action": "find_definitions", "symbol": "helper"}),
    );
    assert_eq!(definitions["data"]["definitions"][0]["name"], "helper");
    assert_eq!(
        definitions["data"]["definitions"][0]["range_handle"]["read_capability"],
        "filesystem.read_text_range"
    );

    let references = execute_json(
        &repo,
        json!({"action": "find_references", "symbol": "helper"}),
    );
    assert!(
        references["data"]["references"]
            .as_array()
            .is_some_and(|items| items.len() >= 2),
        "{references}"
    );
}

#[test]
fn retrieve_context_uses_structured_symbols_and_bounded_source_ranges() {
    let repo = TempRepo::new();
    let result = execute_json(
        &repo,
        json!({
            "action": "retrieve_context",
            "symbols": ["execute"],
            "context_lines": 1
        }),
    );
    let snippet = &result["data"]["snippets"][0];
    assert_eq!(snippet["symbol"], "execute");
    assert!(snippet["excerpt"]
        .as_str()
        .is_some_and(|excerpt| excerpt.contains("pub fn execute")));
    assert_eq!(
        snippet["range_handle"]["read_capability"],
        "filesystem.read_text_range"
    );
}

#[test]
fn changed_impact_connects_changed_definitions_to_dependent_test_files() {
    let repo = TempRepo::new();
    fs::create_dir_all(repo.path.join("tests")).expect("create tests");
    fs::write(
        repo.path.join("tests/integration.rs"),
        r#"
use demo::Worker;

#[test]
fn integration_worker() {
    let _ = Worker.execute();
}
"#,
    )
    .expect("write integration test");
    let result = execute_json(
        &repo,
        json!({
            "action": "changed_impact",
            "paths": ["src/lib.rs"]
        }),
    );
    assert!(result["data"]["dependent_files"]
        .as_array()
        .is_some_and(|paths| paths.iter().any(|path| path == "tests/integration.rs")));
    assert!(result["data"]["impacted_tests"]
        .as_array()
        .is_some_and(|tests| tests
            .iter()
            .any(|test| test["name"] == "integration_worker")));
}

#[test]
fn workspace_traversal_is_rejected_as_machine_error() {
    let repo = TempRepo::new();
    let error = execute(
        &repo.path,
        &json!({"action": "retrieve_context", "paths": ["../outside.rs"]}),
    )
    .expect_err("traversal must fail");
    assert_eq!(error.code, "path_outside_workspace");
}
