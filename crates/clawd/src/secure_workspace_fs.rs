use std::fs::File;
use std::io::{self, Write};
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
fn open_directory(path: &Path, access: libc::c_int) -> io::Result<OwnedFd> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| invalid_path("workspace_path_nul_forbidden"))?;
    // SAFETY: `path` is a valid NUL-terminated path and the returned descriptor
    // is immediately transferred into OwnedFd.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            access | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
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
    let directory = open_directory(&workspace_root.canonicalize()?, libc::O_RDONLY)?;
    walk_parent_directory(
        directory,
        components,
        create_missing,
        libc::O_RDONLY,
        &mut |_| Ok(()),
    )
}

#[cfg(unix)]
fn walk_parent_directory(
    mut directory: OwnedFd,
    components: Vec<&std::ffi::OsStr>,
    create_missing: bool,
    access: libc::c_int,
    guard: &mut impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<(OwnedFd, CString)> {
    let filename = c_string(components.last().expect("nonempty components"))?;
    guard(&directory)?;
    for component in &components[..components.len() - 1] {
        let component = c_string(component)?;
        // SAFETY: the directory fd and component CString are valid for this call.
        let mut next = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                component.as_ptr(),
                access | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
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
                    access | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
        }
        if next < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `next` is a fresh descriptor returned by openat.
        directory = unsafe { OwnedFd::from_raw_fd(next) };
        guard(&directory)?;
    }
    Ok((directory, filename))
}

#[cfg(unix)]
fn directory_search_access() -> io::Result<libc::c_int> {
    #[cfg(target_os = "linux")]
    return Ok(libc::O_PATH);
    #[cfg(target_os = "macos")]
    return Ok(libc::O_SEARCH);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "workspace_directory_search_unsupported",
    ))
}

#[cfg(unix)]
fn open_absolute_directory_with_guard(
    path: &Path,
    guard: &mut impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(invalid_path("workspace_path_outside_root"));
    }
    let components = relative_components(Path::new("/"), &path.join(".directory-handle"))?
        .into_iter()
        .map(std::ffi::OsStr::to_owned)
        .collect::<Vec<_>>();
    let components = components.iter().map(|part| part.as_os_str()).collect();
    let access = directory_search_access()?;
    let (directory, _) = walk_parent_directory(
        open_directory(Path::new("/"), access)?,
        components,
        false,
        access,
        guard,
    )?;
    Ok(directory)
}

