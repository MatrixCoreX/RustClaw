use super::*;
#[path = "test_fixtures.rs"]
mod fixtures;
use fixtures::*;

#[test]
fn empty_export_retains_schema1_vec_checksum() {
    let raw = export(vec![]);
    let result = parse_export(&raw[..], 1024, |_| panic!("empty_export_callback")).unwrap();
    assert_eq!(result.item_count, 0);
    assert_eq!(result.checksum, digest(b"[]"));
    assert_eq!(result.schema_version, 1);
    assert_eq!(result.exported_at_ts, 100);
    assert_eq!(result.scope_kind, "principal");
}

#[test]
fn one_byte_reads_preserve_unicode_escapes_and_canonical_item_order() {
    let values = vec![item("中文，quote\" newline\n slash\\ 🐼"), item("第二条")];
    let expected = serde_json::to_vec(&values).unwrap();
    let raw = String::from_utf8(export(values))
        .unwrap()
        .replace("中文", r"\u4e2d\u6587");
    let mut decoded = Vec::new();
    let summary = parse_export(OneByte(io::Cursor::new(raw.into_bytes())), 4096, |item| {
        decoded.push(item);
        Ok(())
    })
    .unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), expected);
    assert_eq!(summary.checksum, digest(&expected));
    assert_eq!(summary.item_count, 2);
}

#[test]
fn top_level_field_order_and_unknown_fields_match_legacy_typed_hash() {
    let raw = export(vec![item("field order")]);
    let mut value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    value["ignored"] = serde_json::json!({"future":[1,2,3]});
    value["items"][0]["unknown_item_field"] = serde_json::json!({"future":true});
    let items = serde_json::to_string(&value["items"]).unwrap();
    let checksum = serde_json::to_string(&value["checksum"]).unwrap();
    for raw in [
        serde_json::to_string(&value).unwrap(),
        format!(
            r#"{{"items":{items},"scope_kind":"principal","checksum":{checksum},"exported_at_ts":100,"schema_version":1}}"#
        ),
        format!(
            r#"{{"checksum":{checksum},"schema_version":1,"exported_at_ts":100,"scope_kind":"principal","items":{items}}}"#
        ),
    ] {
        let result = parse_export(raw.as_bytes(), 4096, |_| Ok(())).unwrap();
        assert_eq!(result.item_count, 1);
        assert_eq!(result.checksum, value["checksum"].as_str().unwrap());
    }
}

#[test]
fn decoder_does_not_impose_the_legacy_ten_thousand_item_ceiling() {
    let raw = export((0..10_001).map(|_| item("small record")).collect());
    let mut count = 0u64;
    let summary = parse_export(&raw[..], 2048, |_| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 10_001);
    assert_eq!(summary.item_count, count);
}

