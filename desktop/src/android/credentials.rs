//! Password-wrapped vault material and saved logins are additionally encrypted
//! by an Android Keystore AES-GCM key. There is no plaintext fallback.
use super::bridge;
#[derive(Debug)]
pub enum Error {
    NoEntry,
    Unavailable,
}
pub struct Entry {
    service: String,
    id: String,
}
impl Entry {
    pub fn new(service: &str, id: &str) -> std::result::Result<Self, Error> {
        if service.is_empty() || id.is_empty() || service.len() > 100 || id.len() > 100
            || service.contains('\0') || id.contains('\0')
        {
            return Err(Error::Unavailable);
        }
        Ok(Self {
            service: service.into(),
            id: id.into(),
        })
    }
    pub fn get_password(&self) -> std::result::Result<String, Error> {
        bridge::string("credentialGet", &[&self.service, &self.id])
            .map_err(|_| Error::Unavailable)?
            .ok_or(Error::NoEntry)
    }
    pub fn set_password(&self, value: &str) -> std::result::Result<(), Error> {
        bridge::string("credentialPut", &[&self.service, &self.id, value])
            .map_err(|_| Error::Unavailable)
            .map(|_| ())
    }
    pub fn delete_credential(&self) -> std::result::Result<(), Error> {
        bridge::string("credentialDelete", &[&self.service, &self.id])
            .map_err(|_| Error::Unavailable)
            .map(|_| ())
    }
}
