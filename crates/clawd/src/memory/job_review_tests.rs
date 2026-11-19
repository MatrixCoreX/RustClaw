use super::*;
fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE memory_jobs(job_id TEXT,principal_id TEXT,job_kind TEXT,status TEXT,attempt INTEGER,error_code TEXT,retryable INTEGER,checkpoint_json TEXT,updated_at_ts INTEGER);
    INSERT INTO memory_jobs VALUES('job','owner','extract','failed',5,'memory_job_source_binding_unverified',0,'{\"failure\":{\"phase\":\"preflight\"}}',1);").unwrap();
    db
}
#[test]
fn review_is_owner_scoped_and_archive_is_reversible_without_discarding_failure() {
    let mut db = fixture();
    let rows = list(&db, "owner").unwrap();
    let expected = rows["jobs"][0]["review_token"].as_str().unwrap();
    assert!(list(&db, "other").unwrap()["jobs"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(disposition(&mut db, "other", "job", expected, "archive").is_err());
    disposition(&mut db, "owner", "job", expected, "archive").unwrap();
    assert!(disposition(&mut db, "owner", "job", expected, "unarchive").is_err());
    let rows = list(&db, "owner").unwrap();
    assert_eq!(rows["jobs"][0]["archived"], true);
    let expected = rows["jobs"][0]["review_token"].as_str().unwrap();
    assert!(disposition(&mut db, "owner", "job", expected, "retry").is_err());
    disposition(&mut db, "owner", "job", expected, "unarchive").unwrap();
    let checkpoint: String = db
        .query_row("SELECT checkpoint_json FROM memory_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&checkpoint).unwrap()["failure"]["phase"],
        "preflight"
    );
    assert_eq!(list(&db, "owner").unwrap()["jobs"][0]["archived"], false);
}
#[test]
fn stale_review_cannot_change_a_running_job() {
    let mut db = fixture();
    let rows = list(&db, "owner").unwrap();
    let expected = rows["jobs"][0]["review_token"].as_str().unwrap();
    db.execute("UPDATE memory_jobs SET status='running'", [])
        .unwrap();
    assert!(disposition(&mut db, "owner", "job", expected, "archive").is_err());
}
