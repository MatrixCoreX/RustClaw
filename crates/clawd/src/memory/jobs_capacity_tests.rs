use crate::memory::capacity_test_fixtures::{job_status, pressure, Fixture};

#[test]
fn queued_generation_is_not_claimed_after_global_capacity_pauses() {
    for state in ["automatic_generation_paused", "explicit_write_blocked"] {
        let fixture = Fixture::memory();
        fixture.seed_job("queued", "queued");
        {
            let db = fixture.state().core.db.get().unwrap();
            pressure(&db, state);
        }
        assert!(
            super::claim_next_job(fixture.state(), "capacity-worker")
                .unwrap()
                .is_none(),
            "{state}"
        );
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(job_status(&db, "queued"), ("queued".to_string(), 0));
    }
}

#[test]
fn queued_generation_rechecks_rows_bytes_and_spent_cost_quotas_at_claim() {
    for column in ["max_rows", "max_bytes", "max_background_cost_microunits"] {
        let fixture = Fixture::memory();
        fixture.seed_job("queued", "queued");
        {
            let db = fixture.state().core.db.get().unwrap();
            db.execute(
                &format!("UPDATE memory_principal_quotas SET {column} = 0 WHERE principal_id = ?1"),
                [&fixture.principal],
            )
            .unwrap();
        }
        assert!(
            super::claim_next_job(fixture.state(), "capacity-worker")
                .unwrap()
                .is_none(),
            "{column}"
        );
    }
}

#[test]
fn generation_claim_rechecks_a_pressure_change_after_its_readiness_probe() {
    let fixture = Fixture::memory();
    fixture.seed_job("queued", "queued");
    {
        let db = fixture.state().core.db.get().unwrap();
        assert!(super::memory_job_claim_needs_write(&db, crate::now_ts_u64() as i64).unwrap());
        pressure(&db, "automatic_generation_paused");
    }
    assert!(super::claim_next_job(fixture.state(), "capacity-worker")
        .unwrap()
        .is_none());
}

#[test]
fn paused_generation_still_cancels_queued_work_and_recovers_expired_leases() {
    let fixture = Fixture::memory();
    for (id, state) in [
        ("cancel", "queued"),
        ("expired", "running"),
        ("active", "running"),
    ] {
        fixture.seed_job(id, state);
    }
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "automatic_generation_paused");
        db.execute(
            "UPDATE memory_jobs SET cancel_requested = 1 WHERE job_id = 'cancel'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE memory_jobs SET lease_expires_at_ts = 1 WHERE job_id = 'expired'",
            [],
        )
        .unwrap();
    }
    let claimed = super::claim_next_job(fixture.state(), "capacity-worker").unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(job_status(&db, "cancel"), ("cancelled".to_string(), 0));
    assert_eq!(job_status(&db, "expired"), ("retry_wait".to_string(), 1));
    assert_eq!(job_status(&db, "active"), ("running".to_string(), 1));
    assert!(
        claimed.is_none(),
        "maintenance is not permission to start more provider work"
    );
}

#[test]
fn an_expired_cancelled_generation_lease_is_finalized_without_a_live_worker() {
    let fixture = Fixture::memory();
    fixture.seed_job("expired-cancel", "running");
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "explicit_write_blocked");
        db.execute(
            "UPDATE memory_jobs SET cancel_requested = 1, lease_expires_at_ts = 1",
            [],
        )
        .unwrap();
    }
    assert!(super::claim_next_job(fixture.state(), "capacity-worker")
        .unwrap()
        .is_none());
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        job_status(&db, "expired-cancel"),
        ("cancelled".to_string(), 1)
    );
}

#[test]
fn paused_queued_generation_poll_does_not_contend_for_sqlite_writer_lock() {
    let fixture = Fixture::file();
    fixture.seed_job("queued", "queued");
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "automatic_generation_paused");
    }
    let writer = fixture.competing_writer();
    let result = super::claim_next_job(fixture.state(), "capacity-worker");
    writer.execute_batch("ROLLBACK").unwrap();
    assert!(result
        .expect("paused polling needs no writer transaction")
        .is_none());
}

#[test]
fn normal_capacity_retains_single_owner_claim_behavior() {
    let fixture = Fixture::memory();
    fixture.seed_job("queued", "queued");
    assert_eq!(
        super::claim_next_job(fixture.state(), "capacity-worker")
            .unwrap()
            .unwrap()
            .job_id,
        "queued"
    );
    assert!(super::claim_next_job(fixture.state(), "other-worker")
        .unwrap()
        .is_none());
}