#[cfg(unix)]
pub(crate) fn open_workspace_directory_with_guard(
    workspace_root: &Path,
    target: &Path,
    mut guard: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<OwnedFd> {
    let root = open_absolute_directory_with_guard(&workspace_root.canonicalize()?, &mut guard)?;
    let handle_path = target.join(".directory-handle");
    let components = relative_components(workspace_root, &handle_path)?;
    let (directory, _) = walk_parent_directory(
        root,
        components,
        false,
        directory_search_access()?,
        &mut guard,
    )?;
    Ok(directory)
}

#[cfg(unix)]
pub(crate) fn ensure_private_workspace_directory(
    workspace_root: &Path,
    target: &Path,
) -> io::Result<()> {
    let (directory, _) =
        open_parent_directory(workspace_root, &target.join(".directory-handle"), true)?;
    let directory = File::from(directory);
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid takes no arguments and does not access caller memory.
    if directory.metadata()?.uid() != unsafe { libc::geteuid() } {
        return Err(invalid_path("workspace_private_directory_not_owned"));
    }
    directory.set_permissions(std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub(crate) fn ensure_private_workspace_directory(
    _workspace_root: &Path,
    _target: &Path,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "workspace_private_directory_unsupported",
    ))
}

#[cfg(unix)]
pub(crate) fn open_workspace_file(workspace_root: &Path, target: &Path) -> io::Result<File> {
    let (parent, filename) = open_parent_directory(workspace_root, target, false)?;
    open_regular_file(parent, filename)
}

#[cfg(unix)]
pub(crate) fn open_workspace_file_with_guard(
    workspace_root: &Path,
    target: &Path,
    mut guard: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<File> {
    let root = open_absolute_directory_with_guard(&workspace_root.canonicalize()?, &mut guard)?;
    let components = relative_components(workspace_root, target)?;
    let (parent, filename) = walk_parent_directory(
        root,
        components,
        false,
        directory_search_access()?,
        &mut guard,
    )?;
    open_regular_file(parent, filename)
}

#[cfg(unix)]
fn open_regular_file(parent: OwnedFd, filename: CString) -> io::Result<File> {
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

#[derive(Clone, Copy)]
#[cfg(unix)]
enum Publication {
    Replace,
    CreateNew,
}

#[cfg(unix)]
pub(crate) fn atomic_create_workspace_file(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    atomic_publish_workspace_file(workspace_root, target, bytes, true, Publication::CreateNew)
}

#[cfg(unix)]
fn sync_directory(directory: &OwnedFd) -> io::Result<()> {
    // SAFETY: the caller owns this directory descriptor throughout the call.
    if unsafe { libc::fsync(directory.as_raw_fd()) } == 0 {
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
}

#[cfg(unix)]
pub(crate) fn rename_without_replacement(
    parent: &OwnedFd,
    from: &CString,
    to: &CString,
) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    // SAFETY: both names and the shared directory descriptor remain valid.
    let renamed = unsafe {
        libc::renameat2(
            parent.as_raw_fd(),
            from.as_ptr(),
            parent.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    // SAFETY: same descriptor-relative contract as the Linux implementation.
    let renamed = unsafe {
        libc::renameatx_np(
            parent.as_raw_fd(),
            from.as_ptr(),
            parent.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    return if renamed == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (parent, from, to);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "workspace_atomic_create_unsupported",
        ))
    }
}

#[cfg(unix)]
fn atomic_publish_workspace_file(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
    create_missing_parents: bool,
    publication: Publication,
) -> io::Result<()> {
    let (parent, filename) = open_parent_directory(workspace_root, target, create_missing_parents)?;
    atomic_publish_in_directory(parent, filename, bytes, publication)
}

#[cfg(unix)]
pub(crate) fn open_workspace_parent_with_guard(
    workspace_root: &Path,
    target: &Path,
    create_missing_parents: bool,
    mut guard: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<(OwnedFd, CString)> {
    let root = open_absolute_directory_with_guard(&workspace_root.canonicalize()?, &mut guard)?;
    let components = relative_components(workspace_root, target)?;
    walk_parent_directory(
        root,
        components,
        create_missing_parents,
        directory_search_access()?,
        &mut guard,
    )
}

#[cfg(unix)]
pub(crate) fn atomic_write_workspace_file_with_options_and_guard(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
    create_missing_parents: bool,
    guard: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<()> {
    atomic_publish_workspace_file_with_guard(
        workspace_root,
        target,
        bytes,
        create_missing_parents,
        guard,
        Publication::Replace,
    )
}

#[cfg(unix)]
pub(crate) fn atomic_create_workspace_file_with_guard(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
    guard: impl FnMut(&OwnedFd) -> io::Result<()>,
) -> io::Result<()> {
    atomic_publish_workspace_file_with_guard(
        workspace_root,
        target,
        bytes,
        false,
        guard,
        Publication::CreateNew,
    )
}

#[cfg(unix)]
fn atomic_publish_workspace_file_with_guard(
    workspace_root: &Path,
    target: &Path,
    bytes: &[u8],
    create_missing_parents: bool,
    mut guard: impl FnMut(&OwnedFd) -> io::Result<()>,
    publication: Publication,
) -> io::Result<()> {
    let (parent, filename) = open_workspace_parent_with_guard(
        workspace_root,
        target,
        create_missing_parents,
        &mut guard,
    )?;
    let parent = rustix::fs::openat(
        parent,
        ".",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    guard(&parent)?;
    atomic_publish_in_directory(parent, filename, bytes, publication)
}

#[cfg(unix)]
fn atomic_publish_in_directory(
    parent: OwnedFd,
    filename: CString,
    bytes: &[u8],
    publication: Publication,
) -> io::Result<()> {
    let temporary_name = CString::new(format!(
        ".agent-write-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ))
    .expect("generated filename");
    let existing_mode = match open_regular_file(parent.try_clone()?, filename.clone()) {
        Ok(_) if matches!(publication, Publication::CreateNew) => {
            // Another publisher may not have synced its rename yet. Make reuse
            // durable before the caller verifies and publishes its reference.
            sync_directory(&parent)?;
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "workspace_file_exists",
            ));
        }
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
        // Restore permissions before writing: umask must not narrow an existing
        // mode, while content writes retain the kernel's normal set-id clearing.
        if let Some(mode) = existing_mode {
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        let renamed = match publication {
            Publication::CreateNew => {
                rename_without_replacement(&parent, &temporary_name, &filename)
            }
            Publication::Replace => {
                // SAFETY: both names are relative to the same open directory;
                // renameat replaces the link entry without following it.
                if unsafe {
                    libc::renameat(
                        parent.as_raw_fd(),
                        temporary_name.as_ptr(),
                        parent.as_raw_fd(),
                        filename.as_ptr(),
                    )
                } == 0
                {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        };
        if renamed
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::AlreadyExists)
        {
            sync_directory(&parent)?;
        }
        renamed?;
        sync_directory(&parent)
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
pub(crate) fn atomic_create_workspace_file(
    _workspace_root: &Path,
    _target: &Path,
    _bytes: &[u8],
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "workspace_atomic_create_unsupported",
    ))
}

#[cfg(test)]
#[path = "secure_workspace_fs_tests.rs"]
pub(crate) mod tests;
