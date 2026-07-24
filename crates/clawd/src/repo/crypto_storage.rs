use claw_core::types::ExchangeCredentialStatus;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::{now_ts, AppState};

#[derive(Clone, Debug)]
pub(crate) struct StoredExchangeCredential {
    user_key: String,
    exchange: String,
    api_key: String,
    api_secret: String,
    passphrase: Option<String>,
    enabled: i64,
    updated_at: String,
}

pub(crate) fn status_for_user_key(
    state: &AppState,
    user_key: &str,
) -> anyhow::Result<Vec<ExchangeCredentialStatus>> {
    let user_key = super::auth::normalize_user_key(user_key);
    if user_key.is_empty() {
        return Ok(Vec::new());
    }
    let db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    let mut out = Vec::new();
    for exchange in ["binance", "okx"] {
        let row = db
            .query_row(
                "SELECT api_key, updated_at, enabled
                 FROM exchange_api_credentials
                 WHERE user_key = ?1 AND exchange = ?2
                 LIMIT 1",
                params![user_key, exchange],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let (configured, api_key_masked, updated_at) = match row {
            Some((api_key, updated_at, enabled)) if enabled == 1 => {
                (true, Some(api_key), Some(updated_at))
            }
            _ => (false, None, None),
        };
        out.push(ExchangeCredentialStatus {
            exchange: exchange.to_string(),
            configured,
            api_key_masked,
            updated_at,
        });
    }
    Ok(out)
}

pub(crate) fn upsert_for_user_key(
    state: &AppState,
    user_key: &str,
    exchange_raw: &str,
    api_key: &str,
    api_secret: &str,
    passphrase: Option<&str>,
) -> anyhow::Result<ExchangeCredentialStatus> {
    let user_key = super::auth::normalize_user_key(user_key);
    if user_key.is_empty() {
        anyhow::bail!("user_key is required");
    }
    let exchange = crate::normalize_exchange_name(exchange_raw)?;
    let api_key = api_key.trim();
    let api_secret = api_secret.trim();
    if api_key.is_empty() || api_secret.is_empty() {
        anyhow::bail!("api_key and api_secret are required");
    }
    let passphrase = passphrase.map(str::trim).filter(|value| !value.is_empty());
    let now = now_ts();
    let db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    db.execute(
        "INSERT INTO exchange_api_credentials
            (user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(user_key, exchange) DO UPDATE SET
            api_key = excluded.api_key,
            api_secret = excluded.api_secret,
            passphrase = excluded.passphrase,
            enabled = 1,
            updated_at = excluded.updated_at",
        params![user_key, exchange, api_key, api_secret, passphrase, now],
    )?;
    Ok(ExchangeCredentialStatus {
        exchange,
        configured: true,
        api_key_masked: Some(api_key.to_string()),
        updated_at: Some(now),
    })
}

pub(crate) fn credential_context_for_user_key(
    state: &AppState,
    user_key: &str,
) -> anyhow::Result<Value> {
    let user_key = super::auth::normalize_user_key(user_key);
    if user_key.is_empty() {
        return Ok(serde_json::json!({}));
    }
    let db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    let mut stmt = db.prepare(
        "SELECT exchange, api_key, api_secret, passphrase
         FROM exchange_api_credentials
         WHERE user_key = ?1 AND enabled = 1",
    )?;
    let rows = stmt.query_map(params![user_key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut exchanges = serde_json::Map::new();
    for row in rows {
        let (exchange, api_key, api_secret, passphrase) = row?;
        exchanges.insert(
            exchange,
            serde_json::json!({
                "api_key": api_key,
                "api_secret": api_secret,
                "passphrase": passphrase,
            }),
        );
    }
    Ok(Value::Object(exchanges))
}

pub(crate) fn rebind_user_key(
    state: &AppState,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<usize> {
    let db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    db.execute(
        "UPDATE exchange_api_credentials SET user_key = ?2 WHERE user_key = ?1",
        params![old_user_key, new_user_key],
    )
    .map_err(Into::into)
}

pub(crate) fn take_for_user_key(
    state: &AppState,
    user_key: &str,
) -> anyhow::Result<Vec<StoredExchangeCredential>> {
    let mut db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    let rows = select_credentials(&db, Some(user_key))?;
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM exchange_api_credentials WHERE user_key = ?1",
        params![user_key],
    )?;
    tx.commit()?;
    Ok(rows)
}

pub(crate) fn take_all(state: &AppState) -> anyhow::Result<Vec<StoredExchangeCredential>> {
    let mut db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    let rows = select_credentials(&db, None)?;
    let tx = db.transaction()?;
    tx.execute("DELETE FROM exchange_api_credentials", [])?;
    tx.commit()?;
    Ok(rows)
}

pub(crate) fn restore(state: &AppState, rows: &[StoredExchangeCredential]) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut db = state
        .core
        .skill_storage
        .crypto_pool()
        .get()
        .map_err(|error| anyhow::anyhow!("crypto storage pool: {error}"))?;
    let tx = db.transaction()?;
    for row in rows {
        tx.execute(
            "INSERT INTO exchange_api_credentials
                (user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(user_key, exchange) DO UPDATE SET
                api_key = excluded.api_key,
                api_secret = excluded.api_secret,
                passphrase = excluded.passphrase,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
            params![
                row.user_key,
                row.exchange,
                row.api_key,
                row.api_secret,
                row.passphrase,
                row.enabled,
                row.updated_at
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn select_credentials(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<StoredExchangeCredential>> {
    let sql = if user_key.is_some() {
        "SELECT user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at
         FROM exchange_api_credentials WHERE user_key = ?1 ORDER BY exchange"
    } else {
        "SELECT user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at
         FROM exchange_api_credentials ORDER BY user_key, exchange"
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(StoredExchangeCredential {
            user_key: row.get(0)?,
            exchange: row.get(1)?,
            api_key: row.get(2)?,
            api_secret: row.get(3)?,
            passphrase: row.get(4)?,
            enabled: row.get(5)?,
            updated_at: row.get(6)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

#[cfg(test)]
#[path = "crypto_storage_tests.rs"]
mod tests;
