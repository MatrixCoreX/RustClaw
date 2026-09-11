use crate::Result;
use uuid::Uuid;

pub trait KeyStore: Send + Sync {
    fn load(&self, id: Uuid) -> Result<String>;
    fn save(&self, id: Uuid, wrapped: &str) -> Result<()>;
}

pub struct NativeKeyStore;
fn entry(id: Uuid) -> Result<keyring::Entry> {
    keyring::Entry::new("agent-runtime.desktop.asset-vault.v1", &id.to_string())
        .map_err(|_| "wallet_keystore_unavailable".into())
}
impl KeyStore for NativeKeyStore {
    fn load(&self, id: Uuid) -> Result<String> {
        entry(id)?
            .get_password()
            .map_err(|_| "wallet_keystore_unavailable".into())
    }
    fn save(&self, id: Uuid, wrapped: &str) -> Result<()> {
        entry(id)?
            .set_password(wrapped)
            .map_err(|_| "wallet_keystore_unavailable".into())
    }
}
