fn task_is_active_and_owned(
    db: &Connection,
    task_id: &str,
    owner_principal_id: &str,
) -> Result<bool, ConversationInputStoreError> {
    db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM tasks
            WHERE task_id = ?1 AND principal_id = ?2 AND status IN ('queued', 'running')
              AND NOT EXISTS (
                  SELECT 1 FROM conversation_terminal_boundaries
                  WHERE conversation_terminal_boundaries.task_id = tasks.task_id
              )
         )",
        params![task_id, owner_principal_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .map_err(database_error)
}

pub(crate) fn conversation_input_owns_initial_task_delivery(
    pool: &DbPool,
    owner_principal_id: &str,
    input_id: Uuid,
) -> Result<bool, ConversationInputStoreError> {
    let db = pool
        .get()
        .context("conversation_input_db_pool_failed")
        .map_err(ConversationInputStoreError::Database)?;
    ensure_conversation_input_schema(&db).map_err(ConversationInputStoreError::Database)?;
    db.query_row(
        "SELECT COALESCE(applied_checkpoint_ref = 'initial_task_payload', 0)
         FROM conversation_inputs
         WHERE owner_principal_id = ?1 AND input_id = ?2",
        params![owner_principal_id, input_id.to_string()],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(database_error)?
    .map(|value| value != 0)
    .ok_or(ConversationInputStoreError::NotFound)
}

fn scope_for_record(
    db: &Connection,
    input_id: &Uuid,
) -> Result<OwnedConversationInputScope, ConversationInputStoreError> {
    db.query_row(
        "SELECT owner_principal_id, agent_id, channel, channel_account_id, conversation_id
         FROM conversation_inputs WHERE input_id = ?1",
        [input_id.to_string()],
        |row| {
            Ok(OwnedConversationInputScope {
                owner_principal_id: row.get(0)?,
                conversation: claw_core::conversation_input::ConversationInputScopeRef {
                    agent_id: row.get(1)?,
                    channel: row.get(2)?,
                    channel_account_id: row.get(3)?,
                    conversation_id: row.get(4)?,
                },
            })
        },
    )
    .optional()
    .map_err(database_error)?
    .ok_or(ConversationInputStoreError::NotFound)
}

fn append_event(
    tx: &Transaction<'_>,
    scope: &OwnedConversationInputScope,
    input_id: Uuid,
    event_kind: &str,
    payload: serde_json::Value,
    now_ts: i64,
) -> Result<u64, ConversationInputStoreError> {
    let event_seq = tx
        .query_row(
            "SELECT next_event_seq FROM conversation_input_scopes
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?;
    tx.execute(
        "INSERT INTO conversation_input_events(
            owner_principal_id, agent_id, channel, channel_account_id,
            conversation_id, event_seq, input_id, event_kind, payload_json, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            event_seq,
            input_id.to_string(),
            event_kind,
            payload.to_string(),
            now_ts,
        ],
    )
    .map_err(database_error)?;
    tx.execute(
        "UPDATE conversation_input_scopes
         SET next_event_seq = ?6, updated_at_ts = ?7
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            event_seq.saturating_add(1),
            now_ts,
        ],
    )
    .map_err(database_error)?;
    from_i64(event_seq).map_err(database_error)
}

fn load_task_records(
    db: &Connection,
    task_id: &str,
    disposition: &str,
    limit: u32,
) -> Result<Vec<ConversationInputRecord>, ConversationInputStoreError> {
    let mut statement = db
        .prepare(
            "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                    channel_account_id, conversation_id, content_json, delivery_mode,
                    preparation_state, disposition, expected_task_id,
                    expected_instruction_revision, target_task_id, decision_ref,
                    source_json, instruction_revision, execution_epoch,
                    accepted_at_ts, updated_at_ts
             FROM conversation_inputs
             WHERE target_task_id = ?1 AND disposition = ?2
               AND preparation_state = 'ready'
             ORDER BY input_seq ASC LIMIT ?3",
        )
        .map_err(database_error)?;
    let records = statement
        .query_map(
            params![task_id, disposition, i64::from(limit)],
            row_to_record,
        )
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(records)
}

