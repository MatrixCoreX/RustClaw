use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

// Profile-string encoding only, not path admission or a permission grant.
#[cfg(any(target_os = "macos", test))]
pub fn seatbelt_path_literal(path: &Path) -> Result<String, &'static str> {
    let value = path.to_str().ok_or("sandbox_path_invalid")?;
    if value
        .chars()
        .any(|value| matches!(value, '\0' | '\n' | '\r'))
    {
        return Err("sandbox_path_invalid");
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

// Filesystem-only policy. This does not fence inherited descriptors, process
// inspection, a privileged host administrator or another sandbox's namespace.
pub struct DirectoryExclusions {
    roots: Vec<PathBuf>,
}

impl DirectoryExclusions {
    pub fn admit(requested: &[PathBuf], execution_root: &Path) -> Result<Self, &'static str> {
        if requested.len() > 32 {
            return Err("sandbox_private_directories_limit");
        }
        if requested.is_empty() {
            return Ok(Self { roots: Vec::new() });
        }
        let execution_root = execution_root
            .canonicalize()
            .map_err(|_| "sandbox_directory_unavailable")?;
        let mut roots = Vec::new();
        for path in requested {
            if !path.is_absolute() {
                return Err("sandbox_private_directory_invalid");
            }
            let metadata =
                fs::symlink_metadata(path).map_err(|_| "sandbox_private_directory_unavailable")?;
            if !metadata.is_dir()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o7777 != 0o700
            {
                return Err("sandbox_private_directory_invalid");
            }
            let canonical = path
                .canonicalize()
                .map_err(|_| "sandbox_private_directory_unavailable")?;
            if canonical.parent().is_none() || execution_root.starts_with(&canonical) {
                return Err("sandbox_execution_root_private");
            }
            roots.push(canonical);
        }
        roots.sort();
        let mut admitted: Vec<PathBuf> = Vec::new();
        for root in roots {
            if !admitted.iter().any(|parent| root.starts_with(parent)) {
                admitted.push(root);
            }
        }
        Ok(Self { roots: admitted })
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn check_writable(&self, source: &Path) -> Result<(), &'static str> {
        if self.roots.iter().any(|root| source.starts_with(root)) {
            return Err("sandbox_private_directory_writable");
        }
        Ok(())
    }

    // Apply after all workspace/additional binds, including an ancestor
    // rebound at a different destination. A direct bind of private data fails.
    #[cfg(target_os = "linux")]
    pub fn mount_targets(
        &self,
        mappings: &[(PathBuf, PathBuf)],
    ) -> Result<Vec<PathBuf>, &'static str> {
        let mut targets = self.roots.clone();
        for (source, destination) in mappings {
            self.check_writable(source)?;
            for root in &self.roots {
                if destination.starts_with(root) {
                    return Err("sandbox_private_mount_conflict");
                }
                if let Ok(relative) = root.strip_prefix(source) {
                    let target = destination.join(relative);
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
            }
        }
        Ok(targets)
    }

    // A mask follows its mountpoint when an ancestor is moved, but a later
    // invocation would mask the replacement pathname instead. Anchor every
    // writable ancestor in each visible mapping before installing the masks.
    #[cfg(target_os = "linux")]
    pub fn mount_anchors(
        &self,
        writable_mappings: &[(PathBuf, PathBuf)],
    ) -> Result<Vec<(PathBuf, PathBuf)>, &'static str> {
        if self.roots.is_empty() {
            return Ok(Vec::new());
        }
        if writable_mappings.len() > 128 {
            return Err("sandbox_private_mount_limit");
        }
        self.mount_targets(writable_mappings)?;
        let mut anchors = Vec::new();
        let mut bytes = 0_usize;
        for (source, destination) in writable_mappings {
            if !source.is_absolute() || !destination.is_absolute() {
                return Err("sandbox_private_mount_invalid");
            }
            for root in &self.roots {
                let Some(relative) = root.strip_prefix(source).ok().and_then(Path::parent) else {
                    continue;
                };
                let mut source_parent = source.clone();
                let mut target_parent = destination.clone();
                for component in relative.components() {
                    let std::path::Component::Normal(component) = component else {
                        return Err("sandbox_private_mount_invalid");
                    };
                    source_parent.push(component);
                    target_parent.push(component);
                    let pair = (source_parent.clone(), target_parent.clone());
                    if !anchors.contains(&pair) {
                        bytes = bytes
                            .saturating_add(pair.0.as_os_str().len())
                            .saturating_add(pair.1.as_os_str().len());
                        if anchors.len() == 4096 || bytes > 1024 * 1024 {
                            return Err("sandbox_private_mount_limit");
                        }
                        anchors.push(pair);
                    }
                }
            }
        }
        Ok(anchors)
    }

    #[cfg(any(target_os = "macos", test))]
    pub fn seatbelt_rules(&self) -> Result<String, &'static str> {
        let mut rules = String::new();
        for root in &self.roots {
            let path =
                seatbelt_path_literal(root).map_err(|_| "sandbox_private_directory_invalid")?;
            rules.push_str(&format!("(deny file-read* (subpath \"{path}\"))\n"));
            rules.push_str(&format!("(deny file-write* (subpath \"{path}\"))\n"));
        }
        Ok(rules)
    }
}

#[cfg(test)]
#[path = "sandbox_private_tests.rs"]
mod tests;
