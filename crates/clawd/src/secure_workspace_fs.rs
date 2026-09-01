use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn invalid_path(code: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, code)
}

fn relative_components<'a>(
    workspace_root: &Path,
    target: &'a Path,
) -> io::Result<Vec<&'a std::ffi::OsStr>> {
    let root = workspace_root.canonicalize()?;
    let relative = target
        .strip_prefix(&root)
        .or_else(|_| target.strip_prefix(workspace_root))
        .map_err(|_| invalid_path("workspace_path_outside_root"))?;
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => components.push(value),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(invalid_path("workspace_path_component_forbidden"));
            }
        }
    }
    if components.is_empty() {
        return Err(invalid_path("workspace_file_path_missing"));
    }
    Ok(components)
}

#[cfg(unix)]
fn c_string(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| invalid_path("workspace_path_nul_forbidden"))
}

#[cfg(unix)]
fn open_directory(path: &Path) -> io::Result<OwnedFd> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| invalid_path("workspace_path_nul_forbidden"))?;
    // SAFETY: `path` is a valid NUL-terminated path and the returned descriptor
    // is immediately transferred into OwnedFd.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was returned by open and is uniquely owned here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_parent_directory(
    workspace_root: &Path,
    target: &Path,
    create_missing: bool,
) -> io::Result<(OwnedFd, CString)> {
    let components = relative_components(workspace_root, target)?;
    let filename = c_string(components.last().expect("nonempty components"))?;
    let mut directory = open_directory(&workspace_root.canonicalize()?)?;
    for component in &components[..components.len() - 1] {
        let component = c_string(component)?;
        // SAFETY: the directory fd and component CString are valid for this call.
        let mut next = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if next < 0
            && create_missing
            && io::Error::last_os_error().kind() == io::ErrorKind::NotFound
        {
            // SAFETY: mkdirat receives a valid directory fd and component name.
            let created =
                unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o700) };
            if created != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: same validated descriptor and component as above.
            next = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    component.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
        }
        if next < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `next` is a fresh descriptor returned by openat.
        directory = unsafe { OwnedFd::from_raw_fd(next) };
    }
    Ok((directory, filename))
}

#[cfg(unix)]
pub(crate) fn open_workspace_file(workspace_root: &Path, target: &Path) -> io::Result<File> {
    let (parent, filename) = open_parent_directory(workspace_root, target, false)?;
    // SAFETY: parent and filename are validated and remain alive for the call.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            filename.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is uniquely owned and transferred to File.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(invalid_path("workspace_path_not_regular_file"));
    }
    Ok(file)
}

#[cfg(not(unix))]
pub(crate) fn open_workspace_file(workspace_root: &Path, target: &Path) -> io::Result<File> {
    let _ = relative_components(workspace_root, target)?;
    let metadata = std::fs::symlink_metadata(target)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_path("workspace_path_not_regular_file"));
    }
    File::open(target)
}

#[cfg(unix)]
pub(crate) fn atomic_write_workspace_file(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    atomic_write_workspace_file_with_options(workspace_root, target, bytes, true)
}

#[cfg(unix)]
pub(crate) fn atomic_write_workspace_file_with_options(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
    create_missing_parents: bool,
) -> io::Result<()> {
    let (parent, filename) = open_parent_directory(workspace_root, target, create_missing_parents)?;
    let temporary_name = CString::new(format!(
        ".agent-write-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ))
    .expect("generated filename");
    let existing_mode = match open_workspace_file(workspace_root, target) {
        Ok(file) => Some(file.metadata()?.permissions().mode() & 0o7777),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    // SAFETY: parent and temporary_name are valid. O_EXCL and O_NOFOLLOW bind
    // creation to this exact directory descriptor without following links.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temporary_name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            existing_mode.unwrap_or(0o600) as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is uniquely owned and transferred to File.
    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        // SAFETY: both names are relative to the same open directory. renameat
        // replaces a link entry itself and never follows the target symlink.
        let renamed = unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                parent.as_raw_fd(),
                filename.as_ptr(),
            )
        };
        if renamed != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `parent` remains open and owned for the duration of this call.
        let synced = unsafe { libc::fsync(parent.as_raw_fd()) };
        if synced == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::EINVAL) | Some(libc::ENOTSUP)
        ) {
            Ok(())
        } else {
            Err(error)
        }
    })();
    if result.is_err() {
        // SAFETY: unlinkat receives the same trusted directory and generated name.
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
        }
    }
    result
}

#[cfg(not(unix))]
pub(crate) fn atomic_write_workspace_file(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    atomic_write_workspace_file_with_options(workspace_root, target, bytes, true)
}

#[cfg(not(unix))]
pub(crate) fn atomic_write_workspace_file_with_options(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
    create_missing_parents: bool,
) -> io::Result<()> {
    let _ = relative_components(workspace_root, target)?;
    if target
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(invalid_path("workspace_path_symlink_forbidden"));
    }
    let parent = target
        .parent()
        .ok_or_else(|| invalid_path("workspace_parent_missing"))?;
    if create_missing_parents {
        std::fs::create_dir_all(parent)?;
    } else if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "workspace_parent_missing",
        ));
    }
    let temporary = parent.join(format!(
        ".agent-write-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

pub(crate) fn read_workspace_file(workspace_root: &Path, target: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_workspace_file(workspace_root, target)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(unix)]
pub(crate) fn remove_workspace_file(workspace_root: &Path, target: &Path) -> io::Result<()> {
    let (parent, filename) = open_parent_directory(workspace_root, target, false)?;
    let file = open_workspace_file(workspace_root, target)?;
    if !file.metadata()?.is_file() {
        return Err(invalid_path("workspace_path_not_regular_file"));
    }
    // SAFETY: the target name is relative to a trusted directory descriptor.
    let removed = unsafe { libc::unlinkat(parent.as_raw_fd(), filename.as_ptr(), 0) };
    if removed != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn remove_workspace_file(workspace_root: &Path, target: &Path) -> io::Result<()> {
    let _ = relative_components(workspace_root, target)?;
    let metadata = std::fs::symlink_metadata(target)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_path("workspace_path_not_regular_file"));
    }
    std::fs::remove_file(target)
}

#[cfg(test)]
#[path = "secure_workspace_fs_tests.rs"]
mod tests;