fn load_task_records_after_seq(
    db: &Connection,
    task_id: &str,
    disposition: &str,
    checkpoint_ref: &str,
    after_input_seq: u64,
    limit: u32,
) -> Result<Vec<ConversationInputRecord>, ConversationInputStoreError> {
    let mut statement = db
        .prepare(
            "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                    channel_account_id, conversation_id, content_json, delivery_mode,
                    preparation_state, disposition, expected_task_id,
                    expected_instruction_revision, target_task_id, decision_ref,
                    source_json, instruction_revision, execution_epoch,
                    accepted_at_ts, updated_at_ts
             FROM conversation_inputs
             WHERE target_task_id = ?1 AND disposition = ?2
               AND applied_checkpoint_ref = ?3 AND input_seq > ?4
               AND COALESCE(decision_kind, '') NOT IN ('pause', 'stop')
             ORDER BY input_seq ASC LIMIT ?5",
        )
        .map_err(database_error)?;
    let records = statement
        .query_map(
            params![
                task_id,
                disposition,
                checkpoint_ref,
                to_i64(after_input_seq)?,
                i64::from(limit),
            ],
            row_to_record,
        )
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(records)
}

fn accept_conversation_input_in_db(
    db: &mut Connection,
    input: &AcceptConversationInput,
    now_ts: u64,
) -> Result<AcceptConversationInputOutcome, ConversationInputStoreError> {
    accept_conversation_input_in_db_with_resources(db, input, None, &[], now_ts)
}

