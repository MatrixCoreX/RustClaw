use claw_core::conversation_input::{
    ConversationInputContent, ConversationInputDeliveryMode, ConversationInputDisposition,
    ConversationInputPreparationState, ConversationInputScopeRef, ConversationInputSource,
    ConversationInputSubmission, OwnedConversationInputScope, CONVERSATION_INPUT_SCHEMA_VERSION,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use uuid::Uuid;

use super::*;

#[path = "conversation_inputs_tests/fault_injection.rs"]
mod fault_injection;

fn submission(client_message_id: &str) -> ConversationInputSubmission {
    ConversationInputSubmission {
        schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
        client_message_id: client_message_id.to_string(),
        scope: ConversationInputScopeRef {
            conversation_id: "conversation-1".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "web-session".to_string(),
        },
        content: vec![ConversationInputContent::Text {
            text: "Keep the existing result and change the format.".to_string(),
        }],
        delivery_mode: ConversationInputDeliveryMode::Auto,
        expected_task_id: None,
        expected_instruction_revision: Some(0),
        source: ConversationInputSource::default(),
    }
}

fn input(client_message_id: &str) -> AcceptConversationInput {
    AcceptConversationInput {
        owner_principal_id: "principal-1".to_string(),
        submission: submission(client_message_id),
        preparation_state: ConversationInputPreparationState::Ready,
    }
}

fn scope(owner_principal_id: &str) -> OwnedConversationInputScope {
    OwnedConversationInputScope {
        owner_principal_id: owner_principal_id.to_string(),
        conversation: submission("unused").scope,
    }
}

fn database() -> Connection {
    let database = Connection::open_in_memory().expect("open database");
    ensure_conversation_input_schema(&database).expect("ensure schema");
    database
        .execute_batch(
            "CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                principal_id TEXT,
                status TEXT NOT NULL,
                channel TEXT NOT NULL DEFAULT 'ui',
                external_user_id TEXT,
                external_chat_id TEXT,
                payload_json TEXT NOT NULL DEFAULT '{}',
                result_json TEXT,
                updated_at TEXT
             );
             CREATE TABLE auth_keys (
                user_key TEXT PRIMARY KEY,
                principal_id TEXT,
                enabled INTEGER NOT NULL
             );
             CREATE TABLE channel_bindings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel TEXT NOT NULL,
                external_user_id TEXT,
                external_chat_id TEXT,
                user_key TEXT NOT NULL
             );
             INSERT INTO auth_keys(user_key, principal_id, enabled)
             VALUES ('key-principal-1', 'principal-1', 1);",
        )
        .expect("create tasks table");
    database
}

fn pool() -> Pool<SqliteConnectionManager> {
    let pool = Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .expect("build pool");
    let database = pool.get().expect("get connection");
    ensure_conversation_input_schema(&database).expect("ensure schema");
    database
        .execute_batch(
            "CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                principal_id TEXT,
                status TEXT NOT NULL,
                channel TEXT NOT NULL DEFAULT 'ui',
                external_user_id TEXT,
                external_chat_id TEXT,
                payload_json TEXT NOT NULL DEFAULT '{}',
                result_json TEXT,
                updated_at TEXT
             );
             CREATE TABLE auth_keys (
                user_key TEXT PRIMARY KEY,
                principal_id TEXT,
                enabled INTEGER NOT NULL
             );
             CREATE TABLE channel_bindings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel TEXT NOT NULL,
                external_user_id TEXT,
                external_chat_id TEXT,
                user_key TEXT NOT NULL
             );
             INSERT INTO auth_keys(user_key, principal_id, enabled)
             VALUES ('key-principal-1', 'principal-1', 1);",
        )
        .expect("create tasks table");
    drop(database);
    pool
}

#[test]
fn schema_migration_is_idempotent_and_digest_bound() {
    let database = database();
    ensure_conversation_input_schema(&database).expect("repeat migration");
    let count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id = ?1",
            [MIGRATION_ID],
            |row| row.get(0),
        )
        .expect("migration count");
    assert_eq!(count, 1);
    let action_dispatch_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id = ?1",
            [ACTION_DISPATCH_MIGRATION_ID],
            |row| row.get(0),
        )
        .expect("action dispatch migration count");
    assert_eq!(action_dispatch_count, 1);
    let terminal_boundary_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id = ?1",
            [TERMINAL_BOUNDARY_MIGRATION_ID],
            |row| row.get(0),
        )
        .expect("terminal boundary migration count");
    assert_eq!(terminal_boundary_count, 1);

    database
        .execute(
            "UPDATE runtime_schema_migrations SET schema_digest = 'sha256:wrong'
             WHERE migration_id = ?1",
            [MIGRATION_ID],
        )
        .expect("poison digest");
    let error = ensure_conversation_input_schema(&database).expect_err("digest drift rejected");
    assert!(error
        .to_string()
        .contains("runtime_schema_migration_digest_mismatch"));
}

#[test]
fn same_client_message_replays_the_original_receipt() {
    let mut database = database();
    let request = input("message-1");
    let first =
        accept_conversation_input_in_db(&mut database, &request, 100).expect("accept first input");
    let replay =
        accept_conversation_input_in_db(&mut database, &request, 200).expect("replay input");

    assert_eq!(
        first.record.receipt.input_id,
        replay.record.receipt.input_id
    );
    assert_eq!(first.record.receipt.input_seq, 1);
    assert_eq!(replay.record.receipt.input_seq, 1);
    assert!(!first.record.receipt.replayed);
    assert!(replay.record.receipt.replayed);
    assert_eq!(first.event_seq, replay.event_seq);
}

#[test]
fn conversation_events_are_cursor_paged_and_owner_scoped() {
    let pool = pool();
    accept_conversation_input(&pool, &input("message-1")).expect("accept first input");
    accept_conversation_input(&pool, &input("message-2")).expect("accept second input");

    let first = list_conversation_input_events(&pool, &scope("principal-1"), 0, 1)
        .expect("first event page");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].event_seq, 1);
    assert_eq!(first[0].event_kind, "accepted");
    let second =
        list_conversation_input_events(&pool, &scope("principal-1"), first[0].event_seq, 10)
            .expect("second event page");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].event_seq, 2);
    assert!(
        list_conversation_input_events(&pool, &scope("principal-2"), 0, 10)
            .expect("other owner page")
            .is_empty()
    );
}

