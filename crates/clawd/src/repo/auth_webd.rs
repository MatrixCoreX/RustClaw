use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordVerifier};
use rusqlite::{params, Connection, OptionalExtension};

use super::normalize_user_key;
use crate::now_ts;

/// Returns `user_key` when username/password match and both webd row and auth_keys are enabled.
pub(crate) fn verify_webd_password_login(
    db: &Connection,
    username: &str,
    password: &str,
) -> anyhow::Result<Option<String>> {
    let username_norm = username.trim().to_lowercase();
    if username_norm.is_empty() || password.is_empty() {
        return Ok(None);
    }
    let row: Option<(String, String)> = db
        .query_row(
            "SELECT w.password_hash, w.user_key
             FROM webd_login_accounts w
             INNER JOIN auth_keys k ON k.user_key = w.user_key
             WHERE w.username = ?1 AND w.enabled = 1 AND k.enabled = 1",
            params![username_norm],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((hash_str, user_key)) = row else {
        return Ok(None);
    };
    let parsed = PasswordHash::new(&hash_str)
        .map_err(|e| anyhow::anyhow!("invalid stored password hash: {e}"))?;
    if Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_err()
    {
        return Ok(None);
    }
    Ok(Some(user_key))
}

pub(crate) fn hash_password_for_webd_login(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))
}

/// Admin: bind or update a webd login row for an existing enabled auth key.
pub(crate) fn upsert_webd_login_account(
    db: &Connection,
    username: &str,
    password: &str,
    user_key: &str,
) -> anyhow::Result<()> {
    let username_norm = username.trim().to_lowercase();
    let uk = normalize_user_key(user_key);
    let password = password.trim();
    if username_norm.is_empty() || uk.is_empty() {
        anyhow::bail!("username and user_key required");
    }
    if password.is_empty() {
        anyhow::bail!("password required");
    }
    let exists: bool = db
        .query_row(
            "SELECT 1 FROM auth_keys WHERE user_key = ?1 AND enabled = 1 LIMIT 1",
            params![uk],
            |_| Ok(1_i32),
        )
        .optional()?
        .is_some();
    if !exists {
        anyhow::bail!("user_key does not exist or is disabled in auth_keys");
    }
    let username_owner = db
        .query_row(
            "SELECT user_key FROM webd_login_accounts WHERE username = ?1 LIMIT 1",
            params![username_norm],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if username_owner
        .as_deref()
        .is_some_and(|existing_user_key| existing_user_key != uk)
    {
        anyhow::bail!("username already assigned to another key");
    }
    let ph = hash_password_for_webd_login(password)?;
    let now = now_ts();
    db.execute(
        "DELETE FROM webd_login_accounts WHERE user_key = ?1 AND username != ?2",
        params![uk, username_norm],
    )?;
    db.execute(
        "INSERT INTO webd_login_accounts (username, password_hash, user_key, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(username) DO UPDATE SET
           password_hash=excluded.password_hash,
           user_key=excluded.user_key,
           enabled=1,
           updated_at=excluded.updated_at",
        params![username_norm, ph, uk, now],
    )?;
    Ok(())
}
