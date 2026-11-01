use super::*;

#[test]
fn host_pin_mismatch_is_rejected_before_the_admission_sequence_write() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let store = journal.initialize(&fixture.broker()).unwrap();
    let expected = store.authority_fingerprint().unwrap();
    let wrong = absent_pin();
    assert_ne!(expected, wrong);
    drop(store);
    assert!(matches!(
        journal.resume(&fixture.broker(), &wrong),
        Err(AdmissionFailure::AuthorityMismatch)
    ));
    assert_eq!(fixture.sequence(), 1);
    assert_eq!(
        journal
            .resume(&fixture.broker(), &expected)
            .unwrap()
            .authority_fingerprint()
            .unwrap(),
        expected
    );
    assert_eq!(fixture.sequence(), 2);
}

#[test]
fn coherent_journal_and_key_replacement_cannot_replace_the_host_pin() {
    let fixture = Fixture::new();
    let replacement = Fixture::new();
    let mut journal = fixture.journal();
    let expected = journal
        .initialize(&fixture.broker())
        .unwrap()
        .authority_fingerprint()
        .unwrap();
    let different = replacement
        .journal()
        .initialize(&replacement.broker())
        .unwrap()
        .authority_fingerprint()
        .unwrap();
    assert_ne!(expected, different);
    // Only new owned synthetic authority files are replaced; no key bytes are
    // retained in logs, frames or ordinary runtime state.
    fs::copy(replacement.key_file(), fixture.key_file()).unwrap();
    journal
        .db
        .execute(
            "UPDATE local_process_authority_bootstrap SET fingerprint = ?1",
            [different.as_bytes().as_slice()],
        )
        .unwrap();
    let inode = fs::metadata(fixture.key_file()).unwrap().ino();
    assert!(matches!(
        journal.resume(&fixture.broker(), &expected),
        Err(AdmissionFailure::AuthorityMismatch)
    ));
    assert_eq!(fixture.sequence(), 1);
    assert_eq!(fs::metadata(fixture.key_file()).unwrap().ino(), inode);
}
