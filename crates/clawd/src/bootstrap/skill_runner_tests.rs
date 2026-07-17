use super::*;
use std::fs;

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_skill_runner_{name}_{}_{}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp root");
        Self { path }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn installed_companion_is_independent_of_user_workspace() {
    let root = TempRoot::new("installed");
    let install_dir = root.path.join("install");
    let workspace = root.path.join("workspace");
    fs::create_dir_all(&install_dir).expect("create install dir");
    fs::create_dir_all(&workspace).expect("create workspace");
    let clawd = install_dir.join("clawd");
    let runner = install_dir.join("skill-runner");
    fs::write(&clawd, "clawd").expect("write clawd");
    fs::write(&runner, "runner").expect("write runner");

    assert_eq!(
        resolve_skill_runner_path_from(&workspace, None, Some(&clawd)),
        runner
    );
}

#[test]
fn explicit_path_supports_absolute_and_workspace_relative_values() {
    let root = TempRoot::new("explicit");
    let absolute = root.path.join("installed/skill-runner");

    assert_eq!(
        resolve_skill_runner_path_from(&root.path, Some("tools/skill-runner"), None),
        root.path.join("tools/skill-runner")
    );
    assert_eq!(
        resolve_skill_runner_path_from(
            &root.path,
            Some(absolute.to_str().expect("absolute path")),
            None,
        ),
        absolute
    );
}

#[test]
fn missing_companion_falls_back_to_workspace_release_runner() {
    let root = TempRoot::new("fallback");
    let clawd = root.path.join("install/clawd");
    let workspace = root.path.join("workspace");
    let runner = workspace.join("target/release/skill-runner");
    fs::create_dir_all(clawd.parent().expect("clawd parent")).expect("create install dir");
    fs::create_dir_all(runner.parent().expect("runner parent")).expect("create runner dir");
    fs::write(&clawd, "clawd").expect("write clawd");
    fs::write(&runner, "runner").expect("write runner");

    assert_eq!(
        resolve_skill_runner_path_from(&workspace, None, Some(&clawd)),
        runner
    );
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
