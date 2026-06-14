use super::*;

pub(super) fn call_vendor_vision(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
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
            let text = openai_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
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
            let text = google_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
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
            let text = anthropic_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
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
                prompt,
                images,
                max_input_bytes,
                vendor_name,
                false,
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
            let text = mimo_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
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
            if let Ok(text) = minimax_mcp_vision(&vcfg, prompt, images, timeout_seconds) {
                return Ok((text, model, "mcp"));
            }
            let text = minimax_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
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
                prompt,
                images,
                max_input_bytes,
                vendor_name,
                false,
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
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    openai_compat_vision(
        client,
        cfg,
        model,
        prompt,
        images,
        max_input_bytes,
        "openai",
        false,
    )
}

pub(super) fn minimax_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut content = String::from(prompt);
    for (idx, image) in images.iter().enumerate() {
        let encoded = image_base64_payload(image, max_input_bytes)?;
        content.push_str("\n\nimage ");
        content.push_str(&(idx + 1).to_string());
        content.push_str(":\n[图片base64:");
        content.push_str(&encoded);
        content.push(']');
    }
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":content}],
        "temperature": 0.2
    });
    let url = format!("{}/chat/completions", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .map_err(|err| format!("minimax request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse minimax response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "minimax error status={status}: {}",
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
        "minimax response missing text: {}",
        provider_error_excerpt(&v, 400)
    ))
}

pub(super) fn minimax_mcp_vision(
    cfg: &VendorConfig,
    prompt: &str,
    images: &[ImageSource],
    timeout_seconds: u64,
) -> Result<String, String> {
    if images.len() != 1 {
        return Err("minimax mcp image understanding supports one image per call".to_string());
    }
    let (image_arg, cleanup_path) = image_source_for_minimax_mcp(&images[0])?;
    let mut cmd = Command::new("npx");
    cmd.arg("-y")
        .arg("@jayjanii/pi-minimax-mcp")
        .arg("understand")
        .arg(&image_arg)
        .arg("--prompt")
        .arg(prompt)
        .env("MINIMAX_API_KEY", &cfg.api_key)
        .env("MINIMAX_API_HOST", minimax_mcp_api_host(&cfg.base_url))
        .env(
            "MINIMAX_MCP_STARTUP_TIMEOUT_MS",
            std::env::var("MINIMAX_MCP_STARTUP_TIMEOUT_MS").unwrap_or_else(|_| "60000".to_string()),
        )
        .env(
            "MINIMAX_MCP_TIMEOUT_MS",
            std::env::var("MINIMAX_MCP_TIMEOUT_MS")
                .unwrap_or_else(|_| (timeout_seconds.max(60) * 1000).to_string()),
        );
    if let Some(path) = path_with_local_uvx() {
        cmd.env("PATH", path);
    }
    if std::env::var_os("MINIMAX_MCP_UV_PATH").is_none() {
        if let Some(uvx) = default_uvx_path() {
            cmd.env("MINIMAX_MCP_UV_PATH", uvx);
        }
    }
    let output = cmd
        .output()
        .map_err(|err| format!("minimax mcp launch failed: {err}"));
    if let Some(path) = cleanup_path {
        let _ = std::fs::remove_file(path);
    }
    let output = output?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "minimax mcp failed status={}: {}{}",
        output
            .status
            .code()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        redact_sensitive_inline(&truncate(&stderr, 600)),
        if stdout.is_empty() {
            String::new()
        } else {
            format!(
                " stdout={}",
                redact_sensitive_inline(&truncate(&stdout, 300))
            )
        }
    ))
}

pub(super) fn image_source_for_minimax_mcp(
    image: &ImageSource,
) -> Result<(String, Option<PathBuf>), String> {
    match image {
        ImageSource::Url(s) => Ok((s.to_string(), None)),
        ImageSource::Path(p) => Ok((p.to_string_lossy().to_string(), None)),
        ImageSource::Base64(s) => {
            let path = std::env::temp_dir().join(format!(
                "rustclaw-image-vision-{}-{}.png",
                std::process::id(),
                monotonic_millis()
            ));
            let data = STANDARD
                .decode(strip_base64_data_url(s))
                .map_err(|err| format!("decode base64 image failed: {err}"))?;
            std::fs::write(&path, data).map_err(|err| format!("write temp image failed: {err}"))?;
            Ok((path.to_string_lossy().to_string(), Some(path)))
        }
    }
}

pub(super) fn minimax_mcp_api_host(base_url: &str) -> String {
    let trimmed = trim_trailing_slash(base_url);
    trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed.as_str())
        .to_string()
}

pub(super) fn path_with_local_uvx() -> Option<String> {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").ok()?;
    let local_bin = format!("{home}/.local/bin");
    if current.split(':').any(|part| part == local_bin) {
        Some(current)
    } else if Path::new(&local_bin).is_dir() {
        Some(format!("{local_bin}:{current}"))
    } else {
        Some(current)
    }
}

pub(super) fn default_uvx_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let uvx = Path::new(&home).join(".local/bin/uvx");
    uvx.exists().then(|| uvx.to_string_lossy().to_string())
}

pub(super) fn monotonic_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis())
        .unwrap_or(0)
}

pub(super) fn mimo_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    openai_compat_vision(
        client,
        cfg,
        model,
        prompt,
        images,
        max_input_bytes,
        "mimo",
        true,
    )
}

pub(super) fn image_base64_payload(
    image: &ImageSource,
    max_input_bytes: usize,
) -> Result<String, String> {
    match image {
        ImageSource::Url(s) => Ok(s.to_string()),
        ImageSource::Path(p) => {
            let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            Ok(STANDARD.encode(bytes))
        }
        ImageSource::Base64(s) => Ok(strip_base64_data_url(s).to_string()),
    }
}

pub(super) fn openai_compat_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
    error_label: &str,
    include_api_key_header: bool,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        let url = match image {
            ImageSource::Url(s) => s.to_string(),
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                format!("data:{mime};base64,{}", STANDARD.encode(bytes))
            }
            ImageSource::Base64(s) => normalize_base64_image(s),
        };
        content.push(json!({"type":"image_url","image_url":{"url":url}}));
    }
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":content}],
        "temperature": 0.2
    });
    let url = format!("{}/chat/completions", trim_trailing_slash(&cfg.base_url));
    let mut request = client.post(url).bearer_auth(&cfg.api_key);
    if include_api_key_header {
        request = request.header("api-key", &cfg.api_key);
    }
    let resp = request
        .json(&body)
        .send()
        .map_err(|err| format!("{error_label} request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse openai response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "{error_label} error status={status}: {}",
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
        "{error_label} response missing text: {}",
        provider_error_excerpt(&v, 400)
    ))
}

pub(super) fn google_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut parts = vec![json!({"text":prompt})];
    for image in images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
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
    let body = json!({"contents":[{"parts":parts}]});
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
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
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
        "max_tokens": 1024,
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
