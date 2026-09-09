use super::*;

#[test]
fn streaming_digest_preserves_serialized_bytes_across_buffer_boundaries() {
    let artifacts: Vec<_> = (0..40000)
        .map(|index| ArtifactReceipt {
            path: format!("runtime/data/{index:05}-quoted-\"-file"),
            sha256: "a".repeat(64),
            size_bytes: u64::MAX - index,
            executable: index % 2 == 0,
        })
        .collect();
    let raw = serde_json::to_vec(&artifacts).unwrap();
    assert!(raw.len() > 4 * 1024 * 1024);
    assert_eq!(
        digest_json(&artifacts).unwrap(),
        hex::encode(Sha256::digest(&raw))
    );
    assert_eq!(
        digest_json(&Vec::<ArtifactReceipt>::new()).unwrap(),
        hex::encode(Sha256::digest(b"[]"))
    );
}

#[test]
fn streaming_digest_reports_serialization_errors() {
    struct Invalid;
    impl Serialize for Invalid {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("fixture_error"))
        }
    }
    assert!(digest_json(&Invalid).is_err());
}
