pub(crate) fn ensure_conversation_input_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_SQL)?;
    verify_or_record_migration(db, MIGRATION_ID, MIGRATION_MANIFEST)?;
    verify_or_record_migration(db, TASK_CLAIM_MIGRATION_ID, TASK_CLAIM_MIGRATION_MANIFEST)?;
    verify_or_record_migration(
        db,
        ACTION_DISPATCH_MIGRATION_ID,
        ACTION_DISPATCH_MIGRATION_MANIFEST,
    )?;
    verify_or_record_migration(
        db,
        TERMINAL_BOUNDARY_MIGRATION_ID,
        TERMINAL_BOUNDARY_MIGRATION_MANIFEST,
    )?;
    verify_or_record_migration(db, ATTACHMENT_MIGRATION_ID, ATTACHMENT_MIGRATION_MANIFEST)?;
    crate::ensure_column_exists(
        db,
        "conversation_input_task_claims",
        "claim_token",
        "ALTER TABLE conversation_input_task_claims ADD COLUMN claim_token TEXT NOT NULL DEFAULT ''",
    )?;
    verify_or_record_migration(
        db,
        TASK_CLAIM_FENCING_MIGRATION_ID,
        TASK_CLAIM_FENCING_MIGRATION_MANIFEST,
    )?;
    verify_or_record_migration(
        db,
        TASK_TEMPLATE_MIGRATION_ID,
        TASK_TEMPLATE_MIGRATION_MANIFEST,
    )?;
    crate::ensure_column_exists(
        db,
        "conversation_inputs",
        "decision_kind",
        "ALTER TABLE conversation_inputs ADD COLUMN decision_kind TEXT",
    )?;
    verify_or_record_migration(
        db,
        DECISION_KIND_MIGRATION_ID,
        DECISION_KIND_MIGRATION_MANIFEST,
    )?;
    Ok(())
}

pub(crate) fn accept_conversation_input(
    pool: &DbPool,
    input: &AcceptConversationInput,
) -> Result<AcceptConversationInputOutcome, ConversationInputStoreError> {
    validate_accept_input(input)?;
    let outcome = crate::sqlite_busy_retry::with_sqlite_busy_retry(
        crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
        || -> anyhow::Result<AcceptConversationInputOutcome> {
            let mut db = pool.get().context("conversation_input_db_pool_failed")?;
            ensure_conversation_input_schema(&db)?;
            Ok(accept_conversation_input_in_db(
                &mut db,
                input,
                crate::now_ts_u64(),
            )?)
        },
    );
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => match error.downcast::<ConversationInputStoreError>() {
            Ok(error) => Err(error),
            Err(error) => Err(ConversationInputStoreError::Database(error)),
        },
    }
}

pub(crate) fn accept_conversation_input_with_task_template(
    pool: &DbPool,
    input: &AcceptConversationInput,
    template: &ConversationInputTaskTemplate,
    attachments: &[ConversationInputAttachmentBinding],
) -> Result<AcceptConversationInputOutcome, ConversationInputStoreError> {
    validate_accept_input(input)?;
    validate_task_template(template)?;
    let outcome = crate::sqlite_busy_retry::with_sqlite_busy_retry(
        crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
        || -> anyhow::Result<AcceptConversationInputOutcome> {
            let mut db = pool.get().context("conversation_input_db_pool_failed")?;
            ensure_conversation_input_schema(&db)?;
            Ok(accept_conversation_input_in_db_with_resources(
                &mut db,
                input,
                Some(template),
                attachments,
                crate::now_ts_u64(),
            )?)
        },
    );
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => match error.downcast::<ConversationInputStoreError>() {
            Ok(error) => Err(error),
            Err(error) => Err(ConversationInputStoreError::Database(error)),
        },
    }
}

