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
/// Only display metadata crosses IPC; saved passwords and keys stay native.
#[derive(Serialize)]
pub struct LoginPrefill {
    pub mode: String,
    pub username: String,
}
impl LoginSecret {
    fn prefill(&self) -> Result<LoginPrefill> {
        if self.secret.is_empty()
            || !matches!(self.mode.as_str(), "password" | "key")
            || (self.mode == "password" && self.username.is_empty())
        {
            return Err("credential_invalid".into());
        }
        Ok(LoginPrefill {
            mode: self.mode.clone(),
            username: if self.mode == "password" {
                self.username.clone()
            } else {
                String::new()
            },
        })
    }
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
    load_optional(profile)?.ok_or("credential_store_unavailable".into())
}
pub fn prefill(profile: Uuid) -> Result<Option<LoginPrefill>> {
    load_optional(profile)?
        .map(|input| input.prefill())
        .transpose()
}
fn load_optional(profile: Uuid) -> Result<Option<LoginSecret>> {
    let mut value = match entry(profile)?.get_password() {
        Ok(value) => value,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(_) => return Err("credential_store_unavailable".into()),
    };
    let result = serde_json::from_str(&value).map_err(|_| "credential_invalid".into());
    value.zeroize();
    result.map(Some)
}

#[cfg(test)]
#[path = "credentials_tests.rs"]
mod tests;
pub fn forget(profile: Uuid) -> Result<()> {
    match entry(profile)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("credential_store_locked".into()),
    }
}