#[test]
fn same_id_with_changed_payload_is_rejected() {
    let mut database = database();
    let first = input("message-1");
    accept_conversation_input_in_db(&mut database, &first, 100).expect("accept first input");

    let mut changed = first;
    changed.submission.content = vec![ConversationInputContent::Text {
        text: "A different instruction.".to_string(),
    }];
    let error = accept_conversation_input_in_db(&mut database, &changed, 101)
        .expect_err("payload conflict");
    assert!(matches!(
        error,
        ConversationInputStoreError::IdempotencyConflict
    ));
}

#[test]
fn pending_input_capacity_rejects_new_ids_but_preserves_idempotent_replay() {
    let mut database = database();
    let first = input("message-0");
    let first_receipt =
        accept_conversation_input_in_db(&mut database, &first, 100).expect("accept first input");
    for index in 1..MAX_PENDING_INPUTS_PER_SCOPE {
        let request = input(&format!("message-{index}"));
        accept_conversation_input_in_db(&mut database, &request, 100 + index)
            .expect("accept within capacity");
    }
    let overflow = input("message-overflow");
    let error = accept_conversation_input_in_db(&mut database, &overflow, 1_000)
        .expect_err("reject over capacity");
    assert!(matches!(
        error,
        ConversationInputStoreError::CapacityExceeded
    ));
    assert_eq!(error.code(), ConversationInputErrorCode::CapacityExceeded);

    let replay = accept_conversation_input_in_db(&mut database, &first, 1_001)
        .expect("replay remains available at capacity");
    assert_eq!(
        replay.record.receipt.input_id,
        first_receipt.record.receipt.input_id
    );
    assert!(replay.record.receipt.replayed);
}

#[test]
fn identical_text_with_new_id_preserves_both_inputs_in_order() {
    let mut database = database();
    let first = accept_conversation_input_in_db(&mut database, &input("message-1"), 100)
        .expect("first input");
    let second = accept_conversation_input_in_db(&mut database, &input("message-2"), 101)
        .expect("second input");

    assert_eq!(first.record.receipt.input_seq, 1);
    assert_eq!(second.record.receipt.input_seq, 2);
    assert_ne!(
        first.record.receipt.input_id,
        second.record.receipt.input_id
    );
    let records = list_in_db(&database, &scope("principal-1"), 0, 10);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].receipt.client_message_id, "message-1");
    assert_eq!(records[1].receipt.client_message_id, "message-2");
}

#[test]
fn principal_and_conversation_scopes_have_independent_sequences() {
    let mut database = database();
    let principal_one = accept_conversation_input_in_db(&mut database, &input("message-1"), 100)
        .expect("principal one");
    let mut second = input("message-1");
    second.owner_principal_id = "principal-2".to_string();
    let principal_two =
        accept_conversation_input_in_db(&mut database, &second, 101).expect("principal two");

    assert_eq!(principal_one.record.receipt.input_seq, 1);
    assert_eq!(principal_two.record.receipt.input_seq, 1);
    assert_ne!(
        principal_one.record.receipt.input_id,
        principal_two.record.receipt.input_id
    );
}

#[test]
fn expected_revision_and_task_are_checked_only_for_new_inputs() {
    let mut database = database();
    let expected_task = Uuid::new_v4();
    database
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![expected_task.to_string(), "principal-1"],
        )
        .expect("insert active task");
    database
        .execute(
            "INSERT INTO conversation_input_scopes(
                owner_principal_id, agent_id, channel, channel_account_id,
                conversation_id, focus_task_id, instruction_revision,
                execution_epoch, created_at_ts, updated_at_ts
             ) VALUES ('principal-1', 'main', 'ui', 'web-session',
                'conversation-1', ?1, 4, 2, 1, 1)",
            [expected_task.to_string()],
        )
        .expect("insert scope");

    let mut accepted = input("message-1");
    accepted.submission.expected_task_id = Some(expected_task);
    accepted.submission.expected_instruction_revision = Some(4);
    let first =
        accept_conversation_input_in_db(&mut database, &accepted, 100).expect("matching target");
    assert_eq!(first.record.receipt.target_task_id, Some(expected_task));
    assert_eq!(first.record.receipt.instruction_revision, 4);
    assert_eq!(first.record.receipt.execution_epoch, 2);

    database
        .execute(
            "UPDATE conversation_input_scopes SET instruction_revision = 5
             WHERE owner_principal_id = 'principal-1'",
            [],
        )
        .expect("advance revision");
    let replay = accept_conversation_input_in_db(&mut database, &accepted, 101)
        .expect("replay ignores later revision");
    assert!(replay.record.receipt.replayed);

    let mut stale = accepted;
    stale.submission.client_message_id = "message-2".to_string();
    let error = accept_conversation_input_in_db(&mut database, &stale, 102)
        .expect_err("stale revision rejected");
    assert!(matches!(error, ConversationInputStoreError::TargetConflict));
}

#[test]
fn deferred_input_is_not_recorded_as_pending_execution() {
    let mut database = database();
    let mut deferred = input("message-1");
    deferred.submission.delivery_mode = ConversationInputDeliveryMode::Defer;
    let accepted = accept_conversation_input_in_db(&mut database, &deferred, 100)
        .expect("accept deferred input");
    assert_eq!(
        accepted.record.receipt.disposition,
        ConversationInputDisposition::Deferred
    );
}