fn accept_conversation_input_in_db_with_resources(
    db: &mut Connection,
    input: &AcceptConversationInput,
    template: Option<&ConversationInputTaskTemplate>,
    attachments: &[ConversationInputAttachmentBinding],
    now_ts: u64,
) -> Result<AcceptConversationInputOutcome, ConversationInputStoreError> {
    let request_digest = request_digest(&input.submission)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let scope = owned_scope(input);
    ensure_scope(&tx, &scope, now_ts)?;

    if let Some((existing, event_seq, existing_digest)) =
        load_by_client_message_id(&tx, &scope, &input.submission.client_message_id)?
    {
        if existing_digest != request_digest {
            return Err(ConversationInputStoreError::IdempotencyConflict);
        }
        if let Some(template) = template.filter(|_| {
            existing.receipt.target_task_id.is_none()
                && existing.delivery_mode == ConversationInputDeliveryMode::Auto
        }) {
            insert_conversation_input_task_template(
                &tx,
                &scope.owner_principal_id,
                existing.receipt.input_id,
                template,
                to_i64(now_ts)?,
            )?;
        }
        if !attachments.is_empty() {
            let expected_ids = existing
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
            insert_conversation_input_attachment_bindings(
                &tx,
                &scope,
                existing.receipt.input_id,
                &expected_ids,
                attachments,
                to_i64(now_ts)?,
            )?;
        }
        tx.commit().map_err(database_error)?;
        let mut replayed = existing;
        replayed.receipt.replayed = true;
        return Ok(AcceptConversationInputOutcome {
            record: replayed,
            event_seq,
        });
    }

    enforce_pending_capacity(&tx, &scope, request_content_bytes(&input.submission)?)?;

    let (input_seq, event_seq, mut focus_task_id, instruction_revision, mut execution_epoch) =
        load_scope_counters(&tx, &scope)?;
    if let Some(task_id) = focus_task_id.as_deref() {
        if !task_is_active_and_owned(&tx, task_id, &scope.owner_principal_id)? {
            execution_epoch = execution_epoch.checked_add(1).ok_or_else(|| {
                ConversationInputStoreError::Database(anyhow::anyhow!(
                    "conversation_input_epoch_overflow"
                ))
            })?;
            tx.execute(
                "UPDATE conversation_input_scopes
                 SET focus_task_id = NULL, execution_epoch = ?6, updated_at_ts = ?7
                 WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
                   AND channel_account_id = ?4 AND conversation_id = ?5",
                params![
                    scope.owner_principal_id,
                    scope.conversation.agent_id,
                    scope.conversation.channel,
                    scope.conversation.channel_account_id,
                    scope.conversation.conversation_id,
                    to_i64(execution_epoch)?,
                    to_i64(now_ts)?,
                ],
            )
            .map_err(database_error)?;
            focus_task_id = None;
        }
    }
    let expected_unfocused_task_is_owned =
        match (focus_task_id.as_deref(), input.submission.expected_task_id) {
            (None, Some(expected_task_id)) => {
                task_is_active_owned_and_scoped(&tx, &expected_task_id.to_string(), &scope)?
            }
            _ => false,
        };
    validate_expected_target(
        &input.submission,
        focus_task_id.as_deref(),
        instruction_revision,
        expected_unfocused_task_is_owned,
    )?;

    let input_id = Uuid::new_v4();
    let content_json = serde_json::to_string(&input.submission.content).map_err(json_error)?;
    let source_json = serde_json::to_string(&input.submission.source).map_err(json_error)?;
    let delivery_mode = delivery_mode_token(input.submission.delivery_mode);
    let disposition = match input.submission.delivery_mode {
        ConversationInputDeliveryMode::Auto => ConversationInputDisposition::Pending,
        ConversationInputDeliveryMode::Defer => ConversationInputDisposition::Deferred,
    };
    let input_seq_i64 = to_i64(input_seq)?;
    let event_seq_i64 = to_i64(event_seq)?;
    let instruction_revision_i64 = to_i64(instruction_revision)?;
    let execution_epoch_i64 = to_i64(execution_epoch)?;
    let now_i64 = to_i64(now_ts)?;
    tx.execute(
        "INSERT INTO conversation_inputs(
            input_id, owner_principal_id, agent_id, channel, channel_account_id,
            conversation_id, client_message_id, input_seq, request_digest,
            content_json, delivery_mode, preparation_state, disposition,
            expected_task_id, expected_instruction_revision, target_task_id,
            source_json, instruction_revision, execution_epoch, accepted_at_ts, updated_at_ts
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?20
         )",
        params![
            input_id.to_string(),
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            input.submission.client_message_id,
            input_seq_i64,
            request_digest,
            content_json,
            delivery_mode,
            preparation_state_token(input.preparation_state),
            disposition_token(disposition),
            input
                .submission
                .expected_task_id
                .map(|value| value.to_string()),
            input
                .submission
                .expected_instruction_revision
                .map(to_i64)
                .transpose()?,
            focus_task_id,
            source_json,
            instruction_revision_i64,
            execution_epoch_i64,
            now_i64,
        ],
    )
    .map_err(database_error)?;
    if !attachments.is_empty() {
        let expected_ids = input
            .submission
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
        insert_conversation_input_attachment_bindings(
            &tx,
            &scope,
            input_id,
            &expected_ids,
            attachments,
            now_i64,
        )?;
    }
    if let Some(template) = template.filter(|_| {
        focus_task_id.is_none() && input.submission.delivery_mode == ConversationInputDeliveryMode::Auto
    }) {
        insert_conversation_input_task_template(
            &tx,
            &scope.owner_principal_id,
            input_id,
            template,
            now_i64,
        )?;
    }
    let event_payload = serde_json::json!({
        "schema_version": CONVERSATION_INPUT_SCHEMA_VERSION,
        "input_id": input_id,
        "input_seq": input_seq,
        "preparation_state": input.preparation_state,
        "disposition": disposition,
        "target_task_id": focus_task_id,
    });
    tx.execute(
        "INSERT INTO conversation_input_events(
            owner_principal_id, agent_id, channel, channel_account_id,
            conversation_id, event_seq, input_id, event_kind, payload_json, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'accepted', ?8, ?9)",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            event_seq_i64,
            input_id.to_string(),
            event_payload.to_string(),
            now_i64,
        ],
    )
    .map_err(database_error)?;
    tx.execute(
        "UPDATE conversation_input_scopes
         SET next_input_seq = ?6, next_event_seq = ?7, updated_at_ts = ?8
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            input_seq_i64.saturating_add(1),
            event_seq_i64.saturating_add(1),
            now_i64,
        ],
    )
    .map_err(database_error)?;
    tx.commit().map_err(database_error)?;

    Ok(AcceptConversationInputOutcome {
        record: ConversationInputRecord {
            receipt: ConversationInputReceipt {
                schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
                input_id,
                client_message_id: input.submission.client_message_id.clone(),
                input_seq,
                scope: input.submission.scope.clone(),
                preparation_state: input.preparation_state,
                disposition,
                target_task_id: focus_task_id.and_then(|value| Uuid::parse_str(&value).ok()),
                decision_ref: None,
                instruction_revision,
                execution_epoch,
                accepted_at_ts: now_ts,
                updated_at_ts: now_ts,
                replayed: false,
            },
            content: input.submission.content.clone(),
            delivery_mode: input.submission.delivery_mode,
            expected_task_id: input.submission.expected_task_id,
            expected_instruction_revision: input.submission.expected_instruction_revision,
            source: input.submission.source.clone(),
        },
        event_seq,
    })
}

