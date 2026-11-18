use super::*;
use crate::local_process_owner::owner_store::tests::{binding, credential};

struct WireFixture {
    fingerprint: AuthorityFingerprint,
    request: Uuid,
    digest: Sha256,
    decoder: InventoryDecoder,
}
impl WireFixture {
    fn new() -> Self {
        let fingerprint = AuthorityFingerprint::from_slice(&[7; 32]).unwrap();
        let request = Uuid::new_v4();
        Self {
            fingerprint,
            request,
            digest: Sha256::new(),
            decoder: InventoryDecoder::new(fingerprint, request),
        }
    }
    fn begin(&self) -> InventoryFrame {
        InventoryFrame::Begin {
            schema_version: 1,
            fingerprint: *self.fingerprint.as_bytes(),
            request_id: self.request,
        }
    }
    fn item(&self, index: u64, job: Uuid) -> InventoryFrame {
        InventoryFrame::Item {
            schema_version: 1,
            fingerprint: *self.fingerprint.as_bytes(),
            request_id: self.request,
            index,
            credential: credential(job),
            binding: binding(),
            budget: OwnerResourceBudget {
                memory_mib: 128,
                cpu_cores: 1,
                network_slots: 0,
                provider_slots: 0,
                browser_slots: 0,
                heavy: false,
                background: true,
            },
        }
    }
    fn send(&mut self, frame: &InventoryFrame) {
        let bytes = serde_json::to_vec(frame).unwrap();
        assert!(!self.decoder.push(&bytes).unwrap());
        self.digest.update(bytes);
    }
    fn end(&self, pending: u64) -> InventoryFrame {
        InventoryFrame::Complete {
            schema_version: 1,
            fingerprint: *self.fingerprint.as_bytes(),
            request_id: self.request,
            examined: pending + 3,
            pending,
            digest: self.digest.clone().finalize().into(),
        }
    }
}

#[test]
fn inventory_wire_supports_multiple_frames_and_requires_exact_complete() {
    let mut wire = WireFixture::new();
    wire.send(&wire.begin());
    for i in 0..16 {
        wire.send(&wire.item(i, Uuid::new_v4()));
    }
    assert!(wire
        .decoder
        .push(&serde_json::to_vec(&wire.end(16)).unwrap())
        .unwrap());
    let (examined, rows) = wire.decoder.finish().unwrap();
    assert_eq!(examined, 19);
    assert_eq!(rows.len(), 16);
}

#[test]
fn inventory_wire_truncation_is_not_a_complete_empty_inventory() {
    for with_item in [false, true] {
        let mut wire = WireFixture::new();
        wire.send(&wire.begin());
        if with_item {
            wire.send(&wire.item(0, Uuid::new_v4()));
        }
        assert!(matches!(
            wire.decoder.finish(),
            Err(InventoryFailure::Incomplete)
        ));
    }
}

#[test]
fn inventory_wire_rejects_sequence_digest_binding_schema_and_unknown_fields() {
    for case in [
        "before_begin",
        "duplicate_begin",
        "index",
        "duplicate_job",
        "digest",
        "count",
        "request",
        "authority",
        "schema",
        "unknown",
    ] {
        let mut wire = WireFixture::new();
        let job = Uuid::new_v4();
        if case != "before_begin" {
            wire.send(&wire.begin());
        }
        let frame = match case {
            "duplicate_begin" => wire.begin(),
            "index" => wire.item(1, job),
            "duplicate_job" => {
                wire.send(&wire.item(0, job));
                wire.item(1, job)
            }
            "digest" | "count" => wire.end(if case == "count" { 1 } else { 0 }),
            _ => wire.item(0, job),
        };
        let mut value = serde_json::to_value(frame).unwrap();
        match case {
            "digest" => value["digest"] = serde_json::to_value([3u8; 32]).unwrap(),
            "request" => value["request_id"] = serde_json::json!(Uuid::new_v4()),
            "authority" => value["fingerprint"] = serde_json::to_value([8u8; 32]).unwrap(),
            "schema" => value["schema_version"] = serde_json::json!(2),
            "unknown" => value["path"] = serde_json::json!("unadmitted"),
            _ => {}
        }
        assert!(
            wire.decoder
                .push(&serde_json::to_vec(&value).unwrap())
                .is_err(),
            "{case}"
        );
        assert!(wire.decoder.finish().is_err(), "{case}");
    }
}

#[test]
fn inventory_wire_cannot_reuse_a_completed_or_failed_stream() {
    let mut wire = WireFixture::new();
    wire.send(&wire.begin());
    assert!(wire
        .decoder
        .push(&serde_json::to_vec(&wire.end(0)).unwrap())
        .unwrap());
    assert!(wire
        .decoder
        .push(&serde_json::to_vec(&wire.begin()).unwrap())
        .is_err());
    assert!(wire.decoder.finish().is_err());
}

#[test]
fn inventory_wire_bound_is_a_refusal_not_a_truncated_success() {
    let mut wire = WireFixture::new();
    wire.send(&wire.begin());
    for i in 0..MAX_PENDING_OWNERS {
        wire.send(&wire.item(i as u64, Uuid::new_v4()));
    }
    assert_eq!(
        wire.decoder.push(
            &serde_json::to_vec(&wire.item(MAX_PENDING_OWNERS as u64, Uuid::new_v4())).unwrap()
        ),
        Err(InventoryFailure::LimitExceeded)
    );
    assert!(wire.decoder.finish().is_err());
}