#[test]
fn concurrent_idle_inputs_share_one_task_creation_claim_and_focus_task() {
    let pool = pool();
    let first = accept_conversation_input(&pool, &input("message-1")).expect("accept first");
    let mut second_input = input("message-2");
    second_input.submission.expected_instruction_revision = None;
    let second = accept_conversation_input(&pool, &second_input).expect("accept second");

    let creator = claim_or_bind_conversation_input_task(
        &pool,
        &scope("principal-1"),
        first.record.receipt.input_id,
    )
    .expect("claim creator");
    let claim_token = match creator {
        ConversationInputTaskClaimOutcome::Creator {
            record,
            claim_token,
        } if record.receipt.input_id == first.record.receipt.input_id => claim_token,
        outcome => panic!("unexpected claim outcome: {outcome:?}"),
    };
    assert_eq!(
        claim_or_bind_conversation_input_task(
            &pool,
            &scope("principal-1"),
            first.record.receipt.input_id,
        )
        .expect("same input observes active lease"),
        ConversationInputTaskClaimOutcome::Waiting
    );
    assert_eq!(
        claim_or_bind_conversation_input_task(
            &pool,
            &scope("principal-1"),
            second.record.receipt.input_id,
        )
        .expect("second waits"),
        ConversationInputTaskClaimOutcome::Waiting
    );

    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert one task");
    complete_conversation_input_task_claim(
        &pool,
        &scope("principal-1"),
        first.record.receipt.input_id,
        claim_token,
        task_id,
    )
    .expect("complete claim");

    let first_record = get_conversation_input(&pool, "principal-1", first.record.receipt.input_id)
        .expect("first record");
    let second_record =
        get_conversation_input(&pool, "principal-1", second.record.receipt.input_id)
            .expect("second record");
    assert_eq!(first_record.receipt.target_task_id, Some(task_id));
    assert_eq!(
        first_record.receipt.disposition,
        ConversationInputDisposition::Applied
    );
    assert_eq!(second_record.receipt.target_task_id, Some(task_id));
    assert_eq!(
        second_record.receipt.disposition,
        ConversationInputDisposition::Pending
    );
    assert!(matches!(
        claim_or_bind_conversation_input_task(
            &pool,
            &scope("principal-1"),
            second.record.receipt.input_id,
        )
        .expect("second observes binding"),
        ConversationInputTaskClaimOutcome::Bound(record)
            if record.receipt.target_task_id == Some(task_id)
    ));
    let remaining_claims: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM conversation_input_task_claims",
            [],
            |row| row.get(0),
        )
        .expect("claim count");
    assert_eq!(remaining_claims, 0);
}

#[test]
fn expired_task_creation_claim_is_taken_over_for_the_oldest_input() {
    let pool = pool();
    let first = accept_conversation_input(&pool, &input("message-1")).expect("accept first");
    let mut second_input = input("message-2");
    second_input.submission.expected_instruction_revision = None;
    let second = accept_conversation_input(&pool, &second_input).expect("accept second");
    let stale_token = match claim_or_bind_conversation_input_task(
        &pool,
        &scope("principal-1"),
        first.record.receipt.input_id,
    )
    .expect("first claim")
    {
        ConversationInputTaskClaimOutcome::Creator { claim_token, .. } => claim_token,
        outcome => panic!("unexpected first claim outcome: {outcome:?}"),
    };
    pool.get()
        .unwrap()
        .execute(
            "UPDATE conversation_input_task_claims SET expires_at_ts = 0",
            [],
        )
        .expect("expire claim");

    let takeover = claim_or_bind_conversation_input_task(
        &pool,
        &scope("principal-1"),
        second.record.receipt.input_id,
    )
    .expect("take over claim");
    let replacement_token = match takeover {
        ConversationInputTaskClaimOutcome::Creator {
            record,
            claim_token,
        } if record.receipt.input_id == first.record.receipt.input_id => claim_token,
        outcome => panic!("unexpected takeover outcome: {outcome:?}"),
    };
    assert_ne!(stale_token, replacement_token);

    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert replacement task");
    assert!(matches!(
        complete_conversation_input_task_claim(
            &pool,
            &scope("principal-1"),
            first.record.receipt.input_id,
            stale_token,
            task_id,
        ),
        Err(ConversationInputStoreError::TargetConflict)
    ));
    assert!(!release_conversation_input_task_claim(
        &pool,
        &scope("principal-1"),
        first.record.receipt.input_id,
        stale_token,
    )
    .expect("stale release is fenced"));
    complete_conversation_input_task_claim(
        &pool,
        &scope("principal-1"),
        first.record.receipt.input_id,
        replacement_token,
        task_id,
    )
    .expect("replacement claim completes");
}

