use super::*;
use std::thread;

#[test]
fn closed_allows_until_threshold() {
    let cb = CircuitBreaker::new();
    for _ in 0..(FAILURE_THRESHOLD - 1) {
        let attempt = cb.before_attempt();
        assert!(matches!(attempt.decision, AttemptDecision::Allow));
        attempt.note_failure();
    }
    // 第 FAILURE_THRESHOLD-1 次失败后，仍处于 Closed
    assert_eq!(cb.snapshot().state, "closed");

    // 第 FAILURE_THRESHOLD 次：触发 Open
    let attempt = cb.before_attempt();
    assert!(matches!(attempt.decision, AttemptDecision::Allow));
    attempt.note_failure();
    assert_eq!(cb.snapshot().state, "open");
}

#[test]
fn open_skips_until_cooldown_elapses() {
    let cb = CircuitBreaker::new();
    for _ in 0..FAILURE_THRESHOLD {
        cb.before_attempt().note_failure();
    }
    let attempt = cb.before_attempt();
    match attempt.decision {
        AttemptDecision::SkipCooldown { remaining_ms } => {
            assert!(remaining_ms <= INITIAL_COOLDOWN_MS);
            assert!(remaining_ms > 0);
        }
        other => panic!("expected SkipCooldown, got {other:?}"),
    }
}

#[test]
fn success_resets_counter_and_state() {
    let cb = CircuitBreaker::new();
    cb.before_attempt().note_failure();
    cb.before_attempt().note_failure();
    assert_eq!(cb.snapshot().consecutive_failures, 2);
    cb.before_attempt().note_success();
    let snapshot = cb.snapshot();
    assert_eq!(snapshot.state, "closed");
    assert_eq!(snapshot.consecutive_failures, 0);
    assert_eq!(snapshot.current_cooldown_ms, INITIAL_COOLDOWN_MS);
    assert_eq!(snapshot.remaining_cooldown_ms, 0);
}

#[test]
fn half_open_failure_doubles_cooldown_capped_at_max() {
    let cb = CircuitBreaker::new();
    for _ in 0..FAILURE_THRESHOLD {
        cb.before_attempt().note_failure();
    }
    expire_cooldown(&cb);
    cb.before_attempt().note_failure();
    let snapshot = cb.snapshot();
    assert_eq!(snapshot.state, "open");
    assert_eq!(snapshot.current_cooldown_ms, INITIAL_COOLDOWN_MS * 2);

    // 反复 HalfOpen→Open，cooldown 翻倍直到封顶
    for _ in 0..20 {
        expire_cooldown(&cb);
        cb.before_attempt().note_failure();
    }
    assert_eq!(cb.snapshot().current_cooldown_ms, MAX_COOLDOWN_MS);
}

#[test]
fn open_transitions_to_half_open_after_cooldown_zero() {
    // 用 0 cooldown 模拟"已到期"
    let cb = CircuitBreaker::new();
    {
        let mut inner = cb.inner.lock().unwrap();
        inner.state = State::Open;
        inner.current_cooldown_ms = 0;
        inner.opened_at = Some(Instant::now());
    }
    // sleep 1ms 确保 elapsed > 0
    thread::sleep(Duration::from_millis(1));
    let attempt = cb.before_attempt();
    match attempt.decision {
        AttemptDecision::AllowTrial => {}
        other => panic!("expected AllowTrial after cooldown, got {other:?}"),
    }
    assert_eq!(cb.snapshot().state, "half_open");
}

#[test]
fn half_open_admits_only_one_trial() {
    let cb = CircuitBreaker::new();
    {
        let mut inner = cb.inner.lock().unwrap();
        inner.state = State::Open;
        inner.current_cooldown_ms = 0;
        inner.opened_at = Some(Instant::now());
    }
    let first = cb.before_attempt();
    let second = cb.before_attempt();
    assert!(matches!(first.decision, AttemptDecision::AllowTrial));
    assert!(matches!(
        second.decision,
        AttemptDecision::SkipTrialInFlight { .. }
    ));
}

fn expire_cooldown(cb: &CircuitBreaker) {
    let mut inner = cb.inner.lock().unwrap();
    assert_eq!(inner.state, State::Open);
    inner.opened_at = Some(Instant::now() - Duration::from_millis(inner.current_cooldown_ms));
}

#[test]
fn dropped_trial_releases_exclusivity_without_counting_a_failure() {
    let cb = CircuitBreaker::new();
    for _ in 0..FAILURE_THRESHOLD {
        cb.before_attempt().note_failure();
    }
    expire_cooldown(&cb);
    let trial = cb.before_attempt();
    assert_eq!(trial.decision, AttemptDecision::AllowTrial);
    drop(trial);
    let snapshot = cb.snapshot();
    assert_eq!(snapshot.state, "open");
    assert_eq!(snapshot.consecutive_failures, FAILURE_THRESHOLD);
    assert_eq!(snapshot.current_cooldown_ms, INITIAL_COOLDOWN_MS);
    expire_cooldown(&cb);
    let next = cb.before_attempt();
    assert_eq!(next.decision, AttemptDecision::AllowTrial);
    next.note_success();
    assert_eq!(cb.snapshot().state, "closed");
}

