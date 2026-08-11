use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;

fn frame(sequence: u64, interval: u64) -> skill_sdk::SkillProgressFrame {
    skill_sdk::SkillProgressFrame {
        schema_version: 1,
        record_type: "skill_progress".to_string(),
        request_id: "task-1".to_string(),
        sequence,
        kind: skill_sdk::SkillProgressKind::Heartbeat,
        detail_key: "media_discovery.background.status".to_string(),
        params: BTreeMap::from([
            ("notification_delivery".to_string(), json!("runtime")),
            ("notification_interval_seconds".to_string(), json!(interval)),
            (
                "message_key".to_string(),
                json!("channel.notice.media_discovery_background_progress"),
            ),
            ("elapsed_minutes".to_string(), json!(15)),
            ("items".to_string(), json!(4)),
            ("videos".to_string(), json!(1)),
            ("images".to_string(), json!(3)),
            ("duplicates".to_string(), json!(2)),
            ("failures".to_string(), json!(0)),
            ("platforms".to_string(), json!(["douyin", "xiaohongshu"])),
        ]),
        current: None,
        total: None,
        reference: None,
    }
}

#[test]
fn runtime_owned_heartbeat_builds_a_valid_localizable_notice() {
    let parsed = runtime_progress_notice(&frame(2, 900)).expect("runtime progress notice");
    assert_eq!(parsed.sequence, 2);
    assert_eq!(parsed.interval, Duration::from_secs(900));
    assert_eq!(
        parsed.notice.message_key,
        "channel.notice.media_discovery_background_progress"
    );
    assert_eq!(parsed.notice.params["platforms"], "douyin,xiaohongshu");
    assert_eq!(parsed.notice.params["items"], "4");
    parsed.notice.validate().expect("valid channel notice");
}

#[test]
fn runtime_progress_notice_rejects_spam_and_non_runtime_frames() {
    assert!(runtime_progress_notice(&frame(1, 899)).is_none());
    let mut wrong_owner = frame(1, 900);
    wrong_owner
        .params
        .insert("notification_delivery".to_string(), json!("skill"));
    assert!(runtime_progress_notice(&wrong_owner).is_none());
    let mut arbitrary_key = frame(1, 900);
    arbitrary_key
        .params
        .insert("message_key".to_string(), json!("skill.arbitrary"));
    assert!(runtime_progress_notice(&arbitrary_key).is_none());
}

#[test]
fn runtime_progress_notice_is_rate_limited_by_the_host() {
    let started = Instant::now();
    assert!(due_runtime_progress_notice(&frame(1, 900), None, started).is_some());
    assert!(due_runtime_progress_notice(
        &frame(2, 900),
        Some(started),
        started + Duration::from_secs(899),
    )
    .is_none());
    assert!(due_runtime_progress_notice(
        &frame(3, 900),
        Some(started),
        started + Duration::from_secs(900),
    )
    .is_some());
}

#[test]
fn durable_stdout_parser_accepts_only_valid_matching_progress_frames() {
    let valid = serde_json::to_string(&frame(1, 900)).expect("encode frame");
    let wrong_request = serde_json::to_string(&skill_sdk::SkillProgressFrame {
        request_id: "another-task".to_string(),
        ..frame(2, 900)
    })
    .expect("encode mismatched frame");
    let lines = format!(
        "diagnostic output\n{valid}\n{wrong_request}\n{{\"request_id\":\"task-1\",\"status\":\"ok\"}}\n"
    );

    let parsed = validated_progress_frames(&lines, "task-1");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].sequence, 1);
    assert_eq!(parsed[0].detail_key, "media_discovery.background.status");
}
