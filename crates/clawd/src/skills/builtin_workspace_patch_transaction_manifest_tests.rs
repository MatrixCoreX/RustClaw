use super::tests::{error_code, Fixture, FIRST};
use super::*;
use std::fs;

fn status(fixture: &Fixture) -> Result<Value, String> {
    fixture.run(json!({"action":"transaction_status","transaction_id":"owned_shards"}))
}

#[test]
fn transaction_manifest_rejects_wrong_identity_and_noncontiguous_shard_records() {
    for case in [
        "identity",
        "duplicate",
        "gap",
        "zero_count",
        "unknown_shard_state",
    ] {
        let fixture = Fixture::new();
        fixture.apply();
        let path = fixture.manifest_path();
        let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match case {
            "identity" => manifest["transaction_id"] = json!("another_transaction"),
            "duplicate" => {
                let record = manifest["shards"][0].clone();
                manifest["shards"].as_array_mut().unwrap().push(record);
            }
            "gap" => manifest["shards"][0]["shard_index"] = json!(42),
            "zero_count" => manifest["shard_count"] = json!(0),
            "unknown_shard_state" => manifest["shards"][0]["state"] = json!("unknown"),
            _ => unreachable!(),
        }
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            error_code(status(&fixture)),
            "transaction_manifest_invalid",
            "{case}"
        );
        assert!(fixture.run(fixture.request(FIRST)).is_err(), "{case}");
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
    }
}

#[test]
fn transaction_manifest_read_bound_accepts_the_boundary_and_refuses_overflow() {
    let fixture = Fixture::new();
    fixture.apply();
    let path = fixture.manifest_path();
    let mut bytes = fs::read(&path).unwrap();
    bytes.resize(MAX_TRANSACTION_MANIFEST_BYTES, b' ');
    fs::write(&path, &bytes).unwrap();
    assert_eq!(status(&fixture).unwrap()["state"], "partial");
    bytes.push(b' ');
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        error_code(status(&fixture)),
        "transaction_manifest_too_large"
    );
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn transaction_dependency_gap_has_bounded_diagnostics_and_does_not_apply() {
    let fixture = Fixture::new();
    let error = fixture
        .run(
            json!({"action":"apply_patch_shard","transaction_id":"owned_shards",
        "shard_index":1000,"shard_count":1001,"patch":FIRST}),
        )
        .unwrap_err();
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "shard_dependency_not_applied");
    let extra = parsed.extra.unwrap();
    let details = &extra["details"]["details"];
    assert_eq!(
        details["missing_shard_indices"].as_array().unwrap().len(),
        MAX_DEPENDENCY_PREVIEW
    );
    assert_eq!(details["missing_shard_count"], 1000);
    assert_eq!(details["next_shard_index"], 0);
    assert_eq!(details["dependency_preview_truncated"], true);
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"before\n");
    assert!(!fixture.manifest_path().exists());
}

#[test]
fn transaction_replay_canonical_preconditions_do_not_recheck_the_old_preimage() {
    let fixture = Fixture::new();
    let patch = format!("{FIRST}{}", FIRST.replace("a.txt", "b.txt"));
    let mut request = fixture.request(&patch);
    let hash = super::super::sha256_label(b"before\n");
    let mut original = Map::new();
    original.insert("a.txt".into(), json!(&hash));
    original.insert("b.txt".into(), json!(&hash));
    request["precondition_hashes"] = Value::Object(original);
    let applied = fixture.run(request.clone()).unwrap();
    let mut reordered = Map::new();
    reordered.insert("b.txt".into(), json!(&hash));
    reordered.insert("a.txt".into(), json!(&hash));
    request["precondition_hashes"] = Value::Object(reordered);
    let replay = fixture.run(request).unwrap();
    assert_eq!(replay["outcome"], "already_applied");
    assert_eq!(replay["current_shard"], applied["current_shard"]);
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
    assert_eq!(fs::read(fixture.root.join("b.txt")).unwrap(), b"after\n");
}

fn manifest_with_encoded_size(fixture: &Fixture, size: usize) -> PatchTransactionManifest {
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let mut manifest = read_manifest(&fixture.root, &fixture.manifest_path(), &filesystem).unwrap();
    manifest.task_id.clear();
    let overhead = serde_json::to_vec_pretty(&manifest).unwrap().len();
    manifest.task_id = "t".repeat(size.checked_sub(overhead).unwrap());
    assert_eq!(serde_json::to_vec_pretty(&manifest).unwrap().len(), size);
    manifest
}

