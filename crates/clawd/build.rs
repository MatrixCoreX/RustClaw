use std::{path::Path, process::Command};
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
fn watch_git_path(name: &str) {
    if let Some(path) =
        git(&["rev-parse", "--git-path", name]).filter(|path| Path::new(path).is_file())
    {
        // Watching a missing packed-refs file makes Cargo rebuild on every invocation.
        println!("cargo:rerun-if-changed={path}");
    }
}
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    watch_git_path("HEAD");
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(&reference);
    }
    watch_git_path("packed-refs");
    let revision = git(&["rev-parse", "--verify", "HEAD"])
        .filter(|v| v.len() == 40 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_default();
    println!("cargo:rustc-env=APP_BUILD_REVISION={revision}");
}
