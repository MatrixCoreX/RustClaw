use super::*;

pub(super) fn call_vendor_vision(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    request: VisionRequest<'_>,
) -> Result<(String, String, &'static str), String> {
    let mode = resolve_adapter_mode(&cfg.image_vision);
    let (vendor_name, vcfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &vcfg.api_key)?;
    match vendor {
        VendorKind::OpenAI => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build openai client failed: {err}"))?;
            let text = openai_vision(&client, &vcfg, &model, request)?;
            Ok((text, model, "native"))
        }
        VendorKind::Google => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build google client failed: {err}"))?;
            let text = google_vision(&client, &vcfg, &model, request)?;
            Ok((text, model, "native"))
        }
        VendorKind::Anthropic => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build anthropic client failed: {err}"))?;
            let text = anthropic_vision(&client, &vcfg, &model, request)?;
            Ok((text, model, "native"))
        }
        VendorKind::Grok | VendorKind::DeepSeek => {
            if mode == AdapterMode::Native {
                return Err(format!(
                    "{vendor_name} native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                ));
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            let text = openai_compat_vision(
                &client,
                &vcfg,
                &model,
                request,
                OpenAiCompatOptions::new(vendor_name),
            )?;
            Ok((text, model, "compat"))
        }
        VendorKind::Mimo => {
            if mode == AdapterMode::Native {
                return Err(
                    "mimo native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build mimo client failed: {err}"))?;
            let text = mimo_vision(&client, &vcfg, &model, request)?;
            Ok((text, model, "compat"))
        }
        VendorKind::MiniMax => {
            if mode == AdapterMode::Native {
                return Err(
                    "minimax native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build minimax client failed: {err}"))?;
            let text = openai_compat_vision(
                &client,
                &vcfg,
                &model,
                request,
                OpenAiCompatOptions::new(vendor_name),
            )?;
            Ok((text, model, "compat"))
        }
        VendorKind::Qwen => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build qwen client failed: {err}"))?;
            if mode == AdapterMode::Native {
                return Err(
                    "qwen native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let text = openai_compat_vision(
                &client,
                &vcfg,
                &model,
                request,
                OpenAiCompatOptions::new(vendor_name),
            )?;
            Ok((text, model, "compat"))
        }
    }
}

pub(super) fn resolve_adapter_mode(cfg: &ImageSkillConfig) -> AdapterMode {
    match cfg
        .adapter_mode
        .as_deref()
        .map(str::trim)
        .unwrap_or("auto")
        .to_ascii_lowercase()
        .as_str()
    {
        "native" => AdapterMode::Native,
        "compat" | "compatible" => AdapterMode::Compat,
        _ => AdapterMode::Auto,
    }
}

pub(super) fn openai_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    request: VisionRequest<'_>,
) -> Result<String, String> {
    openai_compat_vision(
        client,
        cfg,
        model,
        request,
        OpenAiCompatOptions {
            error_label: "openai",
            include_api_key_header: false,
            supports_image_detail: true,
        },
    )
}

pub(super) fn mimo_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    request: VisionRequest<'_>,
) -> Result<String, String> {
    openai_compat_vision(
        client,
        cfg,
        model,
        request,
        OpenAiCompatOptions {
            error_label: "mimo",
            include_api_key_header: true,
            supports_image_detail: false,
        },
    )
}

#[derive(Debug, Clone, Copy)]
struct OpenAiCompatOptions<'a> {
    error_label: &'a str,
    include_api_key_header: bool,
    supports_image_detail: bool,
}

impl<'a> OpenAiCompatOptions<'a> {
    fn new(error_label: &'a str) -> Self {
        Self {
            error_label,
            include_api_key_header: false,
            supports_image_detail: false,
        }
    }
}

fn openai_compat_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    request: VisionRequest<'_>,
    options: OpenAiCompatOptions<'_>,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":request.prompt})];
    for image in request.images {
        let url = match image {
            ImageSource::Url(s) => s.to_string(),
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > request.max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                format!("data:{mime};base64,{}", STANDARD.encode(bytes))
            }
            ImageSource::Base64(s) => normalize_base64_image(s),
        };
        let image_url = if request.options.exact_text && options.supports_image_detail {
            json!({"url":url,"detail":"high"})
        } else {
            json!({"url":url})
        };
        content.push(json!({"type":"image_url","image_url":image_url}));
    }
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":content}],
        "temperature": if request.options.exact_text { 0.0 } else { 0.2 }
    });
    let url = format!("{}/chat/completions", trim_trailing_slash(&cfg.base_url));
    let mut http_request = client.post(url).bearer_auth(&cfg.api_key);
    if options.include_api_key_header {
        http_request = http_request.header("api-key", &cfg.api_key);
    }
    let resp = http_request
        .json(&body)
        .send()
        .map_err(|err| format!("{} request failed: {err}", options.error_label))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse openai response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "{} error status={status}: {}",
            options.error_label,
            provider_error_excerpt(&v, 400)
        ));
    }
    if let Some(s) = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
    {
        return Ok(s.to_string());
    }
    Err(format!(
        "{} response missing text: {}",
        options.error_label,
        provider_error_excerpt(&v, 400)
    ))
}

pub(super) fn google_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    request: VisionRequest<'_>,
) -> Result<String, String> {
    let mut parts = vec![json!({"text":request.prompt})];
    for image in request.images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > request.max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                parts.push(json!({"inline_data":{"mime_type":mime,"data":STANDARD.encode(bytes)}}));
            }
            ImageSource::Base64(s) => {
                let (mime, data) = split_image_data(s);
                parts.push(json!({"inline_data":{"mime_type":mime,"data":data}}));
            }
            ImageSource::Url(u) => {
                parts.push(json!({"text": format!("Image URL: {u}")}));
            }
        }
    }
    let body = json!({
        "contents":[{"parts":parts}],
        "generationConfig": {
            "temperature": if request.options.exact_text { 0.0 } else { 0.2 }
        }
    });
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        trim_trailing_slash(&cfg.base_url),
        model,
        cfg.api_key
    );
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|err| format!("google request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse google response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "google error status={status}: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    let mut out = String::new();
    if let Some(parts) = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "google response missing text: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    Ok(out)
}

pub(super) fn anthropic_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    request: VisionRequest<'_>,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":request.prompt})];
    for image in request.images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > request.max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                content.push(json!({
                    "type":"image",
                    "source":{"type":"base64","media_type":mime,"data":STANDARD.encode(bytes)}
                }));
            }
            ImageSource::Base64(s) => {
                let (mime, data) = split_image_data(s);
                content.push(json!({
                    "type":"image",
                    "source":{"type":"base64","media_type":mime,"data":data}
                }));
            }
            ImageSource::Url(u) => {
                content.push(json!({"type":"text","text":format!("Image URL reference: {u}")}));
            }
        }
    }
    let body = json!({
        "model": model,
        "max_tokens": if request.options.exact_text { 8192 } else { 2048 },
        "temperature": if request.options.exact_text { 0.0 } else { 0.2 },
        "messages": [{"role":"user","content":content}]
    });
    let url = format!("{}/messages", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .map_err(|err| format!("anthropic request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse anthropic response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "anthropic error status={status}: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    let mut out = String::new();
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        for item in arr {
            if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "anthropic response missing text: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    Ok(out)
}
