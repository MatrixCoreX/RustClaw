pub(crate) fn prepare_conversation_input_attachments(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    attachments: &[ConversationInputAttachmentBinding],
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    if !scope.validate() || attachments.is_empty() || attachments.len() > 20 {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    if scope_for_record(&tx, &input_id)? != *scope {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let record = load_record_by_input_id(&tx, &scope.owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    if !matches!(
        record.receipt.preparation_state,
        ConversationInputPreparationState::Pending | ConversationInputPreparationState::Ready
    ) {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let expected_ids = record
        .content
        .iter()
        .filter_map(|content| match content {
            claw_core::conversation_input::ConversationInputContent::Attachment {
                attachment_id,
                ..
            } => Some(attachment_id.as_str()),
            claw_core::conversation_input::ConversationInputContent::Text { .. } => None,
        })
        .collect::<std::collections::HashSet<_>>();
    if expected_ids.len() != attachments.len()
        || attachments
            .iter()
            .any(|attachment| !expected_ids.contains(attachment.attachment_id.as_str()))
    {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    let now_ts = to_i64(crate::now_ts_u64())?;
    insert_conversation_input_attachment_bindings(
        &tx,
        scope,
        input_id,
        &expected_ids,
        attachments,
        now_ts,
    )?;
    if record.receipt.preparation_state == ConversationInputPreparationState::Pending {
        tx.execute(
            "UPDATE conversation_inputs
             SET preparation_state = 'ready', updated_at_ts = ?3
             WHERE input_id = ?1 AND owner_principal_id = ?2 AND preparation_state = 'pending'",
            params![input_id.to_string(), scope.owner_principal_id, now_ts],
        )
        .map_err(database_error)?;
        append_event(
            &tx,
            scope,
            input_id,
            "prepared",
            json!({
                "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
                "input_id": input_id,
                "attachment_count": attachments.len(),
                "preparation_state": "ready",
            }),
            now_ts,
        )?;
    }
    let prepared = load_record_by_input_id(&tx, &scope.owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)?;
    tx.commit().map_err(database_error)?;
    Ok(prepared)
}

fn insert_conversation_input_attachment_bindings(
    db: &Connection,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    expected_ids: &std::collections::HashSet<&str>,
    attachments: &[ConversationInputAttachmentBinding],
    now_ts: i64,
) -> Result<(), ConversationInputStoreError> {
    if expected_ids.len() != attachments.len()
        || attachments
            .iter()
            .any(|attachment| !expected_ids.contains(attachment.attachment_id.as_str()))
    {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    for attachment in attachments {
        let workspace_path = std::path::Path::new(&attachment.workspace_rel_path);
        if !valid_machine_token(&attachment.attachment_id, 512)
            || !valid_machine_token(&attachment.kind, 64)
            || attachment.workspace_rel_path.trim().is_empty()
            || attachment.workspace_rel_path.len() > 4_096
            || attachment.workspace_rel_path.contains('\0')
            || workspace_path.is_absolute()
            || workspace_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            || attachment.sha256.len() != 64
            || !attachment
                .sha256
                .chars()
                .all(|value| value.is_ascii_hexdigit())
        {
            return Err(ConversationInputStoreError::InvalidRequest);
        }
        db.execute(
            "INSERT OR IGNORE INTO conversation_input_attachments(
                attachment_id, input_id, owner_principal_id, kind, workspace_rel_path,
                mime_type, display_name, size_bytes, sha256, created_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                attachment.attachment_id,
                input_id.to_string(),
                scope.owner_principal_id,
                attachment.kind,
                attachment.workspace_rel_path,
                attachment.mime_type,
                attachment.display_name,
                to_i64(attachment.size_bytes)?,
                attachment.sha256,
                now_ts,
            ],
        )
        .map_err(database_error)?;
        let stored = db
            .query_row(
                "SELECT input_id, owner_principal_id, kind, workspace_rel_path,
                        mime_type, display_name, size_bytes, sha256
                 FROM conversation_input_attachments WHERE attachment_id = ?1",
                [&attachment.attachment_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .map_err(database_error)?;
        if stored
            != (
                input_id.to_string(),
                scope.owner_principal_id.clone(),
                attachment.kind.clone(),
                attachment.workspace_rel_path.clone(),
                attachment.mime_type.clone(),
                attachment.display_name.clone(),
                to_i64(attachment.size_bytes)?,
                attachment.sha256.clone(),
            )
        {
            return Err(ConversationInputStoreError::IdempotencyConflict);
        }
    }
    Ok(())
}

pub(crate) fn conversation_input_attachments(
    pool: &DbPool,
    input_id: Uuid,
) -> Result<Vec<ConversationInputAttachmentBinding>, ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let mut statement = db
        .prepare(
            "SELECT attachment_id, kind, workspace_rel_path, mime_type, display_name,
                    size_bytes, sha256
             FROM conversation_input_attachments WHERE input_id = ?1 ORDER BY attachment_id",
        )
        .map_err(database_error)?;
    let attachments = statement
        .query_map([input_id.to_string()], |row| {
            Ok(ConversationInputAttachmentBinding {
                attachment_id: row.get(0)?,
                kind: row.get(1)?,
                workspace_rel_path: row.get(2)?,
                mime_type: row.get(3)?,
                display_name: row.get(4)?,
                size_bytes: from_i64(row.get(5)?)?,
                sha256: row.get(6)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(attachments)
}
