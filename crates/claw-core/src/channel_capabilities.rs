//! Auditable, channel-neutral capability catalog for outbound adapters.
//!
//! A capability's upstream contract, local safety policy, and experimental
//! evidence are different authorities.  Keep that provenance explicit so a
//! local guard is never presented as a provider guarantee.

use serde::Serialize;

use crate::types::ChannelKind;

pub const CHANNEL_CAPABILITY_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_CAPABILITY_POLICY_VERSION: &str = "channel-capability-policy-v1";
pub const CHANNEL_CAPABILITY_VERIFIED_AT: &str = "2026-07-31";
pub const MIB: u64 = 1024 * 1024;

const TELEGRAM_DOCS: &str = "https://core.telegram.org/bots/api#sending-files";
const WHATSAPP_CLOUD_DOCS: &str =
    "https://www.postman.com/meta/whatsapp-business-platform/folder/13382743-ecb27be5-4d27-4763-bbee-6a8002c04bf3";
const WHATSAPP_CLOUD_MESSAGE_DOCS: &str =
    "https://www.postman.com/meta/whatsapp-business-platform/folder/o48mro7/messages";
const FEISHU_FILE_DOCS: &str =
    "https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/file/create";
const FEISHU_MESSAGE_DOCS: &str =
    "https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/create";
const LARK_FILE_DOCS: &str = "https://open.larksuite.com/document/server-docs/im-v1/file/create";
const LARK_MESSAGE_DOCS: &str =
    "https://open.larksuite.com/document/server-docs/im-v1/message/create";
const WECHAT_ILINK_DOCS: &str = "https://github.com/Tencent/openclaw-weixin#backend-api-protocol";
const LOCAL_MEDIA_POLICY: &str = "policy:channel-media-safety-v1";
const LOCAL_UI_POLICY: &str = "policy:web-ui-delivery-v1";
const WHATSAPP_WEB_EVIDENCE: &str = "evidence:whatsapp-web-bridge-smoke-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelAdapterKind {
    TelegramBot,
    WhatsappCloud,
    WhatsappWeb,
    WechatIlink,
    FeishuOpenPlatform,
    LarkOpenPlatform,
    WebUi,
}

impl ChannelAdapterKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TelegramBot => "telegram_bot",
            Self::WhatsappCloud => "whatsapp_cloud",
            Self::WhatsappWeb => "whatsapp_web",
            Self::WechatIlink => "wechat_ilink",
            Self::FeishuOpenPlatform => "feishu_open_platform",
            Self::LarkOpenPlatform => "lark_open_platform",
            Self::WebUi => "web_ui",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCapabilityKind {
    SendText,
    SendImage,
    SendVideo,
    SendAudio,
    SendFile,
}

impl ChannelCapabilityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SendText => "send_text",
            Self::SendImage => "send_image",
            Self::SendVideo => "send_video",
            Self::SendAudio => "send_audio",
            Self::SendFile => "send_file",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCapabilitySourceKind {
    OfficialContract,
    LocalSafetyPolicy,
    ExperimentalInference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChannelCapabilityRecord {
    pub schema_version: u16,
    pub channel: ChannelKind,
    pub adapter: ChannelAdapterKind,
    pub capability: ChannelCapabilityKind,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_payload_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_text_chars: Option<u64>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub accepted_mime_types: &'static [&'static str],
    pub source_kind: ChannelCapabilitySourceKind,
    pub source_ref: &'static str,
    pub verified_at: &'static str,
    pub policy_version: &'static str,
}

const fn capability(
    channel: ChannelKind,
    adapter: ChannelAdapterKind,
    capability: ChannelCapabilityKind,
    max_payload_bytes: Option<u64>,
    max_text_chars: Option<u64>,
    accepted_mime_types: &'static [&'static str],
    source_kind: ChannelCapabilitySourceKind,
    source_ref: &'static str,
) -> ChannelCapabilityRecord {
    ChannelCapabilityRecord {
        schema_version: CHANNEL_CAPABILITY_SCHEMA_VERSION,
        channel,
        adapter,
        capability,
        supported: true,
        max_payload_bytes,
        max_text_chars,
        accepted_mime_types,
        source_kind,
        source_ref,
        verified_at: CHANNEL_CAPABILITY_VERIFIED_AT,
        policy_version: CHANNEL_CAPABILITY_POLICY_VERSION,
    }
}