#[test]
fn initial_task_binding_and_later_input_have_distinct_restore_semantics() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert active task");
    let bound = bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");
    assert_eq!(
        bound.receipt.disposition,
        ConversationInputDisposition::Applied
    );
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).unwrap(),
        Some(task_id)
    );

    let mut followup_input = input("message-2");
    followup_input.submission.expected_instruction_revision = None;
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept follow-up input");
    assert_eq!(followup.record.receipt.target_task_id, Some(task_id));
    assert_eq!(
        followup.record.receipt.disposition,
        ConversationInputDisposition::Pending
    );
    assert!(task_has_pending_conversation_inputs(&pool, &task_id.to_string()).unwrap());

    let applied = apply_pending_conversation_inputs(&pool, &task_id.to_string(), 32)
        .expect("apply pending input");
    assert_eq!(applied.len(), 1);
    assert_eq!(
        applied[0].receipt.input_id,
        followup.record.receipt.input_id
    );
    assert_eq!(
        applied[0].receipt.disposition,
        ConversationInputDisposition::Applied
    );
    assert_eq!(applied[0].receipt.instruction_revision, 2);
    assert_eq!(applied[0].receipt.execution_epoch, 2);
    assert!(!task_has_pending_conversation_inputs(&pool, &task_id.to_string()).unwrap());

    let restored = applied_conversation_inputs_for_task(&pool, &task_id.to_string(), 0, 100)
        .expect("restore loop inputs");
    assert_eq!(restored.len(), 1);
    assert_eq!(
        restored[0].receipt.input_id,
        followup.record.receipt.input_id
    );

    let decision_ref = format!(
        "planner:{task_id}:2:{}:continue_or_amend",
        applied[0].receipt.instruction_revision
    );
    assert_eq!(
        record_conversation_input_decision(
            &pool,
            &task_id.to_string(),
            applied[0].receipt.instruction_revision,
            "continue_or_amend",
            &decision_ref,
        )
        .expect("record planner decision"),
        1
    );
    let decided = get_conversation_input(&pool, "principal-1", followup.record.receipt.input_id)
        .expect("decided follow-up");
    assert_eq!(
        decided.receipt.decision_ref.as_deref(),
        Some(decision_ref.as_str())
    );
    assert_eq!(
        pool.get()
            .unwrap()
            .query_row(
                "SELECT decision_kind FROM conversation_inputs WHERE input_id = ?1",
                [followup.record.receipt.input_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .expect("decision kind"),
        "continue_or_amend"
    );
    assert_eq!(
        applied_conversation_inputs_for_task(&pool, &task_id.to_string(), 0, 100)
            .expect("restore substantive decided input")
            .len(),
        1
    );
    let original = get_conversation_input(&pool, "principal-1", initial.record.receipt.input_id)
        .expect("initial input");
    assert!(original.receipt.decision_ref.is_none());
    assert_eq!(
        record_conversation_input_decision(
            &pool,
            &task_id.to_string(),
            applied[0].receipt.instruction_revision,
            "continue_or_amend",
            &decision_ref,
        )
        .expect("decision replay"),
        0
    );
}

#[test]
fn checkpoint_restore_omits_completed_lifecycle_control_inputs() {
    let pool = pool();
    let initial = accept_conversation_input(&pool, &input("pause-restore-initial"))
        .expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert active task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");

    let mut pause_input = input("pause-restore-control");
    pause_input.submission.expected_instruction_revision = None;
    accept_conversation_input(&pool, &pause_input).expect("accept pause input");
    let applied = apply_pending_conversation_inputs(&pool, &task_id.to_string(), 32)
        .expect("apply pause input");
    assert_eq!(applied.len(), 1);
    let decision_ref = format!(
        "planner:{task_id}:2:{}:pause",
        applied[0].receipt.instruction_revision
    );
    assert_eq!(
        record_conversation_input_decision(
            &pool,
            &task_id.to_string(),
            applied[0].receipt.instruction_revision,
            "pause",
            &decision_ref,
        )
        .expect("record pause decision"),
        1
    );

    assert!(
        applied_conversation_inputs_for_task(&pool, &task_id.to_string(), 0, 100)
            .expect("restore after pause")
            .is_empty(),
        "an already executed pause must not be replayed as current user input"
    );
}

#[test]
fn pending_input_is_rejected_when_the_owner_has_no_enabled_credential() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("auth-revoked-initial")).expect("accept initial");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial");
    let mut followup_input = input("auth-revoked-followup");
    followup_input.submission.expected_instruction_revision = None;
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept pending follow-up");
    pool.get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled = 0 WHERE principal_id = 'principal-1'",
            [],
        )
        .expect("revoke credentials");

    assert!(matches!(
        apply_pending_conversation_inputs(&pool, &task_id.to_string(), 32),
        Err(ConversationInputStoreError::AuthorizationRevoked)
    ));
    let rejected = get_conversation_input(&pool, "principal-1", followup.record.receipt.input_id)
        .expect("rejected input");
    assert_eq!(
        rejected.receipt.disposition,
        ConversationInputDisposition::Rejected
    );
    assert_eq!(
        rejected.receipt.decision_ref.as_deref(),
        Some("authorization_revoked")
    );
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).expect("cleared focus"),
        None
    );
}

#[test]
fn credential_rotation_preserves_pending_input_for_the_same_principal() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("auth-rotation-initial")).expect("accept initial");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial");
    let mut followup_input = input("auth-rotation-followup");
    followup_input.submission.expected_instruction_revision = None;
    accept_conversation_input(&pool, &followup_input).expect("accept follow-up");
    pool.get()
        .unwrap()
        .execute_batch(
            "UPDATE auth_keys SET enabled = 0 WHERE user_key = 'key-principal-1';
             INSERT INTO auth_keys(user_key, principal_id, enabled)
             VALUES ('rotated-key-principal-1', 'principal-1', 1);",
        )
        .expect("rotate credential");

    let applied = apply_pending_conversation_inputs(&pool, &task_id.to_string(), 32)
        .expect("rotated principal stays authorized");
    assert_eq!(applied.len(), 1);
    assert_eq!(
        applied[0].receipt.disposition,
        ConversationInputDisposition::Applied
    );
}

#[test]
fn removing_a_channel_binding_rejects_pending_external_input() {
    let pool = pool();
    let mut initial_input = input("channel-revoked-initial");
    initial_input.submission.scope.channel = "wechat".to_string();
    initial_input.submission.scope.channel_account_id = "wechat-account-1".to_string();
    initial_input.submission.scope.conversation_id = "wechat-chat-1".to_string();
    let external_scope = OwnedConversationInputScope {
        owner_principal_id: "principal-1".to_string(),
        conversation: initial_input.submission.scope.clone(),
    };
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(
                task_id, principal_id, status, channel, external_user_id,
                external_chat_id, payload_json
             ) VALUES (?1, ?2, 'running', 'wechat', ?3, ?4, ?5)",
            rusqlite::params![
                task_id.to_string(),
                "principal-1",
                "wechat-user-1",
                "wechat-chat-1",
                serde_json::json!({
                    "conversation_id": "wechat-chat-1",
                    "agent_id": "main",
                    "channel_ingress": { "account_id": "wechat-account-1" }
                })
                .to_string(),
            ],
        )
        .expect("insert external task");
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO channel_bindings(
                channel, external_user_id, external_chat_id, user_key
             ) VALUES ('wechat', ?1, ?2, 'key-principal-1')",
            rusqlite::params!["wechat-user-1", "wechat-chat-1"],
        )
        .expect("insert channel binding");
    let initial =
        accept_conversation_input(&pool, &initial_input).expect("accept external initial input");
    bind_conversation_input_to_task(
        &pool,
        &external_scope,
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind external initial input");
    let mut followup_input = initial_input;
    followup_input.submission.client_message_id = "channel-revoked-followup".to_string();
    followup_input.submission.expected_instruction_revision = None;
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept external follow-up");
    pool.get()
        .unwrap()
        .execute("DELETE FROM channel_bindings WHERE channel = 'wechat'", [])
        .expect("remove channel binding");

    assert!(matches!(
        apply_pending_conversation_inputs(&pool, &task_id.to_string(), 32),
        Err(ConversationInputStoreError::AuthorizationRevoked)
    ));
    let rejected = get_conversation_input(&pool, "principal-1", followup.record.receipt.input_id)
        .expect("rejected external input");
    assert_eq!(
        rejected.receipt.disposition,
        ConversationInputDisposition::Rejected
    );
    assert_eq!(
        active_conversation_task(&pool, &external_scope).expect("cleared external focus"),
        None
    );
}

