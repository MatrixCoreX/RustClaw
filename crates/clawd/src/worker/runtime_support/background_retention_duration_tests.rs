use super::{cleanup_once, insert_task_with_kind, test_state};
use rusqlite::params;

fn cleanup(days: u64) -> (Vec<String>, Vec<String>) {
    let mut state = test_state();
    state.policy.maintenance.tasks_retention_days = days;
    state.policy.maintenance.audit_retention_days = days;
    state.policy.maintenance.tasks_max_rows = 100;
    state.policy.maintenance.audit_max_rows = 100;
    for id in ["old", "fresh"] {
        insert_task_with_kind(&state, id, "run_skill", "succeeded", None);
    }
    // A future fixture timestamp keeps the control stable if the wall clock
    // advances during cleanup. Exact equality boundaries have pure tests.
    let fresh = (crate::now_ts_u64() + 86_400).to_string();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET created_at=?1,updated_at=?1 WHERE task_id='fresh'",
            [&fresh],
        )
        .unwrap();
    {
        let db = state.core.audit_db.get().unwrap();
        for (id, at) in [("old", "1"), ("fresh", fresh.as_str())] {
            db.execute(
                "INSERT INTO audit_logs(ts,action,detail_json) VALUES (?1,'retention-duration-fixture',?2)",
                params![at, id],
            ).unwrap();
        }
    }
    cleanup_once(&state).unwrap();
    let tasks = state
        .core
        .db
        .get()
        .unwrap()
        .prepare("SELECT task_id FROM tasks ORDER BY task_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    let audit = state.core.audit_db.get().unwrap().prepare(
        "SELECT detail_json FROM audit_logs WHERE action='retention-duration-fixture' ORDER BY detail_json",
    ).unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<Vec<String>>>().unwrap();
    (tasks, audit)
}

#[test]
fn huge_task_and_audit_durations_do_not_delete_retained_rows() {
    for days in [i64::MAX as u64, u64::MAX] {
        let (tasks, audit) = cleanup(days);
        assert_eq!(tasks, ["fresh", "old"], "days={days}");
        assert_eq!(audit, ["fresh", "old"], "days={days}");
    }
}

#[test]
fn ordinary_and_zero_task_and_audit_durations_still_remove_expired_rows() {
    for days in [0, 1] {
        let (tasks, audit) = cleanup(days);
        assert_eq!(tasks, ["fresh"], "days={days}");
        assert_eq!(audit, ["fresh"], "days={days}");
    }
}
