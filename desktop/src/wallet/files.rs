use crate::Result;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const MAX_FILE_BYTES: usize = 1_000_000;

fn options(create: bool, private: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(create)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Inspect the opened handle, not a path that could change after lstat.
        // Nonblocking also prevents a substituted FIFO from hanging the vault.
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        if private {
            options.access_mode(
                0x80000000 | (if create { 0x40000000 } else { 0 }) | 0x00020000 | 0x00040000,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = private;
    options
}

fn ordinary(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }
    !metadata.file_type().is_symlink()
}

fn secure_file(file: &File, private: bool) -> Result<()> {
    let metadata = file.metadata().map_err(|_| "wallet_storage_unavailable")?;
    if !metadata.is_file() || !ordinary(&metadata) {
        return Err("wallet_storage_invalid".into());
    }
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid has no arguments and does not modify process state.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Err("wallet_storage_invalid".into());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "wallet_storage_unavailable")?;
    }
    #[cfg(windows)]
    if private {
        super::windows_security::private_handle(file, false)?;
    }
    Ok(())
}

fn open_file(path: &Path, create: bool, private: bool) -> Result<File> {
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || !ordinary(&m)) {
        return Err("wallet_storage_invalid".into());
    }
    let file = options(create, private)
        .open(path)
        .map_err(|_| "wallet_storage_unavailable")?;
    secure_file(&file, private)?;
    Ok(file)
}

pub fn private_directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir() || !ordinary(&m)) {
        return Err("wallet_storage_invalid".into());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| "wallet_storage_unavailable")?;
    let mut open = OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        };
        open.access_mode(FILE_READ_ATTRIBUTES | 0x00020000 | 0x00040000)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let directory = open.open(path).map_err(|_| "wallet_storage_unavailable")?;
    let metadata = directory
        .metadata()
        .map_err(|_| "wallet_storage_unavailable")?;
    if !metadata.is_dir() || !ordinary(&metadata) {
        return Err("wallet_storage_invalid".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid only reads the process's effective user identifier.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err("wallet_storage_invalid".into());
        }
        directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|_| "wallet_storage_unavailable")?;
    }
    #[cfg(windows)]
    super::windows_security::private_handle(&directory, true)?;
    Ok(())
}

pub fn private_lock(path: &Path) -> Result<File> {
    let lock = open_file(path, true, true)?;
    lock.try_lock().map_err(|_| "wallet_already_open")?;
    Ok(lock)
}

fn read_file(path: &Path, private: bool) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_file(path, false, private)?
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "wallet_storage_unavailable")?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("wallet_data_invalid".into());
    }
    Ok(bytes)
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    read_file(path, false)
}
pub fn read_private(path: &Path) -> Result<Vec<u8>> {
    read_file(path, true)
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("wallet_data_invalid".into());
    }
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || !ordinary(&m)) {
        return Err("wallet_storage_invalid".into());
    }
    let parent = path.parent().ok_or("wallet_storage_invalid")?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "wallet_storage_unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "wallet_storage_unavailable")?;
    }
    #[cfg(windows)]
    super::windows_security::private_handle(file.as_file(), false)?;
    file.write_all(bytes)
        .map_err(|_| "wallet_storage_unavailable")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "wallet_storage_unavailable")?;
    file.persist(path)
        .map_err(|_| "wallet_storage_unavailable")?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| "wallet_storage_unavailable")?;
    Ok(())
}
