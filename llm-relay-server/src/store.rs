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

#[derive(Debug)]
pub enum StoreError {
    InvalidKey,
    KeyDisabled,
    ScopeDenied,
    DailyRequestLimit,
    DailyTokenLimit,
    KeyInflightLimit,
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
    pub token: String,
    pub daily_request_limit: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct KeySummary {
    pub key_id: String,
    pub label: String,
    pub enabled: bool,
    pub daily_request_limit: u32,
    pub created_at_epoch: i64,
    pub revoked_at_epoch: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageSnapshot {
    pub key_id: String,
    pub label: String,
    pub day_utc: String,
    pub request_count: u32,
    pub successful_requests: u32,
    pub failed_requests: u32,
    pub total_tokens: u64,
    pub daily_request_limit: u32,
    pub remaining_requests: u32,
    pub reset_at_epoch: i64,
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
                 ON relay_attempts(created_at_epoch);",
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

    pub fn issue_key(&self, label: &str, daily_request_limit: u32) -> anyhow::Result<IssuedKey> {
        let label = label.trim();
        if label.is_empty() || label.len() > 120 {
            bail!("key label must contain 1 to 120 characters");
        }
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
             (key_id, label, token_digest, scopes, enabled, daily_request_limit, created_at_epoch)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
                params![
                    key_id,
                    label,
                    digest,
                    "chat.completions,models.read,quota.read",
                    daily_request_limit,
                    created_at
                ],
            )?;

        Ok(IssuedKey {
            key_id,
            label: label.to_owned(),
            token,
            daily_request_limit,
        })
    }

    pub fn authenticate(&self, token: &str) -> Result<AuthenticatedKey, StoreError> {
        let key_id = parse_key_id(token).ok_or(StoreError::InvalidKey)?;
        let row = self
            .connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT label, token_digest, scopes, enabled, daily_request_limit
                 FROM relay_keys WHERE key_id = ?1",
                params![key_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, u32>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::InvalidKey)?;

        let mut mac = HmacSha256::new_from_slice(&self.pepper)
            .map_err(|error| StoreError::Database(anyhow::Error::msg(error.to_string())))?;
        mac.update(token.as_bytes());
        mac.verify_slice(&row.1)
            .map_err(|_| StoreError::InvalidKey)?;
        if !row.3 {
            return Err(StoreError::KeyDisabled);
        }

        Ok(AuthenticatedKey {
            key_id: key_id.to_owned(),
            label: row.0,
            scopes: row
                .2
                .split(',')
                .map(str::trim)
                .filter(|scope| !scope.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            daily_request_limit: row.4,
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
            "SELECT key_id, label, enabled, daily_request_limit, created_at_epoch, revoked_at_epoch
             FROM relay_keys ORDER BY created_at_epoch, key_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(KeySummary {
                key_id: row.get(0)?,
                label: row.get(1)?,
                enabled: row.get(2)?,
                daily_request_limit: row.get(3)?,
                created_at_epoch: row.get(4)?,
                revoked_at_epoch: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn active_key_count(&self) -> anyhow::Result<u64> {
        self.connection
            .lock()
            .expect("relay database mutex poisoned")
            .query_row(
                "SELECT COUNT(*) FROM relay_keys WHERE enabled = 1",
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

    fn digest(&self, token: &str) -> anyhow::Result<Vec<u8>> {
        let mut mac = HmacSha256::new_from_slice(&self.pepper)
            .map_err(|error| anyhow::Error::msg(error.to_string()))?;
        mac.update(token.as_bytes());
        Ok(mac.finalize().into_bytes().to_vec())
    }
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