fn insert_conversation_input_task_template(
    db: &Connection,
    owner_principal_id: &str,
    input_id: Uuid,
    template: &ConversationInputTaskTemplate,
    now_ts: i64,
) -> Result<(), ConversationInputStoreError> {
    let template_json = serde_json::to_string(template).map_err(json_error)?;
    let template_digest = format!("{:x}", Sha256::digest(template_json.as_bytes()));
    db.execute(
        "INSERT OR IGNORE INTO conversation_input_task_templates(
            input_id, owner_principal_id, template_json, template_digest,
            next_attempt_at_ts, attempt_count, created_at_ts, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?5)",
        params![
            input_id.to_string(),
            owner_principal_id,
            template_json,
            template_digest,
            now_ts,
        ],
    )
    .map_err(database_error)?;
    let stored_digest = db
        .query_row(
            "SELECT template_digest FROM conversation_input_task_templates
             WHERE input_id = ?1 AND owner_principal_id = ?2",
            params![input_id.to_string(), owner_principal_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or(ConversationInputStoreError::TargetConflict)?;
    if stored_digest != template_digest {
        return Err(ConversationInputStoreError::IdempotencyConflict);
    }
    Ok(())
}

fn enforce_pending_capacity(
    db: &Connection,
    scope: &OwnedConversationInputScope,
    incoming_bytes: u64,
) -> Result<(), ConversationInputStoreError> {
    let (pending_count, pending_bytes) = db
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(content_json)), 0)
             FROM conversation_inputs
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5
               AND disposition IN ('pending', 'deferred', 'needs_clarification')",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| Ok((from_i64(row.get(0)?)?, from_i64(row.get(1)?)?)),
        )
        .map_err(database_error)?;
    if pending_count >= MAX_PENDING_INPUTS_PER_SCOPE
        || pending_bytes.saturating_add(incoming_bytes) > MAX_PENDING_INPUT_BYTES_PER_SCOPE
    {
        return Err(ConversationInputStoreError::CapacityExceeded);
    }
    Ok(())
}

fn validate_accept_input(
    input: &AcceptConversationInput,
) -> Result<(), ConversationInputStoreError> {
    input
        .submission
        .validate()
        .map_err(|_| ConversationInputStoreError::InvalidRequest)?;
    let scope = owned_scope(input);
    if !scope.validate() {
        return Err(ConversationInputStoreError::InvalidRequest);
    }
    Ok(())
}

fn owned_scope(input: &AcceptConversationInput) -> OwnedConversationInputScope {
    OwnedConversationInputScope {
        owner_principal_id: input.owner_principal_id.clone(),
        conversation: input.submission.scope.clone(),
    }
}

fn ensure_scope(
    tx: &Transaction<'_>,
    scope: &OwnedConversationInputScope,
    now_ts: u64,
) -> Result<(), ConversationInputStoreError> {
    let now_ts = to_i64(now_ts)?;
    tx.execute(
        "INSERT INTO conversation_input_scopes(
            owner_principal_id, agent_id, channel, channel_account_id,
            conversation_id, created_at_ts, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT(owner_principal_id, agent_id, channel, channel_account_id, conversation_id)
         DO NOTHING",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            now_ts,
        ],
    )
    .map_err(database_error)?;
    Ok(())
}

