use reqwest::Url;

fn plain_authority(url: &Url) -> bool {
    url.username().is_empty() && url.password().is_none() && url.port().is_none()
}

pub fn local_page(url: &Url, path: &str) -> bool {
    plain_authority(url)
        && ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost")))
        && (url.path() == path || (path == "/index.html" && matches!(url.path(), "" | "/")))
}

pub fn device_asset(url: &Url, prefix: &str) -> bool {
    plain_authority(url)
        && ((url.scheme() == "device" && url.host_str() == Some("localhost"))
            || (url.scheme() == "http" && url.host_str() == Some("device.localhost")))
        && url.path().starts_with(prefix)
}

// WebView2 maps the registered custom protocol to this exact HTTP origin.
// Other platforms use device://localhost. Neither permits arbitrary network origins.
pub fn asset_csp() -> String {
    let origin = if cfg!(target_os = "windows") {
        "http://device.localhost"
    } else {
        "device:"
    };
    format!("default-src 'none'; script-src {origin} 'unsafe-inline'; style-src {origin} 'unsafe-inline'; img-src {origin} data: blob:; media-src {origin} blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
}
