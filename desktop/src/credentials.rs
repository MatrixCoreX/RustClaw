use crate::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct LoginSecret {
    pub mode: String,
    pub username: String,
    pub secret: String,
}
// The reference is bound to an immutable profile (address + trust + transport).
fn entry(profile: Uuid) -> Result<keyring::Entry> {
    keyring::Entry::new("agent-runtime.desktop.v1", &profile.to_string())
        .map_err(|_| "credential_store_unavailable".into())
}
pub fn save(profile: Uuid, secret: &LoginSecret) -> Result<()> {
    let mut value = serde_json::to_string(secret).map_err(|_| "credential_invalid")?;
    let result = entry(profile)?
        .set_password(&value)
        .map_err(|_| "credential_store_locked".into());
    value.zeroize();
    result
}
pub fn load(profile: Uuid) -> Result<LoginSecret> {
    let mut value = entry(profile)?
        .get_password()
        .map_err(|_| "credential_store_unavailable")?;
    let result = serde_json::from_str(&value).map_err(|_| "credential_invalid".into());
    value.zeroize();
    result
}
pub fn forget(profile: Uuid) -> Result<()> {
    match entry(profile)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("credential_store_locked".into()),
    }
}
