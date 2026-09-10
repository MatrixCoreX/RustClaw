use std::path::{Path, PathBuf};

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn directory_hint(directory: &Path) -> bool {
    // Presence is only an installation hint. Never run a discovered executable.
    #[cfg(windows)]
    let names = ("agentctl.exe", "clawd.exe", "webd.exe");
    #[cfg(not(windows))]
    let names = ("agentctl", "clawd", "webd");
    executable(&directory.join(names.0))
        || (executable(&directory.join(names.1)) && executable(&directory.join(names.2)))
}

pub(super) fn installed_hint() -> bool {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .take(64)
                .collect()
        })
        .unwrap_or_default();
    #[cfg(unix)]
    {
        directories.extend(["/usr/local/bin", "/usr/bin"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME") {
            directories.push(PathBuf::from(home).join(".local/bin"));
        }
    }
    if let Ok(current) = std::env::current_dir() {
        // Cover a source checkout or an extracted release launched from its folder.
        for base in current.ancestors().take(3) {
            directories.extend([
                base.to_owned(),
                base.join("bin"),
                base.join("target/release"),
            ]);
        }
    }
    directories
        .iter()
        .any(|directory| directory_hint(directory))
}
