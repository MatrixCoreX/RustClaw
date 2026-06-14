use claw_core::config::{AppConfig, ChannelBindingConfig};
use rusqlite::{params, Connection};

use super::normalize_user_key;
use crate::{normalize_external_id_opt, now_ts};

fn seed_channel_binding_rows(
    db: &Connection,
    channel: &str,
    bindings: &[ChannelBindingConfig],
) -> anyhow::Result<()> {
    let now = now_ts();
    for binding in bindings {
        let user_key = normalize_user_key(&binding.user_key);
        if user_key.is_empty() {
            continue;
        }
        let external_user_id = normalize_external_id_opt(Some(&binding.external_user_id));
        let external_chat_id = normalize_external_id_opt(Some(&binding.external_chat_id))
            .or_else(|| external_user_id.clone());
        if external_user_id.is_none() && external_chat_id.is_none() {
            continue;
        }
        db.execute(
            "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(channel, external_user_id, external_chat_id)
             DO UPDATE SET user_key=excluded.user_key, updated_at=excluded.updated_at",
            params![channel, external_user_id, external_chat_id, user_key, now],
        )?;
    }
    Ok(())
}

pub(crate) fn seed_channel_bindings(db: &Connection, config: &AppConfig) -> anyhow::Result<()> {
    seed_channel_binding_rows(db, "telegram", &config.telegram.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp_cloud.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp_web.bindings)?;
    Ok(())
}
