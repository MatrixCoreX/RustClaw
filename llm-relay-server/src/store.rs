use std::{path::Path, sync::Mutex};

use anyhow::{bail, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Days, NaiveDate, Utc};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const CLIENT_SCOPES: &str = "chat.completions,models.read,quota.read";
const ADMIN_SCOPES: &str = "usage.admin.read,usage.admin.write";

#[derive(Debug)]
pub enum StoreError {
    InvalidKey,
    KeyDisabled,
    ScopeDenied,
    DailyRequestLimit,
    DailyTokenLimit,
    KeyInflightLimit,
    DeviceNotAllowed,
    EnrollmentInvalid,
    EnrollmentExpired,
    EnrollmentReplay,
    Database(anyhow::Error),
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error.into())
    }
}

#[derive(Clone, Debug)]
pub struct AuthenticatedKey {
    pub key_id: String,
    pub label: String,
    pub device_pubkey: Option<String>,
    pub daily_request_limit: u32,
    scopes: Vec<String>,
}

impl AuthenticatedKey {
    pub fn require_scope(&self, scope: &str) -> Result<(), StoreError> {
        self.scopes
            .iter()
            .any(|candidate| candidate == scope)
            .then_some(())
            .ok_or(StoreError::ScopeDenied)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct IssuedKey {
    pub key_id: String,
    pub label: String,
    pub device_pubkey: Option<String>,
    pub token: String,
    pub daily_request_limit: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct KeySummary {
    pub key_id: String,
    pub label: String,
    pub device_pubkey: Option<String>,
    pub enabled: bool,
    pub daily_request_limit: u32,
    pub created_at_epoch: i64,
    pub revoked_at_epoch: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageSnapshot {
    pub key_id: String,
    pub label: String,
    pub device_pubkey: String,
    pub day_utc: String,
    pub request_count: u32,
    pub successful_requests: u32,
    pub failed_requests: u32,
    pub total_tokens: u64,
    pub daily_request_limit: u32,
    pub remaining_requests: u32,
    pub reset_at_epoch: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdminUsageItem {
    pub key_id: String,
    pub label: String,
    pub device_pubkey: String,
    pub enabled: bool,
    pub day_utc: String,
    pub request_count: u32,
    pub successful_requests: u32,
    pub failed_requests: u32,
    pub total_tokens: u64,
    pub daily_request_limit: u32,
    pub remaining_requests: u32,
    pub last_request_at_epoch: Option<i64>,
    pub created_at_epoch: i64,
    pub revoked_at_epoch: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdminUsagePage {
    pub schema_version: u32,
    pub day_utc: String,
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u64,
    pub reset_at_epoch: i64,
    pub devices: Vec<AdminUsageItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdminAllowlistItem {
    pub device_pubkey: String,
    pub label: String,
    pub enabled: bool,
    pub daily_request_limit: u32,
    pub created_at_epoch: i64,
    pub revoked_at_epoch: Option<i64>,
    pub enrollment_status: String,
    pub key_id: Option<String>,
    pub key_created_at_epoch: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AdminAllowlistPage {
    pub schema_version: u32,
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u64,
    pub enrolled_total: u64,
    pub devices: Vec<AdminAllowlistItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DailyLimitUpdate {
    pub schema_version: u32,
    pub device_pubkey: String,
    pub previous_daily_request_limit: u32,
    pub daily_request_limit: u32,
    pub changed_at_epoch: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceAllowlistEntry {
    pub device_pubkey: String,
    pub label: String,
    pub enabled: bool,
    pub daily_request_limit: u32,
    pub created_at_epoch: i64,
    pub revoked_at_epoch: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnrollmentChallenge {
    pub schema_version: u32,
    pub challenge_id: String,
    pub device_pubkey: String,
    pub challenge: String,
    pub expires_at_epoch: i64,
}

pub struct RelayStore {
    connection: Mutex<Connection>,
    pepper: Vec<u8>,
}

impl RelayStore {
    pub fn open(path: &Path, pepper: &str) -> anyhow::Result<Self> {
        Self::open_inner(path, pepper, false)
    }

    pub fn open_and_recover(path: &Path, pepper: &str) -> anyhow::Result<Self> {
        Self::open_inner(path, pepper, true)
    }

    fn open_inner(path: &Path, pepper: &str, recover_interrupted: bool) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut connection = Connection::open(path)
            .with_context(|| format!("failed to open relay database {}", path.display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS relay_keys (
                 key_id TEXT PRIMARY KEY,
                 label TEXT NOT NULL,
                 device_pubkey TEXT,
                 token_digest BLOB NOT NULL,
                 scopes TEXT NOT NULL,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 daily_request_limit INTEGER NOT NULL,
                 created_at_epoch INTEGER NOT NULL,
                 revoked_at_epoch INTEGER
             );
             CREATE TABLE IF NOT EXISTS daily_usage (
                 key_id TEXT NOT NULL,
                 day_utc TEXT NOT NULL,
                 request_count INTEGER NOT NULL DEFAULT 0,
                 successful_requests INTEGER NOT NULL DEFAULT 0,
                 failed_requests INTEGER NOT NULL DEFAULT 0,
                 total_tokens INTEGER NOT NULL DEFAULT 0,
                 reserved_tokens INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (key_id, day_utc),
                 FOREIGN KEY (key_id) REFERENCES relay_keys(key_id)
             );
             CREATE TABLE IF NOT EXISTS relay_attempts (
                 request_id TEXT PRIMARY KEY,
                 key_id TEXT NOT NULL,
                 day_utc TEXT NOT NULL,
                 reserved_tokens INTEGER NOT NULL,
                 status TEXT NOT NULL,
                 total_tokens INTEGER NOT NULL DEFAULT 0,
                 created_at_epoch INTEGER NOT NULL,
                 completed_at_epoch INTEGER,
                 FOREIGN KEY (key_id) REFERENCES relay_keys(key_id)
             );
             CREATE INDEX IF NOT EXISTS relay_attempts_created_idx
                 ON relay_attempts(created_at_epoch);
             CREATE TABLE IF NOT EXISTS relay_admin_audit (
                 event_id TEXT PRIMARY KEY,
                 actor_key_id TEXT NOT NULL,
                 action TEXT NOT NULL,
                 target_key_id TEXT NOT NULL,
                 previous_value INTEGER,
                 new_value INTEGER,
                 created_at_epoch INTEGER NOT NULL,
                 FOREIGN KEY (actor_key_id) REFERENCES relay_keys(key_id),
                 FOREIGN KEY (target_key_id) REFERENCES relay_keys(key_id)
             );
             CREATE INDEX IF NOT EXISTS relay_admin_audit_created_idx
                 ON relay_admin_audit(created_at_epoch);",
        )?;
        if !table_has_column(&connection, "relay_keys", "device_pubkey")? {
            connection.execute("ALTER TABLE relay_keys ADD COLUMN device_pubkey TEXT", [])?;
        }
        connection.execute_batch(
            "DROP INDEX IF EXISTS relay_keys_device_pubkey_unique;
             CREATE UNIQUE INDEX IF NOT EXISTS relay_keys_active_device_pubkey_unique
                 ON relay_keys(device_pubkey)
                 WHERE device_pubkey IS NOT NULL AND enabled = 1;
             CREATE TABLE IF NOT EXISTS relay_device_allowlist (
                 device_pubkey TEXT PRIMARY KEY,
                 label TEXT NOT NULL,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 daily_request_limit INTEGER NOT NULL,
                 created_at_epoch INTEGER NOT NULL,
                 revoked_at_epoch INTEGER
             );
             CREATE TABLE IF NOT EXISTS relay_enrollment_challenges (
                 challenge_id TEXT PRIMARY KEY,
                 device_pubkey TEXT NOT NULL,
                 challenge TEXT NOT NULL,
                 expires_at_epoch INTEGER NOT NULL,
                 consumed_at_epoch INTEGER,
                 created_at_epoch INTEGER NOT NULL,
                 FOREIGN KEY (device_pubkey) REFERENCES relay_device_allowlist(device_pubkey)
             );
             CREATE INDEX IF NOT EXISTS relay_enrollment_challenges_expiry_idx
                 ON relay_enrollment_challenges(expires_at_epoch);",
        )?;
        connection.execute(
            "UPDATE relay_keys
             SET enabled = 0, revoked_at_epoch = COALESCE(revoked_at_epoch, ?1)
             WHERE enabled = 1 AND device_pubkey IS NULL
               AND scopes NOT LIKE '%usage.admin.%'",
            params![Utc::now().timestamp()],
        )?;
        if recover_interrupted {
            recover_interrupted_attempts(&mut connection)?;
        }
        connection.execute(
            "DELETE FROM relay_attempts WHERE created_at_epoch < ?1",
            params![Utc::now().timestamp() - 31 * 24 * 60 * 60],
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            pepper: pepper.as_bytes().to_vec(),
        })
    }

    pub fn allow_device(
        &self,
        label: &str,
        device_pubkey: &str,
        daily_request_limit: u32,
    ) -> anyhow::Result<DeviceAllowlistEntry> {
        let label = validate_label(label)?;
        let device_pubkey = normalize_device_pubkey(device_pubkey)?;
        if daily_request_limit == 0 {
            bail!("daily request limit must be positive");
        }
        let now = Utc::now().timestamp();
        self.connection
            .lock()
            .expect("relay database mutex poisoned")
            .execute(
                "INSERT INTO relay_device_allowlist
                 (device_pubkey, label, enabled, daily_request_limit, created_at_epoch, revoked_at_epoch)
                 VALUES (?1, ?2, 1, ?3, ?4, NULL)
                 ON CONFLICT(device_pubkey) DO UPDATE SET
                   label = excluded.label,
                   enabled = 1,
                   daily_request_limit = excluded.daily_request_limit,
                   revoked_at_epoch = NULL",
                params![device_pubkey, label, daily_request_limit, now],
            )?;
        Ok(DeviceAllowlistEntry {
            device_pubkey,
            label,
            enabled: true,
            daily_request_limit,
            created_at_epoch: now,
            revoked_at_epoch: None,
        })
    }

    pub fn issue_admin_key(&self, label: &str) -> anyhow::Result<IssuedKey> {
        self.issue_key_with_scopes(label, None, 1, ADMIN_SCOPES)
    }

    fn issue_key_with_scopes(
        &self,
        label: &str,
        device_pubkey: Option<String>,
        daily_request_limit: u32,
        scopes: &str,
    ) -> anyhow::Result<IssuedKey> {
        let label = validate_label(label)?;
        if daily_request_limit == 0 {
            bail!("daily request limit must be positive");
        }

        let key_id = Uuid::new_v4().simple().to_string();
        let mut secret = [0_u8; 32];
        OsRng.fill_bytes(&mut secret);
        let token = format!("lrk_{key_id}_{}", URL_SAFE_NO_PAD.encode(secret));
        let digest = self.digest(&token)?;
        let created_at = Utc::now().timestamp();
        self.connection
            .lock()
            .expect("relay database mutex poisoned")
            .execute(
                "INSERT INTO relay_keys
             (key_id, label, device_pubkey, token_digest, scopes, enabled,
              daily_request_limit, created_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
                params![
                    key_id,
                    label,
                    device_pubkey,
                    digest,
                    scopes,
                    daily_request_limit,
                    created_at
                ],
            )?;

        Ok(IssuedKey {
            key_id,
            label,
            device_pubkey,
            token,
            daily_request_limit,
        })
    }

    pub fn list_allowed_devices(&self) -> anyhow::Result<Vec<DeviceAllowlistEntry>> {
        let connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT device_pubkey, label, enabled, daily_request_limit,
                    created_at_epoch, revoked_at_epoch
             FROM relay_device_allowlist ORDER BY created_at_epoch, device_pubkey",
        )?;
        let entries = statement
            .query_map([], |row| {
                Ok(DeviceAllowlistEntry {
                    device_pubkey: row.get(0)?,
                    label: row.get(1)?,
                    enabled: row.get(2)?,
                    daily_request_limit: row.get(3)?,
                    created_at_epoch: row.get(4)?,
                    revoked_at_epoch: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(anyhow::Error::from)?;
        Ok(entries)
    }

    pub fn revoke_device(&self, device_pubkey: &str) -> anyhow::Result<bool> {
        let device_pubkey = normalize_device_pubkey(device_pubkey)?;
        let now = Utc::now().timestamp();
        let mut connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE relay_device_allowlist SET enabled = 0, revoked_at_epoch = ?2
             WHERE device_pubkey = ?1 AND enabled = 1",
            params![device_pubkey, now],
        )?;
        tx.execute(
            "UPDATE relay_keys SET enabled = 0, revoked_at_epoch = ?2
             WHERE device_pubkey = ?1 AND enabled = 1",
            params![device_pubkey, now],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn create_enrollment_challenge(
        &self,
        device_pubkey: &str,
    ) -> Result<EnrollmentChallenge, StoreError> {
        let device_pubkey =
            normalize_device_pubkey(device_pubkey).map_err(|_| StoreError::EnrollmentInvalid)?;
        let now = Utc::now().timestamp();
        let allowed = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT enabled FROM relay_device_allowlist WHERE device_pubkey = ?1",
                params![device_pubkey],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        if !allowed {
            return Err(StoreError::DeviceNotAllowed);
        }
        let challenge_id = Uuid::new_v4().to_string();
        let expires_at_epoch = now + 300;
        let challenge = crate::device_proof::canonical_enrollment_challenge(
            &challenge_id,
            &device_pubkey,
            expires_at_epoch,
        );
        self.connection
            .lock()
            .expect("relay database mutex poisoned")
            .execute(
                "INSERT INTO relay_enrollment_challenges
                 (challenge_id, device_pubkey, challenge, expires_at_epoch,
                  consumed_at_epoch, created_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![
                    challenge_id,
                    device_pubkey,
                    challenge,
                    expires_at_epoch,
                    now
                ],
            )?;
        Ok(EnrollmentChallenge {
            schema_version: 1,
            challenge_id,
            device_pubkey,
            challenge,
            expires_at_epoch,
        })
    }

    pub fn pending_enrollment_challenge(
        &self,
        challenge_id: &str,
        device_pubkey: &str,
    ) -> Result<EnrollmentChallenge, StoreError> {
        let device_pubkey =
            normalize_device_pubkey(device_pubkey).map_err(|_| StoreError::EnrollmentInvalid)?;
        let now = Utc::now().timestamp();
        let challenge = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT challenge, expires_at_epoch, consumed_at_epoch
                 FROM relay_enrollment_challenges
                 WHERE challenge_id = ?1 AND device_pubkey = ?2",
                params![challenge_id, device_pubkey],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::EnrollmentInvalid)?;
        if challenge.2.is_some() {
            return Err(StoreError::EnrollmentReplay);
        }
        if challenge.1 < now {
            return Err(StoreError::EnrollmentExpired);
        }
        Ok(EnrollmentChallenge {
            schema_version: 1,
            challenge_id: challenge_id.to_owned(),
            device_pubkey,
            challenge: challenge.0,
            expires_at_epoch: challenge.1,
        })
    }

    pub fn complete_enrollment(
        &self,
        challenge_id: &str,
        device_pubkey: &str,
    ) -> Result<IssuedKey, StoreError> {
        let device_pubkey =
            normalize_device_pubkey(device_pubkey).map_err(|_| StoreError::EnrollmentInvalid)?;
        let now = Utc::now().timestamp();
        let mut secret = [0_u8; 32];
        OsRng.fill_bytes(&mut secret);
        let existing_key_id = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT key_id FROM relay_keys WHERE device_pubkey = ?1
                 ORDER BY created_at_epoch DESC LIMIT 1",
                params![device_pubkey],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let key_id = existing_key_id.unwrap_or_else(|| Uuid::new_v4().simple().to_string());
        let token = format!("lrk_{key_id}_{}", URL_SAFE_NO_PAD.encode(secret));
        let digest = self.digest(&token).map_err(StoreError::Database)?;
        let mut connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let allowlist = tx
            .query_row(
                "SELECT label, daily_request_limit FROM relay_device_allowlist
                 WHERE device_pubkey = ?1 AND enabled = 1",
                params![device_pubkey],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?
            .ok_or(StoreError::DeviceNotAllowed)?;
        let consumed = tx.execute(
            "UPDATE relay_enrollment_challenges SET consumed_at_epoch = ?3
             WHERE challenge_id = ?1 AND device_pubkey = ?2
               AND consumed_at_epoch IS NULL AND expires_at_epoch >= ?3",
            params![challenge_id, device_pubkey, now],
        )?;
        if consumed != 1 {
            return Err(StoreError::EnrollmentReplay);
        }
        let updated = tx.execute(
            "UPDATE relay_keys SET label = ?2, token_digest = ?3, scopes = ?4,
                 enabled = 1, daily_request_limit = ?5, revoked_at_epoch = NULL
             WHERE key_id = ?1 AND device_pubkey = ?6",
            params![
                key_id,
                allowlist.0,
                digest,
                CLIENT_SCOPES,
                allowlist.1,
                device_pubkey
            ],
        )?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO relay_keys
                 (key_id, label, device_pubkey, token_digest, scopes, enabled,
                  daily_request_limit, created_at_epoch, revoked_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, NULL)",
                params![
                    key_id,
                    allowlist.0,
                    device_pubkey,
                    digest,
                    CLIENT_SCOPES,
                    allowlist.1,
                    now
                ],
            )?;
        }
        tx.commit()?;
        Ok(IssuedKey {
            key_id,
            label: allowlist.0,
            device_pubkey: Some(device_pubkey),
            token,
            daily_request_limit: allowlist.1,
        })
    }

    pub fn authenticate(&self, token: &str) -> Result<AuthenticatedKey, StoreError> {
        let key_id = parse_key_id(token).ok_or(StoreError::InvalidKey)?;
        let row = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT label, device_pubkey, token_digest, scopes, enabled, daily_request_limit
                 FROM relay_keys WHERE key_id = ?1",
                params![key_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, u32>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::InvalidKey)?;

        let mut mac = HmacSha256::new_from_slice(&self.pepper)
            .map_err(|error| StoreError::Database(anyhow::Error::msg(error.to_string())))?;
        mac.update(token.as_bytes());
        mac.verify_slice(&row.2)
            .map_err(|_| StoreError::InvalidKey)?;
        if !row.4 {
            return Err(StoreError::KeyDisabled);
        }

        Ok(AuthenticatedKey {
            key_id: key_id.to_owned(),
            label: row.0,
            device_pubkey: row.1,
            scopes: row
                .3
                .split(',')
                .map(str::trim)
                .filter(|scope| !scope.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            daily_request_limit: row.5,
        })
    }

    pub fn revoke_key(&self, key_id: &str) -> anyhow::Result<bool> {
        let changed = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .execute(
                "UPDATE relay_keys SET enabled = 0, revoked_at_epoch = ?2
             WHERE key_id = ?1 AND enabled = 1",
                params![key_id, Utc::now().timestamp()],
            )?;
        Ok(changed == 1)
    }

    pub fn list_keys(&self) -> anyhow::Result<Vec<KeySummary>> {
        let connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT key_id, label, device_pubkey, enabled, daily_request_limit,
                    created_at_epoch, revoked_at_epoch
             FROM relay_keys ORDER BY created_at_epoch, key_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(KeySummary {
                key_id: row.get(0)?,
                label: row.get(1)?,
                device_pubkey: row.get(2)?,
                enabled: row.get(3)?,
                daily_request_limit: row.get(4)?,
                created_at_epoch: row.get(5)?,
                revoked_at_epoch: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn active_key_count(&self) -> anyhow::Result<u64> {
        self.connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT COUNT(*) FROM relay_keys
                 WHERE enabled = 1 AND device_pubkey IS NOT NULL
                   AND scopes NOT LIKE '%usage.admin.%'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn reserve_attempt(
        &self,
        key: &AuthenticatedKey,
        request_id: &str,
        requested_tokens: u32,
        daily_token_limit: u64,
        max_inflight_per_key: u32,
    ) -> Result<(), StoreError> {
        let now = Utc::now();
        let day = now.format("%Y-%m-%d").to_string();
        let mut connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_key = tx
            .query_row(
                "SELECT enabled, daily_request_limit FROM relay_keys WHERE key_id = ?1",
                params![key.key_id],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?
            .ok_or(StoreError::InvalidKey)?;
        if !current_key.0 {
            return Err(StoreError::KeyDisabled);
        }
        let inflight = tx.query_row(
            "SELECT COUNT(*) FROM relay_attempts
             WHERE key_id = ?1 AND status = 'dispatched'",
            params![key.key_id],
            |row| row.get::<_, u32>(0),
        )?;
        if inflight >= max_inflight_per_key {
            return Err(StoreError::KeyInflightLimit);
        }
        tx.execute(
            "INSERT OR IGNORE INTO daily_usage(key_id, day_utc) VALUES (?1, ?2)",
            params![key.key_id, day],
        )?;
        let usage = tx.query_row(
            "SELECT request_count, total_tokens, reserved_tokens
             FROM daily_usage WHERE key_id = ?1 AND day_utc = ?2",
            params![key.key_id, day],
            |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )?;
        if usage.0 >= current_key.1 {
            return Err(StoreError::DailyRequestLimit);
        }
        if usage
            .1
            .saturating_add(usage.2)
            .saturating_add(u64::from(requested_tokens))
            > daily_token_limit
        {
            return Err(StoreError::DailyTokenLimit);
        }

        tx.execute(
            "UPDATE daily_usage
             SET request_count = request_count + 1,
                 reserved_tokens = reserved_tokens + ?3
             WHERE key_id = ?1 AND day_utc = ?2",
            params![key.key_id, day, requested_tokens],
        )?;
        tx.execute(
            "INSERT INTO relay_attempts
             (request_id, key_id, day_utc, reserved_tokens, status, created_at_epoch)
             VALUES (?1, ?2, ?3, ?4, 'dispatched', ?5)",
            params![
                request_id,
                key.key_id,
                day,
                requested_tokens,
                now.timestamp()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn settle_attempt(
        &self,
        request_id: &str,
        succeeded: bool,
        total_tokens: u64,
    ) -> anyhow::Result<()> {
        let mut connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let attempt = tx
            .query_row(
                "SELECT key_id, day_utc, reserved_tokens, status
                 FROM relay_attempts WHERE request_id = ?1",
                params![request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((key_id, day, reserved_tokens, status)) = attempt else {
            return Ok(());
        };
        if status != "dispatched" {
            return Ok(());
        }
        let (success_delta, failure_delta, status) = if succeeded {
            (1, 0, "succeeded")
        } else {
            (0, 1, "failed")
        };
        tx.execute(
            "UPDATE daily_usage
             SET successful_requests = successful_requests + ?3,
                 failed_requests = failed_requests + ?4,
                 total_tokens = total_tokens + ?5,
                 reserved_tokens = MAX(0, reserved_tokens - ?6)
             WHERE key_id = ?1 AND day_utc = ?2",
            params![
                key_id,
                day,
                success_delta,
                failure_delta,
                total_tokens,
                reserved_tokens
            ],
        )?;
        tx.execute(
            "UPDATE relay_attempts
             SET status = ?2, total_tokens = ?3, completed_at_epoch = ?4
             WHERE request_id = ?1 AND status = 'dispatched'",
            params![request_id, status, total_tokens, Utc::now().timestamp()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn quota_snapshot(&self, key: &AuthenticatedKey) -> anyhow::Result<UsageSnapshot> {
        let now = Utc::now();
        let day = now.format("%Y-%m-%d").to_string();
        let usage = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT request_count, successful_requests, failed_requests, total_tokens
                 FROM daily_usage WHERE key_id = ?1 AND day_utc = ?2",
                params![key.key_id, day],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                },
            )
            .optional()?
            .unwrap_or_default();
        let reset_at = next_day_epoch(now.date_naive());
        Ok(UsageSnapshot {
            key_id: key.key_id.clone(),
            label: key.label.clone(),
            device_pubkey: key.device_pubkey.clone().ok_or_else(|| {
                anyhow::anyhow!("relay client key is not bound to a device public key")
            })?,
            day_utc: day,
            request_count: usage.0,
            successful_requests: usage.1,
            failed_requests: usage.2,
            total_tokens: usage.3,
            daily_request_limit: key.daily_request_limit,
            remaining_requests: key.daily_request_limit.saturating_sub(usage.0),
            reset_at_epoch: reset_at,
        })
    }

    pub fn admin_usage_page(
        &self,
        day: NaiveDate,
        page: u32,
        per_page: u32,
        status: &str,
    ) -> anyhow::Result<AdminUsagePage> {
        let day_utc = day.format("%Y-%m-%d").to_string();
        let offset = u64::from(page.saturating_sub(1)).saturating_mul(u64::from(per_page));
        let connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let total = connection.query_row(
            "SELECT COUNT(*) FROM relay_keys
             WHERE scopes NOT LIKE '%usage.admin.%' AND device_pubkey IS NOT NULL
               AND (?1 = 'all'
                    OR (?1 = 'enabled' AND enabled = 1)
                    OR (?1 = 'revoked' AND enabled = 0))",
            params![status],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = connection.prepare(
            "SELECT k.key_id, k.label, k.device_pubkey, k.enabled,
                    COALESCE(u.request_count, 0),
                    COALESCE(u.successful_requests, 0),
                    COALESCE(u.failed_requests, 0),
                    COALESCE(u.total_tokens, 0),
                    k.daily_request_limit,
                    MAX(a.created_at_epoch),
                    k.created_at_epoch, k.revoked_at_epoch
             FROM relay_keys k
             LEFT JOIN daily_usage u
               ON u.key_id = k.key_id AND u.day_utc = ?1
             LEFT JOIN relay_attempts a
               ON a.key_id = k.key_id AND a.day_utc = ?1
             WHERE k.scopes NOT LIKE '%usage.admin.%' AND k.device_pubkey IS NOT NULL
               AND (?2 = 'all'
                    OR (?2 = 'enabled' AND k.enabled = 1)
                    OR (?2 = 'revoked' AND k.enabled = 0))
             GROUP BY k.key_id, k.label, k.device_pubkey, k.enabled, u.request_count,
                      u.successful_requests, u.failed_requests, u.total_tokens,
                      k.daily_request_limit, k.created_at_epoch, k.revoked_at_epoch
             ORDER BY MAX(a.created_at_epoch) IS NULL,
                      MAX(a.created_at_epoch) DESC,
                      k.created_at_epoch DESC, k.key_id
             LIMIT ?3 OFFSET ?4",
        )?;
        let rows = statement.query_map(params![day_utc, status, per_page, offset], |row| {
            let daily_request_limit = row.get::<_, u32>(8)?;
            let request_count = row.get::<_, u32>(4)?;
            Ok(AdminUsageItem {
                key_id: row.get(0)?,
                label: row.get(1)?,
                device_pubkey: row.get(2)?,
                enabled: row.get(3)?,
                day_utc: day_utc.clone(),
                request_count,
                successful_requests: row.get(5)?,
                failed_requests: row.get(6)?,
                total_tokens: row.get(7)?,
                daily_request_limit,
                remaining_requests: daily_request_limit.saturating_sub(request_count),
                last_request_at_epoch: row.get(9)?,
                created_at_epoch: row.get(10)?,
                revoked_at_epoch: row.get(11)?,
            })
        })?;
        let devices = rows.collect::<Result<Vec<_>, _>>()?;
        let total_pages = if total == 0 {
            0
        } else {
            total.div_ceil(u64::from(per_page))
        };
        Ok(AdminUsagePage {
            schema_version: 1,
            day_utc,
            page,
            per_page,
            total,
            total_pages,
            reset_at_epoch: next_day_epoch(day),
            devices,
        })
    }

    pub fn admin_allowlist_page(
        &self,
        page: u32,
        per_page: u32,
        status: &str,
    ) -> anyhow::Result<AdminAllowlistPage> {
        let offset = u64::from(page.saturating_sub(1)).saturating_mul(u64::from(per_page));
        let connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let status_filter =
            "(?1 = 'all' OR (?1 = 'enabled' AND a.enabled = 1) OR (?1 = 'revoked' AND a.enabled = 0))";
        let total = connection.query_row(
            &format!("SELECT COUNT(*) FROM relay_device_allowlist a WHERE {status_filter}"),
            params![status],
            |row| row.get::<_, u64>(0),
        )?;
        let enrolled_total = connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM relay_device_allowlist a
                 WHERE {status_filter}
                   AND EXISTS (
                     SELECT 1 FROM relay_keys k
                     WHERE k.device_pubkey = a.device_pubkey
                       AND k.scopes NOT LIKE '%usage.admin.%'
                   )"
            ),
            params![status],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = connection.prepare(&format!(
            "SELECT a.device_pubkey, a.label, a.enabled, a.daily_request_limit,
                    a.created_at_epoch, a.revoked_at_epoch,
                    k.key_id, k.enabled, k.created_at_epoch
             FROM relay_device_allowlist a
             LEFT JOIN relay_keys k ON k.key_id = (
               SELECT candidate.key_id FROM relay_keys candidate
               WHERE candidate.device_pubkey = a.device_pubkey
                 AND candidate.scopes NOT LIKE '%usage.admin.%'
               ORDER BY candidate.enabled DESC, candidate.created_at_epoch DESC,
                        candidate.key_id DESC
               LIMIT 1
             )
             WHERE {status_filter}
             ORDER BY a.enabled DESC, a.created_at_epoch DESC, a.device_pubkey
             LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = statement.query_map(params![status, per_page, offset], |row| {
            let key_id = row.get::<_, Option<String>>(6)?;
            let key_enabled = row.get::<_, Option<bool>>(7)?;
            let enrollment_status = match (key_id.is_some(), key_enabled) {
                (true, Some(true)) => "active",
                (true, _) => "revoked",
                (false, _) => "not_enrolled",
            };
            Ok(AdminAllowlistItem {
                device_pubkey: row.get(0)?,
                label: row.get(1)?,
                enabled: row.get(2)?,
                daily_request_limit: row.get(3)?,
                created_at_epoch: row.get(4)?,
                revoked_at_epoch: row.get(5)?,
                enrollment_status: enrollment_status.to_string(),
                key_id,
                key_created_at_epoch: row.get(8)?,
            })
        })?;
        let devices = rows.collect::<Result<Vec<_>, _>>()?;
        let total_pages = if total == 0 {
            0
        } else {
            total.div_ceil(u64::from(per_page))
        };
        Ok(AdminAllowlistPage {
            schema_version: 1,
            page,
            per_page,
            total,
            total_pages,
            enrolled_total,
            devices,
        })
    }

    pub fn update_daily_request_limit(
        &self,
        actor_key_id: &str,
        device_pubkey: &str,
        daily_request_limit: u32,
    ) -> anyhow::Result<Option<DailyLimitUpdate>> {
        let device_pubkey = normalize_device_pubkey(device_pubkey)?;
        let now = Utc::now().timestamp();
        let mut connection = self
            .connection
            .lock()
            .expect("relay database mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = tx
            .query_row(
                "SELECT key_id, daily_request_limit FROM relay_keys
                 WHERE device_pubkey = ?1 AND scopes NOT LIKE '%usage.admin.%'",
                params![device_pubkey],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?;
        let Some((target_key_id, previous_daily_request_limit)) = previous else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE relay_keys SET daily_request_limit = ?2 WHERE device_pubkey = ?1",
            params![device_pubkey, daily_request_limit],
        )?;
        tx.execute(
            "UPDATE relay_device_allowlist SET daily_request_limit = ?2
             WHERE device_pubkey = ?1",
            params![device_pubkey, daily_request_limit],
        )?;
        tx.execute(
            "INSERT INTO relay_admin_audit
             (event_id, actor_key_id, action, target_key_id,
              previous_value, new_value, created_at_epoch)
             VALUES (?1, ?2, 'daily_request_limit_updated', ?3, ?4, ?5, ?6)",
            params![
                Uuid::new_v4().to_string(),
                actor_key_id,
                target_key_id,
                previous_daily_request_limit,
                daily_request_limit,
                now
            ],
        )?;
        tx.commit()?;
        Ok(Some(DailyLimitUpdate {
            schema_version: 1,
            device_pubkey,
            previous_daily_request_limit,
            daily_request_limit,
            changed_at_epoch: now,
        }))
    }

    fn digest(&self, token: &str) -> anyhow::Result<Vec<u8>> {
        let mut mac = HmacSha256::new_from_slice(&self.pepper)
            .map_err(|error| anyhow::Error::msg(error.to_string()))?;
        mac.update(token.as_bytes());
        Ok(mac.finalize().into_bytes().to_vec())
    }
}

fn validate_label(value: &str) -> anyhow::Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.len() > 120 {
        bail!("key label must contain 1 to 120 characters");
    }
    Ok(normalized.to_owned())
}

pub fn normalize_device_pubkey(value: &str) -> anyhow::Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 128 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("device public key must be exactly 128 hexadecimal characters");
    }
    Ok(normalized)
}

fn table_has_column(connection: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.iter().any(|name| name == column))
}

fn recover_interrupted_attempts(connection: &mut Connection) -> anyhow::Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut statement = tx.prepare(
            "SELECT key_id, day_utc, COALESCE(SUM(reserved_tokens), 0), COUNT(*)
             FROM relay_attempts WHERE status = 'dispatched'
             GROUP BY key_id, day_utc",
        )?;
        let interrupted = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (key_id, day, reserved_tokens, failed_requests) in interrupted {
            tx.execute(
                "UPDATE daily_usage
                 SET reserved_tokens = MAX(0, reserved_tokens - ?3),
                     failed_requests = failed_requests + ?4
                 WHERE key_id = ?1 AND day_utc = ?2",
                params![key_id, day, reserved_tokens, failed_requests],
            )?;
        }
    }
    tx.execute(
        "UPDATE relay_attempts
         SET status = 'interrupted', completed_at_epoch = ?1
         WHERE status = 'dispatched'",
        params![Utc::now().timestamp()],
    )?;
    tx.commit()?;
    Ok(())
}

fn parse_key_id(token: &str) -> Option<&str> {
    let mut parts = token.splitn(3, '_');
    if parts.next()? != "lrk" {
        return None;
    }
    let key_id = parts.next()?;
    let secret = parts.next()?;
    (key_id.len() == 32
        && key_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        && secret.len() >= 32)
        .then_some(key_id)
}

fn next_day_epoch(day: NaiveDate) -> i64 {
    day.checked_add_days(Days::new(1))
        .and_then(|next| next.and_hms_opt(0, 0, 0))
        .map(|next| next.and_utc().timestamp())
        .unwrap_or_else(|| Utc::now().timestamp())
}
