use super::*;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw-package-manager-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn detects_npm_project_from_package_lock() {
    let root = TempDir::new("npm");
    std::fs::write(root.path.join("package.json"), "{}").expect("write manifest");
    std::fs::write(root.path.join("package-lock.json"), "{}").expect("write lock");

    let detected = detect_project_manager(&root.path).expect("project manager");

    assert_eq!(detected.manager, "npm");
    assert_eq!(detected.marker, "package-lock.json");
}

#[test]
fn detects_cargo_project_from_manifest() {
    let root = TempDir::new("cargo");
    std::fs::write(root.path.join("Cargo.toml"), "[package]\nname=\"demo\"\n")
        .expect("write cargo manifest");

    let detected = detect_project_manager(&root.path).expect("project manager");

    assert_eq!(detected.manager, "cargo");
    assert_eq!(detected.marker, "Cargo.toml");
}
