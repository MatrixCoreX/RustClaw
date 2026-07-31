use std::collections::BTreeSet;

use super::*;

#[test]
fn catalog_keys_and_machine_metadata_are_complete_and_unique() {
    let mut keys = BTreeSet::new();
    let mut source_kinds = BTreeSet::new();

    for record in channel_capability_catalog() {
        assert_eq!(record.schema_version, CHANNEL_CAPABILITY_SCHEMA_VERSION);
        assert_eq!(record.verified_at, CHANNEL_CAPABILITY_VERIFIED_AT);
        assert_eq!(record.policy_version, CHANNEL_CAPABILITY_POLICY_VERSION);
        assert!(keys.insert(format!(
            "{}:{}",
            record.adapter.as_str(),
            record.capability.as_str()
        )));
        source_kinds.insert(format!("{:?}", record.source_kind));
        match record.source_kind {
            ChannelCapabilitySourceKind::OfficialContract => {
                assert!(record.source_ref.starts_with("https://"));
            }
            ChannelCapabilitySourceKind::LocalSafetyPolicy => {
                assert!(record.source_ref.starts_with("policy:"));
            }
            ChannelCapabilitySourceKind::ExperimentalInference => {
                assert!(record.source_ref.starts_with("evidence:"));
            }
        }
        if record.capability != ChannelCapabilityKind::SendText
            && record.adapter != ChannelAdapterKind::WebUi
        {
            assert!(record.max_payload_bytes.is_some());
        }
    }

    assert_eq!(source_kinds.len(), 3);
}

#[test]
fn official_and_local_media_limits_are_read_only_catalog_values() {
    let cases = [
        (
            ChannelAdapterKind::TelegramBot,
            ChannelCapabilityKind::SendImage,
            10 * MIB,
            ChannelCapabilitySourceKind::OfficialContract,
        ),
        (
            ChannelAdapterKind::TelegramBot,
            ChannelCapabilityKind::SendFile,
            50 * MIB,
            ChannelCapabilitySourceKind::OfficialContract,
        ),
        (
            ChannelAdapterKind::WhatsappCloud,
            ChannelCapabilityKind::SendImage,
            5 * MIB,
            ChannelCapabilitySourceKind::OfficialContract,
        ),
        (
            ChannelAdapterKind::WhatsappCloud,
            ChannelCapabilityKind::SendFile,
            100 * MIB,
            ChannelCapabilitySourceKind::OfficialContract,
        ),
        (
            ChannelAdapterKind::WechatIlink,
            ChannelCapabilityKind::SendImage,
            25 * MIB,
            ChannelCapabilitySourceKind::LocalSafetyPolicy,
        ),
        (
            ChannelAdapterKind::FeishuOpenPlatform,
            ChannelCapabilityKind::SendFile,
            30 * MIB,
            ChannelCapabilitySourceKind::OfficialContract,
        ),
    ];

    for (adapter, capability, max_bytes, source_kind) in cases {
        let record = channel_capability(adapter, capability).expect("catalog record");
        assert_eq!(record.max_payload_bytes, Some(max_bytes));
        assert_eq!(record.source_kind, source_kind);
        assert_eq!(
            channel_media_max_bytes(adapter, capability),
            Some(max_bytes)
        );
    }
}

#[test]
fn every_active_adapter_has_an_explicit_text_contract() {
    for adapter in [
        ChannelAdapterKind::TelegramBot,
        ChannelAdapterKind::WhatsappCloud,
        ChannelAdapterKind::WhatsappWeb,
        ChannelAdapterKind::WechatIlink,
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelAdapterKind::WebUi,
    ] {
        assert!(channel_capability(adapter, ChannelCapabilityKind::SendText).is_some());
    }
}

#[test]
fn wechat_wire_contract_is_official_but_unknown_size_limits_are_local_policy() {
    let text = channel_capability(
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendText,
    )
    .expect("wechat text contract");
    assert_eq!(
        text.source_kind,
        ChannelCapabilitySourceKind::OfficialContract
    );
    assert!(text.source_ref.contains("Tencent/openclaw-weixin"));

    for capability in [
        ChannelCapabilityKind::SendImage,
        ChannelCapabilityKind::SendVideo,
        ChannelCapabilityKind::SendFile,
    ] {
        let media = channel_capability(ChannelAdapterKind::WechatIlink, capability)
            .expect("wechat media policy");
        assert_eq!(
            media.source_kind,
            ChannelCapabilitySourceKind::LocalSafetyPolicy
        );
        assert!(media.source_ref.starts_with("policy:"));
    }
}

#[test]
fn whatsapp_upload_specs_are_constrained_by_the_catalog_contract() {
    use std::path::Path;

    use crate::channel_media_limits::{whatsapp_cloud_upload_spec, WhatsappCloudMediaKind};

    for (path, media_kind, capability_kind) in [
        (
            "photo.jpg",
            WhatsappCloudMediaKind::Image,
            ChannelCapabilityKind::SendImage,
        ),
        (
            "clip.mp4",
            WhatsappCloudMediaKind::Video,
            ChannelCapabilityKind::SendVideo,
        ),
        (
            "voice.opus",
            WhatsappCloudMediaKind::Audio,
            ChannelCapabilityKind::SendAudio,
        ),
        (
            "report.pdf",
            WhatsappCloudMediaKind::Document,
            ChannelCapabilityKind::SendFile,
        ),
    ] {
        let (mime_type, max_bytes, _) =
            whatsapp_cloud_upload_spec(Path::new(path), media_kind).expect("upload spec");
        let record = channel_capability(ChannelAdapterKind::WhatsappCloud, capability_kind)
            .expect("catalog record");
        assert!(record.accepted_mime_types.contains(&mime_type));
        assert_eq!(record.max_payload_bytes, Some(max_bytes));
    }
}

#[test]
fn whatsapp_web_media_limits_are_local_policy_not_cloud_contracts() {
    for (capability, expected_bytes) in [
        (ChannelCapabilityKind::SendImage, 100 * MIB),
        (ChannelCapabilityKind::SendVideo, 100 * MIB),
        (ChannelCapabilityKind::SendAudio, 100 * MIB),
        (ChannelCapabilityKind::SendFile, 2 * 1024 * MIB),
    ] {
        let record = channel_capability(ChannelAdapterKind::WhatsappWeb, capability)
            .expect("WhatsApp Web local policy record");
        assert_eq!(record.max_payload_bytes, Some(expected_bytes));
        assert_eq!(
            record.source_kind,
            ChannelCapabilitySourceKind::LocalSafetyPolicy
        );
        assert!(record.source_ref.starts_with("policy:"));
        assert!(!record.source_ref.contains("meta"));
    }

    let text = channel_capability(
        ChannelAdapterKind::WhatsappWeb,
        ChannelCapabilityKind::SendText,
    )
    .expect("WhatsApp Web experimental text record");
    assert_eq!(
        text.source_kind,
        ChannelCapabilitySourceKind::ExperimentalInference
    );
    assert!(text.source_ref.starts_with("evidence:"));
}