static CHANNEL_CAPABILITY_CATALOG: &[ChannelCapabilityRecord] = &[
    capability(
        ChannelKind::Telegram,
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendText,
        None,
        Some(4096),
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        TELEGRAM_DOCS,
    ),
    capability(
        ChannelKind::Telegram,
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendImage,
        Some(10 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        TELEGRAM_DOCS,
    ),
    capability(
        ChannelKind::Telegram,
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendVideo,
        Some(50 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        TELEGRAM_DOCS,
    ),
    capability(
        ChannelKind::Telegram,
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendAudio,
        Some(50 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        TELEGRAM_DOCS,
    ),
    capability(
        ChannelKind::Telegram,
        ChannelAdapterKind::TelegramBot,
        ChannelCapabilityKind::SendFile,
        Some(50 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        TELEGRAM_DOCS,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappCloud,
        ChannelCapabilityKind::SendText,
        None,
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        WHATSAPP_CLOUD_MESSAGE_DOCS,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappCloud,
        ChannelCapabilityKind::SendImage,
        Some(5 * MIB),
        None,
        &["image/jpeg", "image/png"],
        ChannelCapabilitySourceKind::OfficialContract,
        WHATSAPP_CLOUD_DOCS,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappCloud,
        ChannelCapabilityKind::SendVideo,
        Some(16 * MIB),
        None,
        &["video/mp4", "video/3gpp"],
        ChannelCapabilitySourceKind::OfficialContract,
        WHATSAPP_CLOUD_DOCS,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappCloud,
        ChannelCapabilityKind::SendAudio,
        Some(16 * MIB),
        None,
        &[
            "audio/aac",
            "audio/mp4",
            "audio/mpeg",
            "audio/amr",
            "audio/ogg",
        ],
        ChannelCapabilitySourceKind::OfficialContract,
        WHATSAPP_CLOUD_DOCS,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappCloud,
        ChannelCapabilityKind::SendFile,
        Some(100 * MIB),
        None,
        &[
            "text/plain",
            "application/pdf",
            "application/vnd.ms-powerpoint",
            "application/msword",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ],
        ChannelCapabilitySourceKind::OfficialContract,
        WHATSAPP_CLOUD_DOCS,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappWeb,
        ChannelCapabilityKind::SendText,
        None,
        None,
        &[],
        ChannelCapabilitySourceKind::ExperimentalInference,
        WHATSAPP_WEB_EVIDENCE,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappWeb,
        ChannelCapabilityKind::SendImage,
        Some(100 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappWeb,
        ChannelCapabilityKind::SendVideo,
        Some(100 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappWeb,
        ChannelCapabilityKind::SendAudio,
        Some(100 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Whatsapp,
        ChannelAdapterKind::WhatsappWeb,
        ChannelCapabilityKind::SendFile,
        Some(2 * 1024 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Wechat,
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendText,
        None,
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        WECHAT_ILINK_DOCS,
    ),
    capability(
        ChannelKind::Wechat,
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendImage,
        Some(25 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Wechat,
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendVideo,
        Some(100 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Wechat,
        ChannelAdapterKind::WechatIlink,
        ChannelCapabilityKind::SendFile,
        Some(100 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_MEDIA_POLICY,
    ),
    capability(
        ChannelKind::Feishu,
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendText,
        Some(150 * 1024),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        FEISHU_MESSAGE_DOCS,
    ),
    capability(
        ChannelKind::Feishu,
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendImage,
        Some(10 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        FEISHU_FILE_DOCS,
    ),
    capability(
        ChannelKind::Feishu,
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendVideo,
        Some(30 * MIB),
        None,
        &["video/mp4"],
        ChannelCapabilitySourceKind::OfficialContract,
        FEISHU_FILE_DOCS,
    ),
    capability(
        ChannelKind::Feishu,
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendAudio,
        Some(30 * MIB),
        None,
        &["audio/opus"],
        ChannelCapabilitySourceKind::OfficialContract,
        FEISHU_FILE_DOCS,
    ),
    capability(
        ChannelKind::Feishu,
        ChannelAdapterKind::FeishuOpenPlatform,
        ChannelCapabilityKind::SendFile,
        Some(30 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        FEISHU_FILE_DOCS,
    ),
    capability(
        ChannelKind::Lark,
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendText,
        Some(150 * 1024),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        LARK_MESSAGE_DOCS,
    ),
    capability(
        ChannelKind::Lark,
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendImage,
        Some(10 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        LARK_FILE_DOCS,
    ),
    capability(
        ChannelKind::Lark,
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendVideo,
        Some(30 * MIB),
        None,
        &["video/mp4"],
        ChannelCapabilitySourceKind::OfficialContract,
        LARK_FILE_DOCS,
    ),
    capability(
        ChannelKind::Lark,
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendAudio,
        Some(30 * MIB),
        None,
        &["audio/opus"],
        ChannelCapabilitySourceKind::OfficialContract,
        LARK_FILE_DOCS,
    ),
    capability(
        ChannelKind::Lark,
        ChannelAdapterKind::LarkOpenPlatform,
        ChannelCapabilityKind::SendFile,
        Some(30 * MIB),
        None,
        &[],
        ChannelCapabilitySourceKind::OfficialContract,
        LARK_FILE_DOCS,
    ),
    capability(
        ChannelKind::Ui,
        ChannelAdapterKind::WebUi,
        ChannelCapabilityKind::SendText,
        None,
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_UI_POLICY,
    ),
    capability(
        ChannelKind::Ui,
        ChannelAdapterKind::WebUi,
        ChannelCapabilityKind::SendFile,
        None,
        None,
        &[],
        ChannelCapabilitySourceKind::LocalSafetyPolicy,
        LOCAL_UI_POLICY,
    ),
];

pub fn channel_capability_catalog() -> &'static [ChannelCapabilityRecord] {
    CHANNEL_CAPABILITY_CATALOG
}

pub fn channel_capability(
    adapter: ChannelAdapterKind,
    capability: ChannelCapabilityKind,
) -> Option<&'static ChannelCapabilityRecord> {
    CHANNEL_CAPABILITY_CATALOG
        .iter()
        .find(|record| record.adapter == adapter && record.capability == capability)
}

pub fn channel_media_max_bytes(
    adapter: ChannelAdapterKind,
    capability: ChannelCapabilityKind,
) -> Option<u64> {
    channel_capability(adapter, capability)
        .filter(|record| record.supported)
        .and_then(|record| record.max_payload_bytes)
}

#[cfg(test)]
#[path = "channel_capabilities_tests.rs"]
mod tests;
