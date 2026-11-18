use crate::memory::capacity_test_fixtures::Fixture;

#[test]
fn claim_order_work_does_not_multiply_queued_and_running_populations() {
    let mut work = Vec::new();
    for population in [128, 256] {
        let fixture = Fixture::memory();
        for n in 0..population {
            fixture.seed_job(&format!("running-{n}"), "running");
            fixture.seed_job(&format!("queued-{n}"), "queued");
        }
        let db = fixture.state().core.db.get().unwrap();
        let before = db.total_changes();
        let mut query = db.prepare(super::GENERATION_CANDIDATE_SQL).unwrap();
        let rows = query
            .query_map([crate::now_ts_u64() as i64], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), population);
        assert_eq!(
            db.total_changes(),
            before,
            "claim-order reads must not mutate jobs"
        );
        let steps = query.get_status(rusqlite::StatementStatus::VmStep) as u64;
        assert!(steps > 0);
        work.push(steps);
    }
    eprintln!(
        "GENERATION_CLAIM_WORK_JSON {}",
        serde_json::json!({
            "small_queued":128,"small_running":128,"large_queued":256,"large_running":256,
            "small_vm_steps":work[0],"large_vm_steps":work[1],"SQLite_version":rusqlite::version()
        })
    );
    assert!(
        work[1] <= 3 * work[0],
        "doubling queued and running jobs must not multiply their populations"
    );
}
