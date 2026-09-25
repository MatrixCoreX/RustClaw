pub(crate) fn active_conversation_task(
    pool: &DbPool,
    scope: &OwnedConversationInputScope,
) -> Result<Option<Uuid>, ConversationInputStoreError> {
    if !scope.validate() {
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
    let focus_task_id = tx
        .query_row(
            "SELECT focus_task_id FROM conversation_input_scopes
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
            ],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(database_error)?
        .flatten();
    let active = match focus_task_id.as_deref() {
        Some(task_id) => task_is_active_and_owned(&tx, task_id, &scope.owner_principal_id)?,
        None => false,
    };
    if !active && focus_task_id.is_some() {
        tx.execute(
            "UPDATE conversation_input_scopes
             SET focus_task_id = NULL, execution_epoch = execution_epoch + 1,
                 updated_at_ts = ?6
             WHERE owner_principal_id = ?1 AND agent_id = ?2 AND channel = ?3
               AND channel_account_id = ?4 AND conversation_id = ?5",
            params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                to_i64(crate::now_ts_u64())?,
            ],
        )
        .map_err(database_error)?;
    }
    tx.commit().map_err(database_error)?;
    if active {
        focus_task_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()
            .map_err(|error| ConversationInputStoreError::Database(error.into()))
    } else {
        Ok(None)
    }
}