#[test]
fn existing_task_adoption_rejects_a_task_that_finished_before_binding() {
    let pool = pool();
    let accepted = accept_conversation_input(&pool, &input("message-1")).expect("accept input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'succeeded')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert terminal task");

    let error = bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        accepted.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::ExistingActiveTask,
    )
    .expect_err("terminal existing task cannot be adopted");

    assert!(matches!(error, ConversationInputStoreError::TargetConflict));
    let stored = get_conversation_input(&pool, "principal-1", accepted.record.receipt.input_id)
        .expect("stored input remains unbound");
    assert_eq!(stored.receipt.target_task_id, None);
    assert_eq!(
        stored.receipt.disposition,
        ConversationInputDisposition::Pending
    );
}

#[test]
fn initial_payload_binding_survives_a_fast_terminal_task() {
    let pool = pool();
    let accepted = accept_conversation_input(&pool, &input("message-1")).expect("accept input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'succeeded')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert fast terminal task");

    let bound = bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        accepted.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("new task claim can finish before binding");

    assert_eq!(bound.receipt.target_task_id, Some(task_id));
    assert_eq!(
        bound.receipt.disposition,
        ConversationInputDisposition::Applied
    );
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).unwrap(),
        None
    );
}

#[test]
fn action_dispatch_claim_is_versioned_and_blocks_pending_or_duplicate_dispatch() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert active task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");
    assert_eq!(
        conversation_execution_snapshot_for_task(&pool, &task_id.to_string()).unwrap(),
        Some(ConversationExecutionSnapshot {
            instruction_revision: 1,
            execution_epoch: 1,
        })
    );

    let mut followup = input("message-2");
    followup.submission.expected_instruction_revision = None;
    accept_conversation_input(&pool, &followup).expect("accept pending follow-up");
    assert!(matches!(
        claim_conversation_action_dispatch(&pool, &task_id.to_string(), 1, 1, 1, 1, "action-a")
            .expect("pending claim result"),
        ConversationActionDispatchClaimOutcome::Stale {
            pending_input: true,
            ..
        }
    ));

    let applied = apply_pending_conversation_inputs(&pool, &task_id.to_string(), 128)
        .expect("apply follow-up");
    assert_eq!(applied[0].receipt.instruction_revision, 2);
    assert_eq!(applied[0].receipt.execution_epoch, 2);
    assert!(matches!(
        claim_conversation_action_dispatch(&pool, &task_id.to_string(), 1, 1, 1, 1, "action-a")
            .expect("stale version result"),
        ConversationActionDispatchClaimOutcome::Stale {
            current: ConversationExecutionSnapshot {
                instruction_revision: 2,
                execution_epoch: 2,
            },
            pending_input: false,
        }
    ));

    let claim_id = match claim_conversation_action_dispatch(
        &pool,
        &task_id.to_string(),
        2,
        2,
        1,
        1,
        "action-a",
    )
    .expect("claim current action")
    {
        ConversationActionDispatchClaimOutcome::Claimed { claim_id } => claim_id,
        other => panic!("unexpected claim outcome: {other:?}"),
    };
    assert!(matches!(
        claim_conversation_action_dispatch(&pool, &task_id.to_string(), 2, 2, 1, 1, "action-a")
            .expect("duplicate claim result"),
        ConversationActionDispatchClaimOutcome::Existing {
            claim_id: existing_id,
            ref status,
        } if existing_id == claim_id && status == "claimed"
    ));
    settle_conversation_action_dispatch(&pool, claim_id, true).expect("settle claim");
    let status: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM conversation_action_dispatch_claims WHERE claim_id = ?1",
            [claim_id.to_string()],
            |row| row.get(0),
        )
        .expect("claim status");
    assert_eq!(status, "settled_ok");
}

#[test]
fn terminal_boundary_serializes_new_input_as_a_followup() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert active task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");

    assert_eq!(
        claim_conversation_terminal_boundary(&pool, &task_id.to_string(), 1, 1)
            .expect("claim terminal boundary"),
        ConversationTerminalBoundaryOutcome::Claimed
    );
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).expect("read focus"),
        None
    );
    assert_eq!(
        conversation_presentation_snapshot_for_task(&pool, &task_id.to_string())
            .expect("presentation snapshot"),
        Some(ConversationExecutionSnapshot {
            instruction_revision: 1,
            execution_epoch: 1,
        })
    );

    let mut followup = input("message-2");
    followup.submission.expected_instruction_revision = None;
    let accepted = accept_conversation_input(&pool, &followup).expect("accept follow-up");
    assert_eq!(accepted.record.receipt.target_task_id, None);
    assert!(matches!(
        claim_or_bind_conversation_input_task(
            &pool,
            &scope("principal-1"),
            accepted.record.receipt.input_id,
        )
        .expect("claim follow-up task"),
        ConversationInputTaskClaimOutcome::Creator { record, .. }
            if record.receipt.input_id == accepted.record.receipt.input_id
    ));
}

#[test]
fn terminal_boundary_yields_to_input_accepted_first() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert active task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");
    let mut followup = input("message-2");
    followup.submission.expected_instruction_revision = None;
    let accepted = accept_conversation_input(&pool, &followup).expect("accept follow-up");
    assert_eq!(accepted.record.receipt.target_task_id, Some(task_id));

    assert_eq!(
        claim_conversation_terminal_boundary(&pool, &task_id.to_string(), 1, 1)
            .expect("terminal boundary sees pending input"),
        ConversationTerminalBoundaryOutcome::PendingOrStale
    );
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).expect("focus remains active"),
        Some(task_id)
    );
}