fn load_scope_counters(
    tx: &Transaction<'_>,
    scope: &OwnedConversationInputScope,
) -> Result<(u64, u64, Option<String>, u64, u64), ConversationInputStoreError> {
    tx.query_row(
        "SELECT next_input_seq, next_event_seq, focus_task_id,
                instruction_revision, execution_epoch
         FROM conversation_input_scopes
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
        ],
        |row| {
            Ok((
                from_i64(row.get(0)?)?,
                from_i64(row.get(1)?)?,
                row.get(2)?,
                from_i64(row.get(3)?)?,
                from_i64(row.get(4)?)?,
            ))
        },
    )
    .map_err(database_error)
}

fn validate_expected_target(
    submission: &ConversationInputSubmission,
    focus_task_id: Option<&str>,
    instruction_revision: u64,
    expected_unfocused_task_is_owned: bool,
) -> Result<(), ConversationInputStoreError> {
    if submission
        .expected_task_id
        .map(|value| value.to_string())
        .as_deref()
        != focus_task_id
        && submission.expected_task_id.is_some()
        && !expected_unfocused_task_is_owned
    {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    if submission
        .expected_instruction_revision
        .is_some_and(|expected| expected != instruction_revision)
    {
        return Err(ConversationInputStoreError::TargetConflict);
    }
    Ok(())
}

fn task_is_active_owned_and_scoped(
    db: &Connection,
    task_id: &str,
    scope: &OwnedConversationInputScope,
) -> Result<bool, ConversationInputStoreError> {
    let task = db
        .query_row(
            "SELECT principal_id, status, channel, payload_json
             FROM tasks WHERE task_id = ?1",
            [task_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((owner, status, channel, payload_json)) = task else {
        return Ok(false);
    };
    if owner.as_deref() != Some(scope.owner_principal_id.as_str())
        || !matches!(status.as_str(), "queued" | "running")
        || channel != scope.conversation.channel
    {
        return Ok(false);
    }
    let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
        return Ok(false);
    };
    let conversation_id = payload
        .get("conversation_id")
        .or_else(|| payload.get("thread_id"))
        .and_then(Value::as_str)
        .map(str::trim);
    let agent_id = payload
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("main");
    Ok(
        conversation_id == Some(scope.conversation.conversation_id.as_str())
            && agent_id == scope.conversation.agent_id,
    )
}

fn load_by_client_message_id(
    tx: &Transaction<'_>,
    scope: &OwnedConversationInputScope,
    client_message_id: &str,
) -> Result<Option<(ConversationInputRecord, u64, String)>, ConversationInputStoreError> {
    tx.query_row(
        "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                channel_account_id, conversation_id, content_json, delivery_mode,
                preparation_state, disposition, expected_task_id,
                expected_instruction_revision, target_task_id, decision_ref,
                source_json, instruction_revision, execution_epoch,
                accepted_at_ts, updated_at_ts, request_digest,
                (SELECT event_seq FROM conversation_input_events events
                 WHERE events.input_id = conversation_inputs.input_id
                 ORDER BY event_seq ASC LIMIT 1)
         FROM conversation_inputs
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5
           AND client_message_id = ?6",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            client_message_id,
        ],
        |row| {
            let record = row_to_record(row)?;
            let digest = row.get::<_, String>(20)?;
            let event_seq = from_i64(row.get::<_, i64>(21)?)?;
            Ok((record, event_seq, digest))
        },
    )
    .optional()
    .map_err(database_error)
}

fn load_record_by_input_id(
    db: &Connection,
    owner_principal_id: &str,
    input_id: &str,
) -> Result<Option<ConversationInputRecord>, ConversationInputStoreError> {
    db.query_row(
        "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                channel_account_id, conversation_id, content_json, delivery_mode,
                preparation_state, disposition, expected_task_id,
                expected_instruction_revision, target_task_id, decision_ref,
                source_json, instruction_revision, execution_epoch,
                accepted_at_ts, updated_at_ts
         FROM conversation_inputs
         WHERE owner_principal_id = ?1 AND input_id = ?2",
        params![owner_principal_id, input_id],
        row_to_record,
    )
    .optional()
    .map_err(database_error)
}

fn load_record_by_client_message_id(
    db: &Connection,
    scope: &OwnedConversationInputScope,
    client_message_id: &str,
) -> Result<Option<ConversationInputRecord>, ConversationInputStoreError> {
    db.query_row(
        "SELECT input_id, client_message_id, input_seq, agent_id, channel,
                channel_account_id, conversation_id, content_json, delivery_mode,
                preparation_state, disposition, expected_task_id,
                expected_instruction_revision, target_task_id, decision_ref,
                source_json, instruction_revision, execution_epoch,
                accepted_at_ts, updated_at_ts
         FROM conversation_inputs
         WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
           AND channel_account_id = ?4 AND conversation_id = ?5
           AND client_message_id = ?6",
        params![
            scope.owner_principal_id,
            scope.conversation.agent_id,
            scope.conversation.channel,
            scope.conversation.channel_account_id,
            scope.conversation.conversation_id,
            client_message_id,
        ],
        row_to_record,
    )
    .optional()
    .map_err(database_error)
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationInputRecord> {
    let input_id = parse_uuid(row.get::<_, String>(0)?, 0)?;
    let input_seq = from_i64(row.get(2)?)?;
    let content = parse_json(row.get::<_, String>(7)?, 7)?;
    let delivery_mode = parse_delivery_mode(&row.get::<_, String>(8)?, 8)?;
    let preparation_state = parse_preparation_state(&row.get::<_, String>(9)?, 9)?;
    let disposition = parse_disposition(&row.get::<_, String>(10)?, 10)?;
    let expected_task_id = parse_optional_uuid(row.get(11)?, 11)?;
    let expected_instruction_revision = optional_from_i64(row.get(12)?)?;
    let target_task_id = parse_optional_uuid(row.get(13)?, 13)?;
    let source: ConversationInputSource = parse_json(row.get::<_, String>(15)?, 15)?;
    Ok(ConversationInputRecord {
        receipt: ConversationInputReceipt {
            schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
            input_id,
            client_message_id: row.get(1)?,
            input_seq,
            scope: claw_core::conversation_input::ConversationInputScopeRef {
                agent_id: row.get(3)?,
                channel: row.get(4)?,
                channel_account_id: row.get(5)?,
                conversation_id: row.get(6)?,
            },
            preparation_state,
            disposition,
            target_task_id,
            decision_ref: row.get(14)?,
            instruction_revision: from_i64(row.get(16)?)?,
            execution_epoch: from_i64(row.get(17)?)?,
            accepted_at_ts: from_i64(row.get(18)?)?,
            updated_at_ts: from_i64(row.get(19)?)?,
            replayed: false,
        },
        content,
        delivery_mode,
        expected_task_id,
        expected_instruction_revision,
        source,
    })
}

fn request_digest(
    submission: &ConversationInputSubmission,
) -> Result<String, ConversationInputStoreError> {
    let encoded = serde_json::to_vec(submission).map_err(json_error)?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn request_content_bytes(
    submission: &ConversationInputSubmission,
) -> Result<u64, ConversationInputStoreError> {
    let encoded = serde_json::to_vec(&submission.content).map_err(json_error)?;
    u64::try_from(encoded.len()).map_err(|error| {
        ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_input_content_size_invalid:{error}"
        ))
    })
}

fn migration_digest(db: &Connection, migration_id: &str) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
        [migration_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn verify_or_record_migration(
    db: &Connection,
    migration_id: &str,
    manifest: &str,
) -> anyhow::Result<()> {
    let manifest_digest = format!("sha256:{:x}", Sha256::digest(manifest.as_bytes()));
    if let Some(applied_digest) = migration_digest(db, migration_id)? {
        anyhow::ensure!(
            applied_digest == manifest_digest,
            "runtime_schema_migration_digest_mismatch:{migration_id}"
        );
    }
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(migration_id) DO NOTHING",
        params![migration_id, manifest_digest, crate::now_ts()],
    )?;
    Ok(())
}

fn delivery_mode_token(value: ConversationInputDeliveryMode) -> &'static str {
    match value {
        ConversationInputDeliveryMode::Auto => "auto",
        ConversationInputDeliveryMode::Defer => "defer",
    }
}