fn validate_task_template(
    template: &ConversationInputTaskTemplate,
) -> Result<(), ConversationInputStoreError> {
    let payload = template
        .task
        .payload
        .as_object()
        .ok_or(ConversationInputStoreError::InvalidRequest)?;
    let ingress = template
        .task
        .ingress
        .as_ref()
        .ok_or(ConversationInputStoreError::InvalidRequest)?;
    let bounded_header = |value: &Option<String>| {
        value
            .as_deref()
            .is_none_or(|value| valid_machine_token(value, 64))
    };
    if template.schema_version != 1
        || !matches!(template.task.kind, claw_core::types::TaskKind::Ask)
        || template.task.user_key.is_some()
        || template.task.idempotency_key.is_some()
        || !ingress.attachments.is_empty()
        || ingress.context_token.is_some()
        || payload.contains_key("text")
        || payload.contains_key("attachments")
        || payload.contains_key("conversation_input_id")
        || payload.contains_key(crate::task_execution_policy::POLICY_PAYLOAD_FIELD)
        || !bounded_header(&template.client_origin)
        || !bounded_header(&template.execution_mode)
    {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    Ok(())
}

pub(crate) fn get_conversation_input(
    pool: &DbPool,
    owner_principal_id: &str,
    input_id: Uuid,
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    load_record_by_input_id(&db, owner_principal_id, &input_id.to_string())?
        .ok_or(ConversationInputStoreError::NotFound)
}

pub(crate) fn get_conversation_input_by_client_message_id(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    client_message_id: &str,
) -> Result<ConversationInputRecord, ConversationInputStoreError> {
    if !scope.validate() || client_message_id.trim().is_empty() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    load_record_by_client_message_id(&db, scope, client_message_id)?
        .ok_or(ConversationInputStoreError::NotFound)
}

pub(crate) fn list_recoverable_conversation_input_tasks(
    pool: &DbPool,
    limit: u32,
) -> Result<Vec<RecoverableConversationInputTask>, ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let now_ts = to_i64(crate::now_ts_u64())?;
    let mut statement = db
        .prepare(
            "SELECT input.input_id, input.owner_principal_id, template.template_json,
                    template.template_digest
             FROM conversation_inputs input
             JOIN conversation_input_task_templates template
               ON template.input_id = input.input_id
              AND template.owner_principal_id = input.owner_principal_id
             WHERE input.target_task_id IS NULL
               AND input.delivery_mode = 'auto'
               AND input.preparation_state = 'ready'
               AND input.disposition = 'pending'
               AND template.next_attempt_at_ts <= ?1
               AND NOT EXISTS(
                    SELECT 1 FROM conversation_inputs older
                    WHERE older.owner_principal_id = input.owner_principal_id
                      AND older.agent_id = input.agent_id
                      AND older.channel = input.channel
                      AND older.channel_account_id = input.channel_account_id
                      AND older.conversation_id = input.conversation_id
                      AND older.target_task_id IS NULL
                      AND older.delivery_mode = 'auto'
                      AND older.preparation_state = 'ready'
                      AND older.disposition = 'pending'
                      AND older.input_seq < input.input_seq
               )
             ORDER BY input.accepted_at_ts ASC, input.input_seq ASC
             LIMIT ?2",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![now_ts, i64::from(limit.clamp(1, 100))], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let mut recovered = Vec::with_capacity(rows.len());
    for (input_id, owner_principal_id, template_json, template_digest) in rows {
        let actual_digest = format!("{:x}", Sha256::digest(template_json.as_bytes()));
        if actual_digest != template_digest {
            return Err(ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_task_template_digest_mismatch"
            )));
        }
        let input_id = Uuid::parse_str(&input_id).map_err(|error| {
            ConversationInputStoreError::Database(anyhow::anyhow!(
                "conversation_input_id_invalid:{error}"
            ))
        })?;
        let record = load_record_by_input_id(&db, &owner_principal_id, &input_id.to_string())?
            .ok_or(ConversationInputStoreError::NotFound)?;
        let template = serde_json::from_str::<ConversationInputTaskTemplate>(&template_json)
            .map_err(|error| {
                ConversationInputStoreError::Database(anyhow::anyhow!(
                    "conversation_input_task_template_invalid:{error}"
                ))
            })?;
        validate_task_template(&template)?;
        recovered.push(RecoverableConversationInputTask {
            scope: OwnedConversationInputScope {
                owner_principal_id,
                conversation: record.receipt.scope.clone(),
            },
            record,
            template,
        });
    }
    Ok(recovered)
}

