//! HTTP + CDN helpers for Weixin ilink bots (OpenClaw weixin plugin alignment).
pub mod cdn;
pub mod crypto;
pub mod http;

pub use cdn::{
    build_cdn_download_url, build_cdn_upload_url, download_decrypted_media,
    download_remote_media_to_temp, fetch_cdn_bytes, media_aes_key_b64_from_hex,
    send_weixin_file_from_file, send_weixin_image_from_file, send_weixin_video_from_file,
    upload_plaintext_to_cdn, GetUploadUrlReq, GetUploadUrlResp, UploadedCdnBlob,
};
pub use crypto::{
    aes_ecb_padded_size, decrypt_aes_128_ecb, encrypt_aes_128_ecb, parse_aes_key_base64,
    parse_aes_key_hex_or_base64_media,
};
pub use http::{base_info, post_ilink_json, IlinkAuth};