#[test]
fn decoder_does_not_confuse_legacy_character_filter_with_a_parser_limit() {
    let text = "字".repeat(32_769);
    let raw = export(vec![item(&text)]);
    let mut actual = String::new();
    parse_export(&raw[..], 128 * 1024, |item| {
        actual = item.content;
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, text);
    // This is decoding evidence, not acceptance by the separate domain policy.
}

#[test]
fn oversized_single_item_is_rejected_before_consumer_allocation() {
    let raw = export(vec![item(&"x".repeat(4096))]);
    let mut callbacks = 0;
    let result = parse_export(&raw[..], 512, |_| {
        callbacks += 1;
        Ok(())
    });
    crate::memory::import_upload::test_fixtures::error(result, UploadError::ValueTooLarge);
    assert_eq!(callbacks, 0);
}

#[test]
fn oversized_metadata_and_unknown_field_keys_are_bounded_too() {
    let mut value: serde_json::Value = serde_json::from_slice(&export(vec![])).unwrap();
    value["scope_kind"] = serde_json::Value::String("x".repeat(4096));
    rejected(
        &serde_json::to_vec(&value).unwrap(),
        512,
        UploadError::ValueTooLarge,
    );
    value["scope_kind"] = serde_json::json!("principal");
    value
        .as_object_mut()
        .unwrap()
        .insert("k".repeat(4096), serde_json::json!(true));
    rejected(
        &serde_json::to_vec(&value).unwrap(),
        512,
        UploadError::ValueTooLarge,
    );
}

#[test]
fn wrong_export_checksum_never_returns_a_publishable_summary() {
    let mut value: serde_json::Value =
        serde_json::from_slice(&export(vec![item("provisional")])).unwrap();
    value["checksum"] = serde_json::json!(digest(b"wrong"));
    let mut provisional = 0;
    let result = parse_export(&serde_json::to_vec(&value).unwrap()[..], 2048, |_| {
        provisional += 1;
        Ok(())
    });
    crate::memory::import_upload::test_fixtures::error(result, UploadError::ExportChecksumMismatch);
    assert_eq!(provisional, 1); // Consumer effects must remain non-canonical staging.
}

#[test]
fn duplicate_missing_malformed_and_trailing_documents_fail_structurally() {
    let raw = String::from_utf8(export(vec![])).unwrap();
    let checksum = serde_json::to_string(&digest(b"[]")).unwrap();
    for bad in [
        format!(
            r#"{{"schema_version":1,"schema_version":1,"exported_at_ts":100,"scope_kind":"principal","items":[],"checksum":{checksum}}}"#
        ),
        format!(
            r#"{{"schema_version":1,"exported_at_ts":100,"scope_kind":"principal","checksum":{checksum}}}"#
        ),
        format!("{raw} {{}}"),
        raw[..raw.len() - 1].to_string(),
    ] {
        rejected(bad.as_bytes(), 2048, UploadError::InvalidJson);
    }
    parse_export(format!("{raw}{}", " ".repeat(4096)).as_bytes(), 512, |_| {
        Ok(())
    })
    .unwrap();
}

#[test]
fn unsupported_schema_is_a_typed_error_not_parsed_prose() {
    let mut value: serde_json::Value = serde_json::from_slice(&export(vec![])).unwrap();
    value["schema_version"] = serde_json::json!(2);
    rejected(
        &serde_json::to_vec(&value).unwrap(),
        2048,
        UploadError::ExportSchemaUnsupported,
    );
}

#[test]
fn consumer_failure_keeps_its_original_machine_error() {
    let raw = export(vec![item("first"), item("must not consume")]);
    let mut count = 0;
    let result = parse_export(&raw[..], 2048, |_| {
        count += 1;
        Err(UploadError::LeaseChanged.into())
    });
    crate::memory::import_upload::test_fixtures::error(result, UploadError::LeaseChanged);
    assert_eq!(count, 1);
}

#[test]
fn underlying_io_failure_keeps_its_actual_error_kind() {
    let reader = BrokenReader {
        bytes: io::Cursor::new(export(vec![item("content")])),
        fail_at: 32,
    };
    let error = parse_export(reader, 2048, |_| Ok(())).unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn actual_two_point_six_megabyte_unicode_export_streams_from_durable_chunks() {
    use crate::memory::import_upload::test_fixtures::{policy, Setup};
    let env = Setup::new(false);
    let raw = export((0..48).map(|_| item(&"文".repeat(18_000))).collect());
    assert!(raw.len() > 2_600_000);
    let upload = env.upload(&raw, 64 * 1024);
    env.put_all(&upload.upload_id, &raw, 64 * 1024);
    let db = env.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &env.actor, &upload.upload_id, 102, policy(64 * 1024)).unwrap();
    let mut reader = ChunkReader::new(&db, &env.actor, &lease).unwrap();
    let mut count = 0;
    let summary = parse_export(&mut reader, 128 * 1024, |record| {
        assert_eq!(record.content.chars().count(), 18_000);
        count += 1;
        Ok(())
    })
    .unwrap();
    reader.verify_complete().unwrap();
    assert_eq!(count, 48);
    assert_eq!(summary.item_count, 48);
}
