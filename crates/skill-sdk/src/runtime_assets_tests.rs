use super::*;
use tempfile::tempdir;

fn manifest(files: Vec<RuntimeFile>) -> PackageManifest {
    let mut manifest = PackageManifest::from_toml_str(crate::tests::manifest_source()).unwrap();
    manifest.build.runtime_files = files;
    manifest
}

fn asset(source: &str, destination: &str) -> RuntimeFile {
    RuntimeFile {
        source: source.into(),
        destination: destination.into(),
    }
}

#[test]
fn runtime_assets_survive_source_removal_and_bind_every_dependency_digest() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    fs::create_dir_all(source.join("deps/module")).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(source.join("helper.js"), "module.exports = 42;").unwrap();
    fs::write(source.join("deps/module/index.js"), "module.exports = 1;").unwrap();
    let spec = manifest(vec![
        asset("helper.js", "runtime/assets/helper.js"),
        asset("deps", "runtime/assets/node_modules"),
    ]);
    let mut artifacts = Vec::new();
    install(&spec, &source, &staging, &mut artifacts).unwrap();
    assert_eq!(artifacts.len(), 2);
    fs::remove_dir_all(&source).unwrap();
    for artifact in &artifacts {
        let path = staging.join(&artifact.path);
        assert_eq!(digest_file(&path).unwrap(), artifact.sha256);
        assert_eq!(fs::metadata(path).unwrap().len(), artifact.size_bytes);
    }
    let artifact = &artifacts[0];
    fs::write(staging.join(&artifact.path), "tampered").unwrap();
    assert_ne!(
        digest_file(&staging.join(&artifact.path)).unwrap(),
        artifact.sha256
    );
}

#[test]
fn runtime_assets_reject_escape_collision_missing_and_overlapping_roots() {
    for entry in [
        asset("../secret", "runtime/assets/a"),
        asset("a", "runtime/assets/../../bin/a"),
        asset("a", "runtime/bin/a"),
    ] {
        assert!(validate(&[entry]).is_err());
    }
    assert!(validate(&[
        asset("a", "runtime/assets/a"),
        asset("b", "runtime/assets/a/b")
    ])
    .is_err());
    let temp = tempdir().unwrap();
    let spec = manifest(vec![asset("missing", "runtime/assets/a")]);
    assert_eq!(
        install(&spec, temp.path(), temp.path(), &mut Vec::new())
            .unwrap_err()
            .code,
        "runtime_asset_unavailable"
    );
    fs::write(temp.path().join("missing"), "source").unwrap();
    fs::create_dir_all(temp.path().join("runtime/assets")).unwrap();
    fs::write(temp.path().join("runtime/assets/a"), "keep").unwrap();
    assert_eq!(
        install(&spec, temp.path(), temp.path(), &mut Vec::new())
            .unwrap_err()
            .code,
        "runtime_asset_collision"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("runtime/assets/a")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn runtime_assets_reject_source_and_destination_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = tempdir().unwrap();
    fs::create_dir(temp.path().join("outside")).unwrap();
    fs::write(temp.path().join("outside/secret"), "secret").unwrap();
    symlink(temp.path().join("outside"), temp.path().join("link")).unwrap();
    let spec = manifest(vec![asset("link/secret", "runtime/assets/a")]);
    assert_eq!(
        install(&spec, temp.path(), temp.path(), &mut Vec::new())
            .unwrap_err()
            .code,
        "runtime_asset_symlink_forbidden"
    );
    fs::create_dir(temp.path().join("runtime")).unwrap();
    symlink(
        temp.path().join("outside"),
        temp.path().join("runtime/assets"),
    )
    .unwrap();
    let spec = manifest(vec![asset("outside/secret", "runtime/assets/a")]);
    assert_eq!(
        install(&spec, temp.path(), temp.path(), &mut Vec::new())
            .unwrap_err()
            .code,
        "runtime_asset_symlink_forbidden"
    );
    assert!(!temp.path().join("outside/a").exists());
}

#[test]
fn runtime_assets_are_part_of_the_manifest_identity() {
    let plain = manifest(Vec::new());
    let packaged = manifest(vec![asset("helper.js", "runtime/assets/helper.js")]);
    assert_ne!(plain.digest().unwrap(), packaged.digest().unwrap());
    let encoded = packaged.to_toml_string().unwrap();
    let parsed = PackageManifest::from_toml_str(&encoded).unwrap();
    assert_eq!(packaged.build.runtime_files, parsed.build.runtime_files);
}

#[test]
fn runtime_assets_reject_oversized_files_before_copying() {
    let temp = tempdir().unwrap();
    let input = fs::File::create(temp.path().join("large")).unwrap();
    input.set_len(MAX_BYTES + 1).unwrap();
    let spec = manifest(vec![asset("large", "runtime/assets/large")]);
    assert_eq!(
        install(&spec, temp.path(), temp.path(), &mut Vec::new())
            .unwrap_err()
            .code,
        "runtime_assets_limit"
    );
    assert!(!temp.path().join("runtime/assets/large").exists());
}