fn preparation_state_token(value: ConversationInputPreparationState) -> &'static str {
    match value {
        ConversationInputPreparationState::Pending => "pending",
        ConversationInputPreparationState::Ready => "ready",
        ConversationInputPreparationState::Failed => "failed",
    }
}

fn disposition_token(value: ConversationInputDisposition) -> &'static str {
    match value {
        ConversationInputDisposition::Pending => "pending",
        ConversationInputDisposition::Deferred => "deferred",
        ConversationInputDisposition::NeedsClarification => "needs_clarification",
        ConversationInputDisposition::Applied => "applied",
        ConversationInputDisposition::Rejected => "rejected",
        ConversationInputDisposition::Withdrawn => "withdrawn",
    }
}

fn parse_delivery_mode(
    value: &str,
    column: usize,
) -> rusqlite::Result<ConversationInputDeliveryMode> {
    match value {
        "auto" => Ok(ConversationInputDeliveryMode::Auto),
        "defer" => Ok(ConversationInputDeliveryMode::Defer),
        _ => Err(invalid_column(
            column,
            "invalid conversation input delivery mode",
        )),
    }
}

fn parse_preparation_state(
    value: &str,
    column: usize,
) -> rusqlite::Result<ConversationInputPreparationState> {
    match value {
        "pending" => Ok(ConversationInputPreparationState::Pending),
        "ready" => Ok(ConversationInputPreparationState::Ready),
        "failed" => Ok(ConversationInputPreparationState::Failed),
        _ => Err(invalid_column(
            column,
            "invalid conversation input preparation state",
        )),
    }
}

