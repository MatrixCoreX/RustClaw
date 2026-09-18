use crate::Result;
use std::path::{Path, PathBuf};
pub struct Selection(PathBuf);
impl Selection {
    pub fn path(&self) -> &Path {
        &self.0
    }
}
pub async fn save(title: &str, name: &str) -> Result<Option<Selection>> {
    #[cfg(not(target_os = "android"))]
    {
        Ok(rfd::AsyncFileDialog::new()
            .set_title(title)
            .set_file_name(name)
            .save_file()
            .await
            .map(|f| Selection(f.path().into())))
    }
    #[cfg(target_os = "android")]
    {
        let _ = title;
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            crate::android::bridge::string("saveDocument", &[&name])
                .map(|p| p.map(|p| Selection(p.into())))
        })
        .await
        .map_err(|_| "android_operation_failed")?
    }
}
pub async fn pick(title: &str) -> Result<Option<Selection>> {
    #[cfg(not(target_os = "android"))]
    {
        Ok(rfd::AsyncFileDialog::new()
            .set_title(title)
            .add_filter("Encrypted account backup", &["json"])
            .pick_file()
            .await
            .map(|f| Selection(f.path().into())))
    }
    #[cfg(target_os = "android")]
    {
        let _ = title;
        tokio::task::spawn_blocking(|| {
            crate::android::bridge::string("pickBackup", &[])
                .map(|p| p.map(|p| Selection(p.into())))
        })
        .await
        .map_err(|_| "android_operation_failed")?
    }
}