#[test]
fn terminal_boundary_retries_a_transient_sqlite_writer_lock() {
    let database_path = std::env::temp_dir().join(format!(
        "agent-runtime-terminal-boundary-{}.db",
        Uuid::new_v4().simple()
    ));
    let manager = SqliteConnectionManager::file(&database_path)
        .with_init(|database| database.busy_timeout(std::time::Duration::ZERO));
    let pool = Pool::builder()
        .max_size(2)
        .build(manager)
        .expect("build file pool");
    {
        let database = pool.get().expect("get setup connection");
        ensure_conversation_input_schema(&database).expect("ensure schema");
        database
            .execute_batch(
                "CREATE TABLE tasks (
                    task_id TEXT PRIMARY KEY,
                    principal_id TEXT,
                    status TEXT NOT NULL,
                    channel TEXT NOT NULL DEFAULT 'ui',
                    external_user_id TEXT,
                    external_chat_id TEXT,
                    payload_json TEXT NOT NULL DEFAULT '{}',
                    result_json TEXT,
                    updated_at TEXT
                 );",
            )
            .expect("create tasks table");
    }
    let initial =
        accept_conversation_input(&pool, &input("writer-lock-message")).expect("accept input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert active task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind input");

    let mut lock_owner = Connection::open(&database_path).expect("open lock owner");
    lock_owner
        .busy_timeout(std::time::Duration::ZERO)
        .expect("disable lock wait");
    let owner_tx = lock_owner
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("hold writer lock");
    let contender_pool = pool.clone();
    let contender_task_id = task_id.to_string();
    let contender = std::thread::spawn(move || {
        claim_conversation_terminal_boundary(&contender_pool, &contender_task_id, 1, 1)
    });
    std::thread::sleep(std::time::Duration::from_millis(40));
    owner_tx.commit().expect("release writer lock");

    assert_eq!(
        contender
            .join()
            .expect("join contender")
            .expect("transient writer lock should recover"),
        ConversationTerminalBoundaryOutcome::Claimed
    );
    drop(lock_owner);
    drop(pool);
    for path in [
        database_path.clone(),
        database_path.with_extension("db-wal"),
        database_path.with_extension("db-shm"),
    ] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn terminal_focus_is_cleared_without_retargeting_a_new_message() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    let database = pool.get().unwrap();
    database
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    drop(database);
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind task");
    pool.get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status = 'succeeded' WHERE task_id = ?1",
            [task_id.to_string()],
        )
        .expect("finish task");

    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).unwrap(),
        None
    );
    let mut next_input = input("message-2");
    next_input.submission.expected_instruction_revision = None;
    let next = accept_conversation_input(&pool, &next_input).expect("accept unbound follow-up");
    assert_eq!(next.record.receipt.target_task_id, None);
}

#[test]
fn accepting_input_revalidates_a_stale_terminal_focus_in_the_same_transaction() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind task");
    pool.get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status = 'canceled' WHERE task_id = ?1",
            [task_id.to_string()],
        )
        .expect("cancel task without clearing focus");

    let mut next_input = input("message-2");
    next_input.submission.expected_instruction_revision = None;
    let next = accept_conversation_input(&pool, &next_input).expect("accept new unbound input");

    assert_eq!(next.record.receipt.target_task_id, None);
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).unwrap(),
        None
    );
}

#[test]
fn cancel_cutoff_defers_pending_inputs_and_clears_the_conversation_focus() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind task");

    let mut followup_input = input("message-2");
    followup_input.submission.expected_instruction_revision = None;
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept pending follow-up");
    assert_eq!(followup.record.receipt.target_task_id, Some(task_id));

    let deferred = {
        let mut database = pool.get().expect("get database");
        database
            .execute(
                "UPDATE tasks SET status = 'canceled' WHERE task_id = ?1",
                [task_id.to_string()],
            )
            .expect("cancel task");
        defer_pending_conversation_inputs_for_cancel(&mut database, &task_id.to_string())
            .expect("apply cancel cutoff")
    };

    assert_eq!(deferred, 1);
    let stored = get_conversation_input(&pool, "principal-1", followup.record.receipt.input_id)
        .expect("load deferred input");
    assert_eq!(
        stored.receipt.disposition,
        ConversationInputDisposition::Deferred
    );
    assert!(stored
        .receipt
        .decision_ref
        .as_deref()
        .is_some_and(|value| value.starts_with(&format!("cancel:{task_id}:"))));
    assert!(!task_has_pending_conversation_inputs(&pool, &task_id.to_string()).unwrap());
    assert_eq!(
        active_conversation_task(&pool, &scope("principal-1")).unwrap(),
        None
    );
    let events = list_conversation_input_events(&pool, &scope("principal-1"), 0, 20)
        .expect("load input events");
    assert!(events.iter().any(|event| {
        event.input_id == followup.record.receipt.input_id
            && event.event_kind == "deferred_on_cancel"
    }));
}

#[test]
fn withdrawing_a_pending_input_invalidates_the_old_execution_epoch() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind task");
    let before = conversation_execution_snapshot_for_task(&pool, &task_id.to_string())
        .expect("load execution snapshot")
        .expect("tracked task");

    let mut followup_input = input("message-2");
    followup_input.submission.expected_instruction_revision = None;
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept pending follow-up");
    let withdrawn =
        withdraw_conversation_input(&pool, "principal-1", followup.record.receipt.input_id)
            .expect("withdraw pending input");

    assert_eq!(
        withdrawn.receipt.disposition,
        ConversationInputDisposition::Withdrawn
    );
    assert_eq!(
        withdrawn.receipt.execution_epoch,
        before.execution_epoch + 1
    );
    assert!(!task_has_pending_conversation_inputs(&pool, &task_id.to_string()).unwrap());
    assert!(matches!(
        claim_conversation_action_dispatch(
            &pool,
            &task_id.to_string(),
            before.instruction_revision,
            before.execution_epoch,
            1,
            1,
            "pre-withdraw-action",
        )
        .expect("check stale dispatch"),
        ConversationActionDispatchClaimOutcome::Stale {
            pending_input: false,
            ..
        }
    ));
    let replayed =
        withdraw_conversation_input(&pool, "principal-1", followup.record.receipt.input_id)
            .expect("withdraw replay");
    assert_eq!(
        replayed.receipt.decision_ref,
        withdrawn.receipt.decision_ref
    );
    assert!(matches!(
        withdraw_conversation_input(&pool, "principal-1", initial.record.receipt.input_id),
        Err(ConversationInputStoreError::TargetConflict)
    ));
}

#[test]
fn withdrawing_an_unbound_creator_input_releases_its_task_creation_claim() {
    let pool = pool();
    let accepted =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    assert!(matches!(
        claim_or_bind_conversation_input_task(
            &pool,
            &scope("principal-1"),
            accepted.record.receipt.input_id,
        )
        .expect("claim task creation"),
        ConversationInputTaskClaimOutcome::Creator { .. }
    ));

    let withdrawn =
        withdraw_conversation_input(&pool, "principal-1", accepted.record.receipt.input_id)
            .expect("withdraw creator input");
    assert_eq!(
        withdrawn.receipt.disposition,
        ConversationInputDisposition::Withdrawn
    );
    let claim_count: i64 = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM conversation_input_task_claims",
            [],
            |row| row.get(0),
        )
        .expect("count task creation claims");
    assert_eq!(claim_count, 0);
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert orphan task");
    assert!(matches!(
        complete_conversation_input_task_claim(
            &pool,
            &scope("principal-1"),
            accepted.record.receipt.input_id,
            Uuid::new_v4(),
            task_id,
        ),
        Err(ConversationInputStoreError::TargetConflict)
    ));
}

