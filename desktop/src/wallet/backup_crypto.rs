//! Portable backup encryption is versioned separately from the local vault.
use super::{crypto, keys, secure_memory::LockedKey};
use crate::Result;
use serde::{Deserialize, Serialize};

const FORMAT: &str = "asset-account-backup-v2";
const CIPHER: &str = "xchacha20-poly1305";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Kdf {
    algorithm: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    output_bytes: u32,
    salt: String,
}
impl Kdf {
    fn validate(&self) -> Result<()> {
        // Only reviewed profiles are accepted. Untrusted files cannot request
        // unbounded work or silently downgrade the encryption profile.
        if self.algorithm != "argon2id"
            || self.version != 19
            || self.memory_kib != 262_144
            || self.iterations != 3
            || self.parallelism != 4
            || self.output_bytes != 32
            || self.salt.len() != 32
        {
            return Err("wallet_backup_parameters_unsupported".into());
        }
        Ok(())
    }
    fn derive(&self, password: &str) -> Result<LockedKey> {
        self.validate()?;
        let salt = hex::decode(&self.salt).map_err(|_| "wallet_backup_invalid")?;
        crypto::derive_with(
            password,
            &salt,
            self.memory_kib,
            self.iterations,
            self.parallelism,
        )
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    format: String,
    public_key: String,
    cipher: String,
    kdf: Kdf,
    encrypted: crypto::Sealed,
}
impl Backup {
    fn aad(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&(&self.format, &self.public_key, &self.cipher, &self.kdf))
            .map_err(|_| "wallet_backup_invalid".into())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyBackup {
    format: String,
    public_key: String,
    encrypted: crypto::Wrapped,
}
#[derive(Deserialize)]
struct Header {
    format: String,
}

pub fn password_valid(password: &str, user_inputs: &[&str]) -> Result<()> {
    let len = password.chars().count();
    if !(16..=128).contains(&len) || password.len() > 512 || password.chars().any(char::is_control)
    {
        return Err("wallet_backup_password_length".into());
    }
    if zxcvbn::zxcvbn(password, user_inputs).score() < zxcvbn::Score::Four {
        return Err("wallet_backup_password_weak".into());
    }
    Ok(())
}
pub fn seal(password: &str, public_key: &str, secret: &LockedKey) -> Result<Vec<u8>> {
    keys::validate_public(public_key)?;
    let mut backup = Backup {
        format: FORMAT.into(),
        public_key: public_key.into(),
        cipher: CIPHER.into(),
        kdf: Kdf {
            algorithm: "argon2id".into(),
            version: 19,
            memory_kib: 262_144,
            iterations: 3,
            parallelism: 4,
            output_bytes: 32,
            salt: hex::encode(crypto::random::<16>()?),
        },
        encrypted: crypto::Sealed {
            nonce: String::new(),
            ciphertext: String::new(),
        },
    };
    let key = backup.kdf.derive(password)?;
    backup.encrypted = crypto::seal(&key, &backup.aad()?, secret.as_ref())?;
    serde_json::to_vec_pretty(&backup).map_err(|_| "wallet_backup_invalid".into())
}
pub fn open(bytes: &[u8], password: &str) -> Result<(String, LockedKey, u32)> {
    if bytes.len() > 16_384 {
        return Err("wallet_backup_invalid".into());
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|_| "wallet_backup_invalid")?;
    let (public, clear, version) = match header.format.as_str() {
        "asset-account-backup-v1" => {
            let b: LegacyBackup =
                serde_json::from_slice(bytes).map_err(|_| "wallet_backup_invalid")?;
            keys::validate_public(&b.public_key)?;
            if b.encrypted.sealed.ciphertext.len() != 96 {
                return Err("wallet_backup_invalid".into());
            }
            let aad = format!("{}:{}", b.format, b.public_key);
            (
                b.public_key,
                crypto::unwrap(password, aad.as_bytes(), &b.encrypted)?,
                1,
            )
        }
        FORMAT => {
            let b: Backup = serde_json::from_slice(bytes).map_err(|_| "wallet_backup_invalid")?;
            keys::validate_public(&b.public_key)?;
            if b.cipher != CIPHER
                || b.encrypted.ciphertext.len() != 96
                || b.encrypted.nonce.len() != 48
            {
                return Err("wallet_backup_invalid".into());
            }
            let key = b.kdf.derive(password)?;
            let clear = crypto::open(&key, &b.aad()?, &b.encrypted)?;
            (b.public_key, clear, 2)
        }
        _ => return Err("wallet_backup_invalid".into()),
    };
    let secret = LockedKey::from_slice(&clear)?;
    if keys::public(&secret)? != public {
        return Err("wallet_backup_invalid".into());
    }
    Ok((public, secret, version))
}
