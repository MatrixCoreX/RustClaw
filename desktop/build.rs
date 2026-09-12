#[allow(dead_code)]
#[path = "../crates/claw-core/src/product_identity.rs"]
mod product_identity;

fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    if std::env::var_os("APP_PRODUCT_IDENTITY_CONFIG").is_none() {
        std::env::set_var(
            "APP_PRODUCT_IDENTITY_CONFIG",
            root.join("../configs/product_identity.toml"),
        );
    }
    println!("cargo:rerun-if-env-changed=APP_PRODUCT_IDENTITY_CONFIG");
    println!(
        "cargo:rerun-if-changed={}",
        std::env::var("APP_PRODUCT_IDENTITY_CONFIG").unwrap()
    );
    println!("cargo:rerun-if-changed=../crates/claw-core/src/product_identity.rs");
    println!(
        "cargo:rustc-env=DESKTOP_DISPLAY_NAME={}",
        product_identity::product_identity().display_name()
    );
    if std::env::var_os("CARGO_FEATURE_GUI").is_some() {
        tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "profiles",
                "media_open",
                "media_close",
                "add_profile",
                "forget_profile",
                "connect_device",
                "login",
                "login_prefill",
                "disconnect_device",
                "request_start",
                "request_headers",
                "request_read",
                "request_cancel",
                "upload_chunk",
                "upload_finish",
                "download",
                "open_external",
                "aipp_open",
                "aipp_context",
                "aipp_bridge",
                "current_session",
                "discover_devices",
                "cancel_discovery",
                "download_cancel",
                "wallet_open",
                "wallet_status",
                "wallet_lock",
                "wallet_select",
                "wallet_capabilities",
                "wallet_read",
                "wallet_prepare",
                "wallet_operations",
                "wallet_check_operation",
                "wallet_initialize",
                "wallet_unlock",
                "wallet_create",
                "wallet_backup",
                "wallet_restore",
                "wallet_pending",
                "wallet_cancel_operation",
                "wallet_confirm",
                "wallet_nodes",
                "wallet_add_node",
                "wallet_connect_node",
                "wallet_prefer_node",
                "wallet_disconnect_node",
                "wallet_market_read",
            ]),
        ))
        .expect("desktop build configuration");
    }
}
