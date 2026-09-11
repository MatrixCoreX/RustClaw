use crate::Result;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

pub fn private_directory(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("wallet_storage_invalid".into());
    }
    fs::create_dir_all(path).map_err(|_| "wallet_storage_unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "wallet_storage_unavailable")?;
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err("wallet_storage_invalid".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "wallet_storage_unavailable")?
        .take(1_000_001)
        .read_to_end(&mut bytes)
        .map_err(|_| "wallet_storage_unavailable")?;
    if bytes.len() > 1_000_000 {
        return Err("wallet_data_invalid".into());
    }
    Ok(bytes)
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
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
