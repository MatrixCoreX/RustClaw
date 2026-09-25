fn principal_has_enabled_auth_key(
    db: &Connection,
    principal_id: &str,
) -> Result<bool, ConversationInputStoreError> {
    db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM auth_keys
            WHERE principal_id = ?1 AND enabled = 1
         )",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(database_error)
}

fn scope_authorization_is_current(
    db: &Connection,
    scope: &OwnedConversationInputScope,
    task_id: &str,
) -> Result<bool, ConversationInputStoreError> {
    if !principal_has_enabled_auth_key(db, &scope.owner_principal_id)? {
        return Ok(false);
    }
    if scope.conversation.channel == "ui" {
        return Ok(true);
    }
    let task_scope = db
        .query_row(
            "SELECT channel, external_user_id, external_chat_id, payload_json
             FROM tasks WHERE task_id = ?1 AND principal_id = ?2",
            params![task_id, scope.owner_principal_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((channel, external_user_id, external_chat_id, payload_json)) = task_scope else {
        return Ok(false);
    };
    if channel != scope.conversation.channel {
        return Ok(false);
    }
    let payload = serde_json::from_str::<Value>(&payload_json).map_err(|error| {
        ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_input_task_payload_invalid:{error}"
        ))
    })?;
    let account_id = payload
        .get("channel_ingress")
        .and_then(Value::as_object)
        .and_then(|ingress| ingress.get("account_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if account_id.is_empty() || account_id != scope.conversation.channel_account_id {
        return Ok(false);
    }
    let external_user_id = external_user_id.as_deref().filter(|value| !value.is_empty());
    let external_chat_id = external_chat_id.as_deref().filter(|value| !value.is_empty());
    if external_user_id.is_none() && external_chat_id.is_none() {
        return Ok(false);
    }
    db.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM channel_bindings binding
            JOIN auth_keys key ON key.user_key = binding.user_key
            WHERE binding.channel = ?1
              AND key.principal_id = ?2
              AND key.enabled = 1
              AND (
                    (?3 IS NOT NULL AND ?4 IS NOT NULL
                     AND binding.external_user_id = ?3
                     AND binding.external_chat_id = ?4)
                 OR (?3 IS NOT NULL AND binding.external_user_id = ?3)
                 OR (?3 IS NULL AND ?4 IS NOT NULL AND binding.external_chat_id = ?4)
              )
         )",
        params![
            scope.conversation.channel,
            scope.owner_principal_id,
            external_user_id,
            external_chat_id,
        ],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(database_error)
}

fn reject_pending_inputs_after_authorization_revocation(
    tx: &Transaction<'_>,
    scope: &OwnedConversationInputScope,
    task_id: &str,
    pending: &[ConversationInputRecord],
) -> Result<(), ConversationInputStoreError> {
    let now_ts = to_i64(crate::now_ts_u64())?;
    for record in pending {
        let changed = tx
            .execute(
                "UPDATE conversation_inputs
                 SET disposition = 'rejected', decision_ref = 'authorization_revoked',
                     updated_at_ts = ?3
                 WHERE input_id = ?1 AND target_task_id = ?2
                   AND preparation_state = 'ready' AND disposition = 'pending'",
                params![record.receipt.input_id.to_string(), task_id, now_ts],
            )
            .map_err(database_error)?;
        if changed != 1 {
            return Err(ConversationInputStoreError::TargetConflict);
        }
        append_event(
            tx,
            scope,
            record.receipt.input_id,
            "rejected",
            serde_json::json!({
                "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
                "input_id": record.receipt.input_id,
                "task_id": task_id,
                "error_code": "authorization_revoked",
                "retryable": false,
            }),
            now_ts,
        )?;
    }
    tx.execute(
        "UPDATE conversation_input_scopes
         SET focus_task_id = NULL, execution_epoch = execution_epoch + 1,
             updated_at_ts = ?6
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5
           AND focus_task_id = ?7",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            now_ts,
            task_id,
        ],
    )
    .map_err(database_error)?;
    Ok(())
}