#[test]
fn late_success_or_failure_cannot_change_a_newer_open_generation() {
    let cb = CircuitBreaker::new();
    let old_success = cb.before_attempt();
    let old_failure = cb.before_attempt();
    for _ in 0..FAILURE_THRESHOLD {
        cb.before_attempt().note_failure();
    }
    let snapshot = cb.snapshot();
    old_success.note_success();
    old_failure.note_failure();
    assert_eq!(cb.snapshot().state, "open");
    assert_eq!(
        cb.snapshot().consecutive_failures,
        snapshot.consecutive_failures
    );
    assert_eq!(
        cb.snapshot().current_cooldown_ms,
        snapshot.current_cooldown_ms
    );
}

#[test]
fn concurrent_recovery_has_exactly_one_trial() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };
    let cb = Arc::new(CircuitBreaker::new());
    for _ in 0..FAILURE_THRESHOLD {
        cb.before_attempt().note_failure();
    }
    expire_cooldown(&cb);
    let barrier = Arc::new(Barrier::new(24));
    let trials = Arc::new(AtomicUsize::new(0));
    let threads = (0..24)
        .map(|_| {
            let (cb, barrier, trials) = (cb.clone(), barrier.clone(), trials.clone());
            thread::spawn(move || {
                let attempt = cb.before_attempt();
                if attempt.decision == AttemptDecision::AllowTrial {
                    trials.fetch_add(1, Ordering::SeqCst);
                }
                barrier.wait();
                drop(attempt);
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(trials.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_an_ordinary_attempt_does_not_change_health() {
    let cb = CircuitBreaker::new();
    drop(cb.before_attempt());
    assert_eq!(cb.snapshot().state, "closed");
    assert_eq!(cb.snapshot().consecutive_failures, 0);
}

#[tokio::test]
async fn cancelling_a_trial_future_releases_its_permit() {
    let cb = std::sync::Arc::new(CircuitBreaker::new());
    for _ in 0..FAILURE_THRESHOLD {
        cb.before_attempt().note_failure();
    }
    expire_cooldown(&cb);
    let (ready, wait_ready) = tokio::sync::oneshot::channel();
    let runner = cb.clone();
    let running = tokio::spawn(async move {
        let attempt = runner.before_attempt();
        assert_eq!(attempt.decision, AttemptDecision::AllowTrial);
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(attempt);
    });
    wait_ready.await.unwrap();
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert_eq!(cb.snapshot().state, "open");
    assert_eq!(cb.snapshot().consecutive_failures, FAILURE_THRESHOLD);
}

#[tokio::test]
async fn text_and_native_gateways_preserve_waiting_when_every_circuit_is_unavailable() {
    use std::sync::Arc;
    for native in [false, true] {
        for busy in [false, true] {
            let mut state =
                crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
            let mut provider = (*state.core.llm_providers[0]).clone();
            provider.config.provider_type = "openai_compat".into();
            provider.config.supports_tools = true;
            provider.config.base_url = "http://127.0.0.1:9".into();
            let breaker = provider.breaker.clone();
            for _ in 0..FAILURE_THRESHOLD {
                breaker.before_attempt().note_failure();
            }
            if busy {
                expire_cooldown(&breaker);
            }
            let trial = busy.then(|| breaker.before_attempt());
            if let Some(trial) = &trial {
                assert_eq!(trial.decision, AttemptDecision::AllowTrial);
            }
            state.core.llm_providers = vec![Arc::new(provider)];
            let task_id = uuid::Uuid::new_v4().to_string();
            state.seed_ask_task_row(&task_id, 7, 11, r#"{"text":"fixture"}"#);
            let task = crate::repo::claim_next_task(&state).unwrap().unwrap();
            let outcome = tokio::time::timeout(Duration::from_secs(2), async {
                if native {
                    crate::llm_gateway::run_native_model_turn_with_fallback(
                        &state,
                        &task,
                        "fixture",
                        "test:circuit_wait",
                        &claw_core::model_turn::ModelTurnRequest::text("fixture"),
                    )
                    .await
                    .map(|_| ())
                } else {
                    crate::llm_gateway::run_with_fallback_with_prompt_source(
                        &state,
                        &task,
                        "fixture",
                        "test:circuit_wait",
                    )
                    .await
                    .map(|_| ())
                }
            })
            .await
            .expect("circuit skip must not call the provider");
            assert!(outcome.is_err(), "native={native} busy={busy}");
            let blocker = state
                .task_provider_blocker(&task.task_id)
                .expect("retain recoverable wait");
            assert_eq!(blocker.status_code, "provider_circuit_open");
            assert!(blocker.external_provider_blocked);
            assert!(blocker.retry_after_seconds > 0);
            if busy {
                assert_eq!(blocker.retry_after_seconds, TRIAL_RECHECK_MS / 1000);
                assert_eq!(breaker.snapshot().state, "half_open");
            }
            let db = state.core.db.get().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM llm_cost_ledger WHERE task_id=?1",
                    [&task.task_id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            drop(trial);
        }
    }
}