#[test]
fn activating_a_deferred_input_reuses_the_single_task_creation_claim() {
    let pool = pool();
    let mut deferred = input("message-1");
    deferred.submission.delivery_mode = ConversationInputDeliveryMode::Defer;
    let accepted = accept_conversation_input(&pool, &deferred).expect("accept deferred input");
    assert_eq!(
        accepted.record.receipt.disposition,
        ConversationInputDisposition::Deferred
    );
    assert_eq!(accepted.record.receipt.target_task_id, None);

    let activated = activate_deferred_conversation_input(
        &pool,
        "principal-1",
        accepted.record.receipt.input_id,
    )
    .expect("activate deferred input");
    assert_eq!(
        activated.receipt.disposition,
        ConversationInputDisposition::Pending
    );
    assert_eq!(activated.delivery_mode, ConversationInputDeliveryMode::Auto);
    assert!(!activated.receipt.replayed);
    let replay = activate_deferred_conversation_input(
        &pool,
        "principal-1",
        accepted.record.receipt.input_id,
    )
    .expect("replay activation");
    assert!(replay.receipt.replayed);
    assert!(matches!(
        claim_or_bind_conversation_input_task(
            &pool,
            &scope("principal-1"),
            accepted.record.receipt.input_id,
        )
        .expect("claim task creation"),
        ConversationInputTaskClaimOutcome::Creator { record, .. }
            if record.receipt.input_id == accepted.record.receipt.input_id
    ));
}

#[test]
fn activating_cancel_deferred_input_does_not_revive_the_cancelled_task() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-1"],
        )
        .expect("insert task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind task");
    let mut followup = input("message-2");
    followup.submission.expected_instruction_revision = None;
    let followup = accept_conversation_input(&pool, &followup).expect("accept follow-up");
    {
        let mut database = pool.get().expect("database");
        database
            .execute(
                "UPDATE tasks SET status = 'canceled' WHERE task_id = ?1",
                [task_id.to_string()],
            )
            .expect("cancel task");
        defer_pending_conversation_inputs_for_cancel(&mut database, &task_id.to_string())
            .expect("defer on cancel");
    }

    let activated = activate_deferred_conversation_input(
        &pool,
        "principal-1",
        followup.record.receipt.input_id,
    )
    .expect("activate after cancel");
    assert_eq!(activated.receipt.target_task_id, None);
    assert_eq!(
        activated.receipt.disposition,
        ConversationInputDisposition::Pending
    );
}

#[test]
fn task_binding_rejects_another_principals_task() {
    let pool = pool();
    let accepted = accept_conversation_input(&pool, &input("message-1")).expect("accept input");
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-2"],
        )
        .expect("insert foreign task");
    let error = bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        accepted.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect_err("cross-principal binding rejected");
    assert!(matches!(error, ConversationInputStoreError::TargetConflict));
    assert_eq!(error.code(), ConversationInputErrorCode::TargetConflict);
}

#[test]
fn adopting_an_unfocused_task_requires_an_active_task_owned_by_the_principal() {
    let pool = pool();
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status) VALUES (?1, ?2, 'running')",
            rusqlite::params![task_id.to_string(), "principal-2"],
        )
        .expect("insert foreign task");
    let mut request = input("message-1");
    request.submission.expected_task_id = Some(task_id);
    let error = accept_conversation_input(&pool, &request)
        .expect_err("cross-principal task adoption rejected");
    assert!(matches!(error, ConversationInputStoreError::TargetConflict));

    pool.get()
        .unwrap()
        .execute(
            "UPDATE tasks SET principal_id = 'principal-1', status = 'succeeded' WHERE task_id = ?1",
            [task_id.to_string()],
        )
        .expect("make task owned but terminal");
    let error =
        accept_conversation_input(&pool, &request).expect_err("terminal task adoption rejected");
    assert!(matches!(error, ConversationInputStoreError::TargetConflict));
}

#[test]
fn adopting_an_unfocused_task_requires_the_same_machine_conversation_scope() {
    let pool = pool();
    let task_id = Uuid::new_v4();
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status, channel, payload_json)
             VALUES (?1, ?2, 'running', 'ui', ?3)",
            rusqlite::params![
                task_id.to_string(),
                "principal-1",
                serde_json::json!({
                    "conversation_id": "another-conversation",
                    "agent_id": "main"
                })
                .to_string(),
            ],
        )
        .expect("insert task from another conversation");
    let mut request = input("message-1");
    request.submission.expected_task_id = Some(task_id);

    let error = accept_conversation_input(&pool, &request)
        .expect_err("cross-conversation task adoption rejected");
    assert!(matches!(error, ConversationInputStoreError::TargetConflict));

    pool.get()
        .unwrap()
        .execute(
            "UPDATE tasks SET payload_json = ?2, channel = 'wechat' WHERE task_id = ?1",
            rusqlite::params![
                task_id.to_string(),
                serde_json::json!({
                    "conversation_id": "conversation-1",
                    "agent_id": "main"
                })
                .to_string(),
            ],
        )
        .expect("move task to another channel");
    let error = accept_conversation_input(&pool, &request)
        .expect_err("cross-channel task adoption rejected");
    assert!(matches!(error, ConversationInputStoreError::TargetConflict));

    pool.get()
        .unwrap()
        .execute(
            "UPDATE tasks SET channel = 'ui' WHERE task_id = ?1",
            [task_id.to_string()],
        )
        .expect("restore matching channel");
    let accepted = accept_conversation_input(&pool, &request).expect("matching task accepted");
    assert_eq!(accepted.record.receipt.target_task_id, None);
    assert_eq!(accepted.record.expected_task_id, Some(task_id));
}