#[test]
fn transaction_manifest_writer_accepts_exact_reader_boundary() {
    let fixture = Fixture::new();
    fixture.apply();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let manifest = manifest_with_encoded_size(&fixture, MAX_TRANSACTION_MANIFEST_BYTES);
    write_manifest(
        &fixture.root,
        &fixture.manifest_path(),
        &manifest,
        &filesystem,
    )
    .unwrap();
    assert_eq!(
        fs::metadata(fixture.manifest_path()).unwrap().len(),
        MAX_TRANSACTION_MANIFEST_BYTES as u64
    );
    let restored = read_manifest(&fixture.root, &fixture.manifest_path(), &filesystem).unwrap();
    assert_eq!(restored.task_id, manifest.task_id);
}

#[test]
fn transaction_manifest_writer_refuses_overflow_without_publishing_or_replacing() {
    for existing in [false, true] {
        let fixture = Fixture::new();
        fixture.apply();
        let filesystem = HostFilesystemPolicy::without_private_authority();
        let path = fixture.manifest_path();
        let before = fs::read(&path).unwrap();
        let manifest = manifest_with_encoded_size(&fixture, MAX_TRANSACTION_MANIFEST_BYTES + 1);
        if !existing {
            fs::remove_file(&path).unwrap();
        }
        let error = write_manifest(&fixture.root, &path, &manifest, &filesystem)
            .expect_err("oversized metadata must not become unreadable storage");
        assert_eq!(
            crate::skills::parse_structured_skill_error(&error)
                .unwrap()
                .error_code,
            "transaction_manifest_too_large"
        );
        if existing {
            assert_eq!(fs::read(&path).unwrap(), before);
        } else {
            assert!(!path.exists());
        }
        assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
    }
}

fn capacity_boundary_manifest(fixture: &Fixture) -> PatchTransactionManifest {
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let mut manifest = read_manifest(&fixture.root, &fixture.manifest_path(), &filesystem).unwrap();
    let template = manifest.shards[0].clone();
    manifest.shard_count = 100_000;
    let mut low = 1;
    let mut high = 10_000;
    // Exercise the actual reservation boundary over bounded typed metadata;
    // these synthetic prefix records are not a replay/ownership proof.
    while low + 1 < high {
        let middle = (low + high) / 2;
        manifest.shards = (0..middle)
            .map(|shard_index| PatchShardRecord {
                shard_index,
                ..template.clone()
            })
            .collect();
        if serde_json::to_vec_pretty(&manifest).unwrap().len() <= MAX_TRANSACTION_MANIFEST_BYTES {
            low = middle;
        } else {
            high = middle;
        }
    }
    manifest.shards = (0..low)
        .map(|shard_index| PatchShardRecord {
            shard_index,
            ..template.clone()
        })
        .collect();
    let mut next = template;
    next.shard_index = low;
    assert!(serde_json::to_vec_pretty(&manifest).unwrap().len() <= MAX_TRANSACTION_MANIFEST_BYTES);
    let mut projected = manifest.clone();
    projected.shards.push(next);
    assert!(serde_json::to_vec_pretty(&projected).unwrap().len() > MAX_TRANSACTION_MANIFEST_BYTES);
    manifest
}

#[test]
fn transaction_manifest_capacity_is_checked_before_reserving_the_next_attempt() {
    let fixture = Fixture::new();
    fixture.apply();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let manifest = capacity_boundary_manifest(&fixture);
    let before = fs::read(fixture.manifest_path()).unwrap();
    let index = manifest.shards.len();
    let digest = super::super::sha256_label(FIRST.as_bytes());
    let error = attempt::ClaimedAttempt::reserve(
        &fixture.root,
        &manifest,
        index,
        &digest,
        &digest,
        &filesystem,
    )
    .err()
    .expect("no capacity for an applied shard");
    let parsed = crate::skills::parse_structured_skill_error(&error).unwrap();
    assert_eq!(parsed.error_code, "transaction_manifest_too_large");
    let extra = parsed.extra.unwrap();
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["retryable"], false);
    assert_eq!(fs::read(fixture.manifest_path()).unwrap(), before);
    assert!(!fixture
        .manifest_path()
        .parent()
        .unwrap()
        .join(format!("owned_shards.attempts/{index}.json"))
        .exists());
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"after\n");
}

#[test]
fn transaction_manifest_capacity_counts_encoded_bytes_not_characters() {
    let fixture = Fixture::new();
    fixture.apply();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let path = fixture.manifest_path();
    let before = fs::read(&path).unwrap();
    let mut manifest = read_manifest(&fixture.root, &path, &filesystem).unwrap();
    manifest.task_id = "\\\n\u{e9}".repeat(MAX_TRANSACTION_MANIFEST_BYTES / 4);
    assert!(manifest.task_id.chars().count() < MAX_TRANSACTION_MANIFEST_BYTES);
    let error = write_manifest(&fixture.root, &path, &manifest, &filesystem).unwrap_err();
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "transaction_manifest_too_large"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}
