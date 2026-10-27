use std::time::Instant;

use super::*;

#[path = "background_resume_lease_tests.rs"]
mod resume_lease;

async fn observe_database_wait(stage: &str) {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.db = r2d2::Pool::builder()
        .max_size(1)
        .build(r2d2_sqlite::SqliteConnectionManager::memory())
        .unwrap();
    let state = state.with_seeded_db_schema();
    {
        let db = state.core.db.get().unwrap();
        crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
        crate::repo::ensure_channel_delivery_outbox_schema(&db).unwrap();
        crate::repo::ensure_conversation_reply_item_schema(&db).unwrap();
    }
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let (outcome, timer_delay) = tokio::join!(
        async {
            match stage {
                "cleanup" => cleanup_tick(state).await,
                "schedule" => schedule_tick(state).await,
                "task_heartbeat" => task_heartbeat_tick(state, "absent".into(), 0)
                    .await
                    .map(|_| ()),
                _ => unreachable!(),
            }
        },
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    release.join().unwrap();
    outcome.unwrap();
    eprintln!(
        "stage={stage} pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(
        timer_delay < Duration::from_millis(400),
        "{stage}: {timer_delay:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_pool_wait_keeps_async_runtime_responsive() {
    observe_database_wait("cleanup").await;
}

#[tokio::test(flavor = "current_thread")]
async fn schedule_pool_wait_keeps_async_runtime_responsive() {
    observe_database_wait("schedule").await;
}

#[tokio::test(flavor = "current_thread")]
async fn heartbeat_pool_wait_keeps_async_runtime_responsive() {
    observe_database_wait("task_heartbeat").await;
}

#[tokio::test]
async fn blocking_ticks_preserve_results_errors_and_panics() {
    assert_eq!(run_blocking_tick("fixture", || Ok(42)).await.unwrap(), 42);
    let error = run_blocking_tick::<()>("fixture", || Err(anyhow!("fixture_error")))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "fixture_error");
    let panic = run_blocking_tick::<()>("fixture_panic", || panic!("fixture_panic"))
        .await
        .unwrap_err();
    assert!(panic
        .to_string()
        .contains("background_tick_join_failed stage=fixture_panic"));
}

#[tokio::test]
async fn cancelled_waiter_does_not_replay_started_blocking_work() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let executions = Arc::new(AtomicUsize::new(0));
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let (done, finished) = tokio::sync::oneshot::channel();
    let count = executions.clone();
    let task = tokio::spawn(run_blocking_tick("fixture", move || {
        count.fetch_add(1, Ordering::SeqCst);
        let _ = started.send(());
        let _ = wait_release.recv_timeout(Duration::from_secs(5));
        let _ = done.send(());
        Ok(())
    }));
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), finished)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn delayed_heartbeat_cannot_renew_a_cancelled_or_reclaimed_task() {
    for cancel in [true, false] {
        let state = test_state();
        state.seed_ask_task_row("heartbeat-owner", 7, 11, r#"{"text":"fixture"}"#);
        let task = repo::claim_next_task(&state).unwrap().unwrap();
        let db = state.core.db.get().unwrap();
        let heartbeat = tokio::spawn(task_heartbeat_tick(
            state.clone(),
            task.task_id.clone(),
            task.claim_attempt,
        ));
        tokio::task::yield_now().await;
        if cancel {
            db.execute(
                "UPDATE tasks SET status='canceled', lease_expires_at=123 WHERE task_id=?1",
                [&task.task_id],
            )
            .unwrap();
        } else {
            db.execute("UPDATE tasks SET claim_attempt=claim_attempt+1, lease_expires_at=123 WHERE task_id=?1", [&task.task_id]).unwrap();
        }
        drop(db);
        assert!(!heartbeat.await.unwrap().unwrap());
        let expires: i64 = state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT lease_expires_at FROM tasks WHERE task_id=?1",
                [&task.task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expires, 123);
    }
}

#[tokio::test]
async fn awaited_ticks_have_only_one_in_flight_operation_per_loop() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    for _ in 0..20 {
        let (active, maximum) = (active.clone(), maximum.clone());
        run_blocking_tick("fixture", move || {
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(count, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
    }
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