fn waiting_checkpoint(checkpoint_id: &str, resume_entrypoint: &str) -> serde_json::Value {
    serde_json::json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "needs_user",
            "checkpoint_id": checkpoint_id
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "boundary_context": {},
            "observations": [],
            "capability_results": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": [],
            "budget": {
                "round": 1,
                "step": 1,
                "llm_calls": 1,
                "tool_calls": 0,
                "elapsed_ms": 1,
                "llm_elapsed_ms": 1,
                "tool_elapsed_ms": 0
            },
            "resume_entrypoint": resume_entrypoint
        }
    })
}

#[test]
fn pending_input_wakes_a_user_checkpoint_into_a_planner_resume() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    let waiting = waiting_checkpoint("checkpoint-user", "await_user_input");
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status, result_json, updated_at)
             VALUES (?1, ?2, 'running', ?3, '1')",
            rusqlite::params![task_id.to_string(), "principal-1", waiting.to_string()],
        )
        .expect("insert waiting task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");
    let mut followup_input = input("message-2");
    followup_input.submission.expected_instruction_revision = Some(1);
    followup_input.submission.expected_task_id = Some(task_id);
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept follow-up input");

    assert!(wake_task_for_pending_conversation_input(
        &pool,
        task_id,
        followup.record.receipt.input_id,
    )
    .expect("wake checkpoint"));
    let raw_result: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| row.get(0),
        )
        .expect("read task result");
    let result: serde_json::Value = serde_json::from_str(&raw_result).expect("result JSON");
    assert_eq!(result["task_lifecycle"]["state"], "waiting");
    assert_eq!(result["task_lifecycle"]["source"], "conversation_input");
    assert_eq!(
        result["task_lifecycle"]["resume_input"]["input_id"],
        followup.record.receipt.input_id.to_string()
    );
    assert_eq!(
        result["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
}

#[test]
fn pending_input_does_not_implicitly_resume_a_manual_pause() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    let mut waiting = waiting_checkpoint("checkpoint-manual", "next_planner_round");
    waiting["task_lifecycle"]["resume_reason"] = serde_json::json!("user_pause_requested");
    waiting["task_lifecycle"]["resume_policy"] = serde_json::json!("manual");
    waiting["task_lifecycle"]["manual_resume_required"] = serde_json::json!(true);
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status, result_json, updated_at)
             VALUES (?1, ?2, 'running', ?3, '1')",
            rusqlite::params![task_id.to_string(), "principal-1", waiting.to_string()],
        )
        .expect("insert manually paused task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");
    let mut followup_input = input("message-2");
    followup_input.submission.expected_instruction_revision = Some(1);
    followup_input.submission.expected_task_id = Some(task_id);
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept follow-up input");

    assert!(!wake_task_for_pending_conversation_input(
        &pool,
        task_id,
        followup.record.receipt.input_id,
    )
    .expect("manual pause remains held"));
    let raw_result: String = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| row.get(0),
        )
        .expect("read task result");
    let result: serde_json::Value = serde_json::from_str(&raw_result).expect("result JSON");
    assert_eq!(result["task_lifecycle"]["manual_resume_required"], true);
    assert_eq!(result["task_lifecycle"]["state"], "needs_user");
}

#[test]
fn pending_input_does_not_replace_an_async_job_poll_checkpoint() {
    let pool = pool();
    let initial =
        accept_conversation_input(&pool, &input("message-1")).expect("accept initial input");
    let task_id = Uuid::new_v4();
    let mut waiting = waiting_checkpoint("checkpoint-job", "poll_async_job");
    waiting["task_checkpoint"]["pending_async_job"] = serde_json::json!({
        "job_id": "job-1",
        "status": "running",
        "poll_after_seconds": 30,
        "expires_at": 9999999999_i64,
        "cancel_ref": "cancel-1",
        "message_key": "job.running"
    });
    pool.get()
        .unwrap()
        .execute(
            "INSERT INTO tasks(task_id, principal_id, status, result_json, updated_at)
             VALUES (?1, ?2, 'running', ?3, '1')",
            rusqlite::params![task_id.to_string(), "principal-1", waiting.to_string()],
        )
        .expect("insert async task");
    bind_conversation_input_to_task(
        &pool,
        &scope("principal-1"),
        initial.record.receipt.input_id,
        task_id,
        ConversationInputTaskBinding::InitialTaskPayload,
    )
    .expect("bind initial input");
    let mut followup_input = input("message-2");
    followup_input.submission.expected_instruction_revision = Some(1);
    followup_input.submission.expected_task_id = Some(task_id);
    let followup =
        accept_conversation_input(&pool, &followup_input).expect("accept follow-up input");

    assert!(!wake_task_for_pending_conversation_input(
        &pool,
        task_id,
        followup.record.receipt.input_id,
    )
    .expect("preserve async checkpoint"));
}

#[test]
fn applying_inputs_to_an_untracked_legacy_task_is_a_noop() {
    let pool = pool();

    let applied = apply_pending_conversation_inputs(&pool, "fixture-task-id", 32)
        .expect("untracked task must not fail the agent loop");

    assert!(applied.is_empty());
    assert_eq!(
        conversation_execution_snapshot_for_task(&pool, "fixture-task-id")
            .expect("untracked execution snapshot"),
        None
    );
    assert_eq!(
        conversation_presentation_snapshot_for_task(&pool, "fixture-task-id")
            .expect("untracked presentation snapshot"),
        None
    );
    assert_eq!(
        claim_conversation_terminal_boundary(&pool, "fixture-task-id", 0, 0)
            .expect("untracked terminal boundary"),
        ConversationTerminalBoundaryOutcome::Untracked
    );
    assert_eq!(
        claim_conversation_action_dispatch(&pool, "fixture-task-id", 0, 0, 1, 1, "fixture-action")
            .expect("untracked action boundary"),
        ConversationActionDispatchClaimOutcome::Untracked
    );
}

fn list_in_db(
    database: &Connection,
    scope: &OwnedConversationInputScope,
    after_input_seq: u64,
    limit: u32,
) -> Vec<claw_core::conversation_input::ConversationInputRecord> {
    let mut statement = database
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
             ORDER BY input_seq ASC LIMIT ?7",
        )
        .expect("prepare list");
    statement
        .query_map(
            rusqlite::params![
                scope.owner_principal_id,
                scope.conversation.agent_id,
                scope.conversation.channel,
                scope.conversation.channel_account_id,
                scope.conversation.conversation_id,
                i64::try_from(after_input_seq).unwrap(),
                i64::from(limit),
            ],
            row_to_record,
        )
        .expect("query list")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect list")
}