fn parse_disposition(value: &str, column: usize) -> rusqlite::Result<ConversationInputDisposition> {
    match value {
        "pending" => Ok(ConversationInputDisposition::Pending),
        "deferred" => Ok(ConversationInputDisposition::Deferred),
        "needs_clarification" => Ok(ConversationInputDisposition::NeedsClarification),
        "applied" => Ok(ConversationInputDisposition::Applied),
        "rejected" => Ok(ConversationInputDisposition::Rejected),
        "withdrawn" => Ok(ConversationInputDisposition::Withdrawn),
        _ => Err(invalid_column(
            column,
            "invalid conversation input disposition",
        )),
    }
}

fn parse_uuid(value: String, column: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value).map_err(|error| invalid_column(column, error))
}

fn parse_optional_uuid(value: Option<String>, column: usize) -> rusqlite::Result<Option<Uuid>> {
    value.map(|value| parse_uuid(value, column)).transpose()
}

fn parse_json<T: serde::de::DeserializeOwned>(value: String, column: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|error| invalid_column(column, error))
}

fn to_i64(value: u64) -> Result<i64, ConversationInputStoreError> {
    i64::try_from(value).map_err(|error| {
        ConversationInputStoreError::Database(anyhow::anyhow!(
            "conversation_input_integer_overflow:{error}"
        ))
    })
}

fn from_i64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| invalid_column(0, error))
}

fn optional_from_i64(value: Option<i64>) -> rusqlite::Result<Option<u64>> {
    value.map(from_i64).transpose()
}

fn invalid_column(column: usize, error: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn database_error(error: rusqlite::Error) -> ConversationInputStoreError {
    ConversationInputStoreError::Database(error.into())
}

fn json_error(error: serde_json::Error) -> ConversationInputStoreError {
    ConversationInputStoreError::Database(error.into())
}

fn valid_machine_token(value: &str, max_chars: usize) -> bool {
    let trimmed = value.trim();
    value == trimmed
        && !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '@'))
}
