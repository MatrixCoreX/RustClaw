use super::*;

#[test]
fn raw_staging_is_measured_as_domain_growth_not_relabelled_as_source_quota() {
    let setup = Setup::new(false);
    let measure = || {
        let db = setup.fixture.state().core.db.get().unwrap();
        match crate::memory::availability::memory_btrees(&db) {
            crate::memory::availability::ByteMeasurement::Measured { bytes } => bytes,
            value => panic!("expected native domain measurement: {value:?}"),
        }
    };
    let before = measure();
    let bytes = vec![b'x'; 32 * 1024];
    let upload = setup.upload(&bytes, bytes.len() as u64);
    let usage = || {
        let db = setup.fixture.state().core.db.get().unwrap();
        db.query_row(
            "SELECT used_rows,used_bytes FROM memory_principal_quotas WHERE principal_id=?1",
            [&setup.actor.principal_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap()
    };
    let source_usage = usage();
    setup.put_all(&upload.upload_id, &bytes, bytes.len());
    assert!(measure() > before);
    assert_eq!(usage(), source_usage);
}
