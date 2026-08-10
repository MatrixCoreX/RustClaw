//! HTTP + CDN helpers for Weixin ilink bots (OpenClaw weixin plugin alignment).
pub mod cdn;
pub mod contract;
pub mod crypto;
pub mod http;

pub use cdn::{
    build_cdn_download_url, build_cdn_upload_url, download_decrypted_media,
    download_remote_media_to_temp, fetch_cdn_bytes, media_aes_key_b64_from_hex,
    send_weixin_file_from_file, send_weixin_file_from_file_with_client_id,
    send_weixin_image_from_file, send_weixin_image_from_file_with_client_id,
    send_weixin_video_from_file, send_weixin_video_from_file_with_client_id,
    upload_plaintext_to_cdn, GetUploadUrlReq, GetUploadUrlResp, UploadedCdnBlob,
};
pub use contract::{
    new_wechat_client_id, WechatCdnMedia, WechatConversationScope, WechatMessageItem,
    WechatSendMessageRequest, MESSAGE_ITEM_FILE, MESSAGE_ITEM_IMAGE, MESSAGE_ITEM_TEXT,
    MESSAGE_ITEM_VIDEO, MESSAGE_ITEM_VOICE, MESSAGE_STATE_FINISH, MESSAGE_STATE_GENERATING,
    MESSAGE_STATE_NEW, TYPING_STATUS_CANCEL, TYPING_STATUS_TYPING, UPLOAD_MEDIA_TYPE_FILE,
    UPLOAD_MEDIA_TYPE_IMAGE, UPLOAD_MEDIA_TYPE_VIDEO, UPLOAD_MEDIA_TYPE_VOICE,
    WECHAT_ILINK_ADAPTER, WECHAT_ILINK_CONTRACT_SOURCE, WECHAT_ILINK_CONTRACT_VERIFIED_AT,
};
pub use crypto::{
    aes_ecb_padded_size, decrypt_aes_128_ecb, encrypt_aes_128_ecb, parse_aes_key_base64,
    parse_aes_key_hex_or_base64_media,
};
pub use http::{base_info, decode_ilink_provider_failure, post_ilink_json, IlinkAuth};