pub(crate) fn defer_conversation_input_task_recovery(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    claim_token: Uuid,
    error_code: &str,
) -> Result<bool, ConversationInputStoreError> {
    if !scope.validate() || !valid_machine_token(error_code, 128) {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let attempts = db
        .query_row(
            "SELECT attempt_count FROM conversation_input_task_templates
             WHERE input_id = ?1 AND owner_principal_id = ?2",
            params![input_id.to_string(), scope.owner_principal_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(database_error)?
        .unwrap_or(0)
        .max(0) as u32;
    let delay_seconds = 2_u64.saturating_pow(attempts.min(8)).clamp(1, 300);
    db.execute(
        "UPDATE conversation_input_task_templates
         SET attempt_count = attempt_count + 1, next_attempt_at_ts = ?5,
             last_error_code = ?6, updated_at_ts = ?4
         WHERE input_id = ?1 AND owner_principal_id = ?2
           AND EXISTS(
                SELECT 1 FROM conversation_input_task_claims claim
                WHERE claim.owner_principal_id = ?2 AND claim.agent_id = ?7
                  AND claim.channel = ?8 AND claim.channel_account_id = ?9
                  AND claim.conversation_id = ?10 AND claim.input_id = ?1
                  AND claim.claim_token = ?3
           )",
        params![
            input_id.to_string(),
            scope.owner_principal_id,
            claim_token.to_string(),
            to_i64(crate::now_ts_u64())?,
            to_i64(crate::now_ts_u64().saturating_add(delay_seconds))?,
            error_code,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
        ],
    )
    .map(|changed| changed == 1)
    .map_err(database_error)
}

pub(crate) fn reject_unbound_conversation_input_authorization(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    claim_token: Uuid,
) -> Result<bool, ConversationInputStoreError> {
    let mut db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let owns_claim = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_input_task_claims
                WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
                  AND channel_account_id = ?4 AND conversation_id = ?5
                  AND input_id = ?6 AND claim_token = ?7
             )",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                input_id.to_string(),
                claim_token.to_string(),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?
        != 0;
    if !owns_claim {
        tx.commit().map_err(database_error)?;
        return Ok(false);
    }
    let now_ts = to_i64(crate::now_ts_u64())?;
    let changed = tx
        .execute(
            "UPDATE conversation_inputs
             SET disposition = 'rejected', decision_ref = 'authorization_revoked',
                 updated_at_ts = ?3
             WHERE input_id = ?1 AND owner_principal_id = ?2
               AND target_task_id IS NULL AND preparation_state = 'ready'
               AND disposition = 'pending'",
            params![input_id.to_string(), scope.owner_principal_id, now_ts],
        )
        .map_err(database_error)?;
    if changed == 1 {
        append_event(
            &tx,
            scope,
            input_id,
            "rejected",
            json!({
                "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
                "input_id": input_id,
                "error_code": "authorization_revoked",
                "retryable": false,
            }),
            now_ts,
        )?;
        tx.execute(
            "DELETE FROM conversation_input_task_templates WHERE input_id = ?1",
            [input_id.to_string()],
        )
        .map_err(database_error)?;
        delete_task_creation_claim(&tx, scope, input_id, claim_token)?;
    }
    tx.commit().map_err(database_error)?;
    Ok(changed == 1)
}

pub(crate) fn list_conversation_inputs(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    after_input_seq: u64,
    limit: u32,
) -> Result<Vec<ConversationInputRecord>, ConversationInputStoreError> {
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let after_input_seq = to_i64(after_input_seq)?;
    let limit = i64::from(limit.clamp(1, MAX_LIST_LIMIT));
    let mut statement = db
        .prepare(
            "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                    channel_account_id, conversation_id, content_json, delivery_mode,
                    preparation_state, disposition, expected_task_id,
                    expected_instruction_revision, target_task_id, decision_ref,
                    source_json, instruction_revision, execution_epoch,
                    accepted_at_ts, updated_at_ts
             FROM conversation_inputs
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5 AND input_seq > ?6
             ORDER BY input_seq ASC
             LIMIT ?7",
        )
        .map_err(database_error)?;
    let records = statement
        .query_map(
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                after_input_seq,
                limit,
            ],
            row_to_record,
        )
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(records)
}

pub(crate) fn list_conversation_input_events(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
    after_event_seq: u64,
    limit: u32,
) -> Result<Vec<ConversationInputEventRecord>, ConversationInputStoreError> {
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    let mut statement = db
        .prepare(
            "SELECT event_seq, input_id, event_kind, payload_json, created_at_ts
             FROM conversation_input_events
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5 AND event_seq > ?6
             ORDER BY event_seq ASC LIMIT ?7",
        )
        .map_err(database_error)?;
    let records = statement
        .query_map(
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                to_i64(after_event_seq)?,
                i64::from(limit.clamp(1, MAX_LIST_LIMIT)),
            ],
            |row| {
                Ok(ConversationInputEventRecord {
                    schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
                    event_seq: from_i64(row.get(0)?)?,
                    input_id: parse_uuid(row.get(1)?, 1)?,
                    event_kind: row.get(2)?,
                    payload: parse_json(row.get(3)?, 3)?,
                    created_at_ts: from_i64(row.get(4)?)?,
                })
            },
        )
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(records)
}
