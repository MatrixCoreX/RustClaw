use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn call_edit(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    quality: Option<&str>,
    n: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(String, &'static str), String> {
    let mode = resolve_adapter_mode(&cfg.image_edit);
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
            openai_compatible_edit(
                &client,
                "openai",
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok((model, "compat"))
        }
        VendorKind::Google => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build google client failed: {err}"))?;
            google_edit(
                &client,
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok((model, "native"))
        }
        VendorKind::Anthropic => {
            if mode == AdapterMode::Native {
                return Err("anthropic native image edit adapter is not available".to_string());
            }
            if !cfg.image_edit.allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(
                    "anthropic native image edit adapter is not available; set image_edit.allow_compat_adapters=true to use compatible endpoint"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build anthropic client failed: {err}"))?;
            openai_compatible_edit(
                &client,
                "anthropic",
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok((model, "compat"))
        }
        VendorKind::Grok | VendorKind::DeepSeek => {
            if mode == AdapterMode::Native {
                return Err(format!(
                    "{vendor_name} native image edit adapter is not available"
                ));
            }
            if !cfg.image_edit.allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(
                    format!(
                        "{vendor_name} native image edit adapter is not available; set image_edit.allow_compat_adapters=true to use compatible endpoint"
                    ),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            openai_compatible_edit(
                &client,
                vendor_name,
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok((model, "compat"))
        }
        VendorKind::MiniMax => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            minimax_reference_edit(
                &client,
                vcfg,
                &model,
                instruction,
                image,
                size,
                quality,
                n,
                output_path,
            )?;
            Ok((model, "native_reference"))
        }
        VendorKind::Qwen => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build qwen client failed: {err}"))?;
            let can_use_native_inputs =
                qwen_native_edit_inputs_supported(&cfg.image_edit, &model, image, mask);
            if should_use_qwen_native_edit(
                &cfg.image_edit,
                &model,
                mode,
                cfg.image_edit.allow_compat_adapters,
                can_use_native_inputs,
            ) {
                qwen_native_edit(
                    &client,
                    &cfg.image_edit,
                    cfg.image_edit.qwen_native_base_url.as_deref(),
                    cfg.image_edit.qwen_native_function.as_deref(),
                    &vcfg.api_key,
                    &model,
                    instruction,
                    image,
                    mask,
                    size,
                    n,
                    timeout_seconds,
                    max_input_bytes,
                    output_path,
                )?;
                return Ok((model, "native"));
            } else {
                if !cfg.image_edit.allow_compat_adapters {
                    return Err(
                        "qwen native image edit adapter is not available; set image_edit.allow_compat_adapters=true to use compatible endpoint"
                            .to_string(),
                    );
                }
                openai_compatible_edit(
                    &client,
                    "qwen",
                    vcfg,
                    &model,
                    instruction,
                    image,
                    mask,
                    size,
                    quality,
                    n,
                    max_input_bytes,
                    output_path,
                )?;
                Ok((model, "compat"))
            }
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

pub(super) fn qwen_uses_native_edit_api(cfg: &ImageSkillConfig, model: &str) -> bool {
    let requested = model.trim();
    cfg.native_models
        .as_ref()
        .and_then(|list| {
            list.iter().map(|s| s.trim()).find(|candidate| {
                !candidate.is_empty() && candidate.eq_ignore_ascii_case(requested)
            })
        })
        .is_some()
}

pub(super) fn qwen_native_edit_inputs_supported(
    cfg: &ImageSkillConfig,
    model: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
) -> bool {
    if is_qwen_multimodal_edit_model(model) {
        return true;
    }
    qwen_native_edit_source_supported(cfg, image)
        && mask
            .map(|source| qwen_native_edit_source_supported(cfg, source))
            .unwrap_or(true)
}

pub(super) fn qwen_native_edit_source_supported(
    cfg: &ImageSkillConfig,
    source: &ImageSource,
) -> bool {
    matches!(source, ImageSource::Url(_))
        || (cfg.local_auto_upload_enabled
            && matches!(source, ImageSource::Path(_) | ImageSource::Base64(_)))
}

pub(super) fn should_use_qwen_native_edit(
    cfg: &ImageSkillConfig,
    model: &str,
    mode: AdapterMode,
    allow_compat: bool,
    can_use_native_inputs: bool,
) -> bool {
    match mode {
        AdapterMode::Native => can_use_native_inputs,
        AdapterMode::Compat => false,
        AdapterMode::Auto => {
            if qwen_uses_native_edit_api(cfg, model) && can_use_native_inputs {
                true
            } else {
                !allow_compat
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn qwen_native_edit(
    client: &Client,
    image_cfg: &ImageSkillConfig,
    native_base_url: Option<&str>,
    native_function: Option<&str>,
    api_key: &str,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    n: u64,
    timeout_seconds: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    if is_qwen_multimodal_edit_model(model) {
        return qwen_wan26_edit(
            client,
            api_key,
            model,
            instruction,
            image,
            mask,
            size,
            n,
            timeout_seconds,
            max_input_bytes,
            output_path,
        );
    }
    let base = native_base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("https://dashscope.aliyuncs.com/api/v1");
    let url = format!(
        "{}/services/aigc/image2image/image-synthesis",
        trim_trailing_slash(base)
    );
    let base_image_url = resolve_qwen_native_image_url(
        client,
        image_cfg,
        image,
        max_input_bytes,
        "image.png",
        "image",
    )?;
    let normalized_size = size.trim().replace('x', "*").replace('X', "*");
    let function = native_function
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("description_edit");
    let mut input = json!({
        "prompt": instruction,
        "function": function,
        "base_image_url": base_image_url
    });
    if let Some(mask_source) = mask {
        let mask_url = resolve_qwen_native_image_url(
            client,
            image_cfg,
            mask_source,
            max_input_bytes,
            "mask.png",
            "mask",
        )?;
        input["mask_image_url"] = Value::String(mask_url);
    }
    let body = json!({
        "model": model,
        "input": input,
        "parameters": {
            "size": normalized_size,
            "n": n,
            "watermark": false
        }
    });

    let create_resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native edit request failed: {err}"))?;
    let create_status = create_resp.status().as_u16();
    let create_v: Value = create_resp
        .json()
        .map_err(|err| format!("parse qwen native edit create response failed: {err}"))?;
    if create_status >= 300 {
        return Err(format!(
            "qwen native edit create error status={create_status}: {}",
            truncate(&create_v.to_string(), 400)
        ));
    }

    let task_id = create_v
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "qwen native edit response missing task_id: {}",
                truncate(&create_v.to_string(), 400)
            )
        })?;

    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(10));
    let task_url = format!("{}/tasks/{task_id}", trim_trailing_slash(base));
    loop {
        if Instant::now() > deadline {
            return Err(format!("qwen native edit task timeout: task_id={task_id}"));
        }
        let task_resp = client
            .get(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native edit poll failed: {err}"))?;
        let task_status = task_resp.status().as_u16();
        let task_v: Value = task_resp
            .json()
            .map_err(|err| format!("parse qwen native edit task response failed: {err}"))?;
        if task_status >= 300 {
            return Err(format!(
                "qwen native edit poll error status={task_status}: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        let status = task_v
            .get("output")
            .and_then(|o| o.get("task_status"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if status == "SUCCEEDED" {
            let url = task_v
                .get("output")
                .and_then(|o| o.get("results"))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    format!(
                        "qwen native edit success response missing image url: {}",
                        truncate(&task_v.to_string(), 400)
                    )
                })?;
            let bytes = client
                .get(url)
                .send()
                .map_err(|err| format!("download edited image failed: {err}"))?
                .bytes()
                .map_err(|err| format!("read edited image bytes failed: {err}"))?;
            ensure_parent_dir(output_path)?;
            std::fs::write(output_path, &bytes)
                .map_err(|err| format!("write output failed: {err}"))?;
            return Ok(());
        }
        if status == "FAILED" || status == "CANCELED" || status == "CANCELLED" {
            return Err(format!(
                "qwen native edit task failed: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        thread::sleep(Duration::from_millis(1200));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn qwen_wan26_edit(
    client: &Client,
    api_key: &str,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    n: u64,
    timeout_seconds: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    let url = "https://dashscope.aliyuncs.com/api/v1/services/aigc/image-generation/generation";
    let mut content = vec![json!({ "text": instruction })];
    content.push(json!({
        "image": image_source_to_wan26_input(client, image, max_input_bytes, "image.png")?
    }));
    if let Some(mask_source) = mask {
        content.push(json!({
            "image": image_source_to_wan26_input(client, mask_source, max_input_bytes, "mask.png")?
        }));
    }
    let body = json!({
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": content
            }]
        },
        "parameters": {
            "size": normalize_wan26_size(size),
            "n": n,
            "prompt_extend": true,
            "watermark": false,
            "enable_interleave": false
        }
    });
    let create_resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native edit request failed: {err}"))?;
    let create_status = create_resp.status().as_u16();
    let create_v: Value = create_resp
        .json()
        .map_err(|err| format!("parse qwen native edit create response failed: {err}"))?;
    if create_status >= 300 {
        return Err(format!(
            "qwen native edit create error status={create_status}: {}",
            truncate(&create_v.to_string(), 400)
        ));
    }
    if let Some(url) = extract_qwen_output_image_url(&create_v) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download edited image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read edited image bytes failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    let task_id = create_v
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "qwen native edit response missing task_id/image: {}",
                truncate(&create_v.to_string(), 400)
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(10));
    let task_url = format!("https://dashscope.aliyuncs.com/api/v1/tasks/{task_id}");
    loop {
        if Instant::now() > deadline {
            return Err(format!("qwen native edit task timeout: task_id={task_id}"));
        }
        let task_resp = client
            .get(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native edit poll failed: {err}"))?;
        let task_status = task_resp.status().as_u16();
        let task_v: Value = task_resp
            .json()
            .map_err(|err| format!("parse qwen native edit task response failed: {err}"))?;
        if task_status >= 300 {
            return Err(format!(
                "qwen native edit poll error status={task_status}: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        let status = task_v
            .get("output")
            .and_then(|o| o.get("task_status"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if status == "SUCCEEDED" {
            let url = extract_qwen_output_image_url(&task_v).ok_or_else(|| {
                format!(
                    "qwen native edit success response missing image url: {}",
                    truncate(&task_v.to_string(), 400)
                )
            })?;
            let bytes = client
                .get(url)
                .send()
                .map_err(|err| format!("download edited image failed: {err}"))?
                .bytes()
                .map_err(|err| format!("read edited image bytes failed: {err}"))?;
            ensure_parent_dir(output_path)?;
            std::fs::write(output_path, &bytes)
                .map_err(|err| format!("write output failed: {err}"))?;
            return Ok(());
        }
        if status == "FAILED" || status == "CANCELED" || status == "CANCELLED" {
            return Err(format!(
                "qwen native edit task failed: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        thread::sleep(Duration::from_millis(1200));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn openai_compatible_edit(
    client: &Client,
    vendor_name: &str,
    cfg: &VendorConfig,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    quality: Option<&str>,
    n: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    let (image_bytes, image_mime) = load_image_bytes(client, image, max_input_bytes)?;
    let image_part = multipart::Part::bytes(image_bytes)
        .file_name("image.png")
        .mime_str(&image_mime)
        .map_err(|err| format!("set image mime failed: {err}"))?;

    let mut form = multipart::Form::new()
        .text("model", model.to_string())
        .text("prompt", instruction.to_string())
        .text("size", size.to_string())
        .text("n", n.to_string())
        .part("image", image_part);

    if let Some(q) = quality {
        form = form.text("quality", q.to_string());
    }
    if let Some(mask_source) = mask {
        let (mask_bytes, mask_mime) = load_image_bytes(client, mask_source, max_input_bytes)?;
        let mask_part = multipart::Part::bytes(mask_bytes)
            .file_name("mask.png")
            .mime_str(&mask_mime)
            .map_err(|err| format!("set mask mime failed: {err}"))?;
        form = form.part("mask", mask_part);
    }

    let url = format!("{}/images/edits", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .map_err(|err| format!("{vendor_name} request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse {vendor_name} response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "{vendor_name} error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }

    let item = v
        .get("data")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| {
            format!(
                "{vendor_name} response missing data: {}",
                truncate(&v.to_string(), 400)
            )
        })?;
    if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
        let bytes = STANDARD
            .decode(b64)
            .map_err(|err| format!("decode image base64 failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download edited image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read edited image bytes failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }

    Err(format!(
        "{vendor_name} response contains no image payload: {}",
        truncate(&v.to_string(), 400)
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn minimax_reference_edit(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    size: &str,
    quality: Option<&str>,
    n: u64,
    output_path: &Path,
) -> Result<(), String> {
    let ImageSource::Url(image_url) = image else {
        return Err(
            "minimax reference edit requires image.url; local path/base64 input is not supported by this adapter"
                .to_string(),
        );
    };
    let image_url = image_url.trim();
    if image_url.is_empty() {
        return Err("minimax reference edit requires non-empty image.url".to_string());
    }
    let mut prompt = format!(
        "Use the reference image as the source. Preserve the main subject and composition, then apply this edit: {instruction}"
    );
    if let Some(q) = quality.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str(&format!("\nQuality hint: {q}"));
    }
    let url = format!("{}/image_generation", trim_trailing_slash(&cfg.base_url));
    let body = json!({
        "model": model,
        "prompt": prompt,
        "response_format": "url",
        "n": n.max(1),
        "prompt_optimizer": true,
        "aspect_ratio": size_to_minimax_aspect_ratio(size),
        "subject_reference": [
            {
                "type": "character",
                "image_file": image_url
            }
        ]
    });
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .map_err(|err| format!("minimax request failed: {err}"))?;
    let status = resp.status().as_u16();
    let raw = resp
        .text()
        .map_err(|err| format!("read minimax response failed: {err}"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "parse minimax response failed: {err}; body={}",
            truncate(&raw, 400)
        )
    })?;
    if status >= 300 {
        return Err(format!(
            "minimax error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    if let Some(url) = minimax_response_image_url(&v) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download edited image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read edited image bytes failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    if let Some(b64) = minimax_response_image_base64(&v) {
        let bytes = STANDARD
            .decode(b64)
            .map_err(|err| format!("decode minimax image base64 failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    Err(format!(
        "minimax response contains no image payload: {}",
        truncate(&v.to_string(), 400)
    ))
}

pub(super) fn minimax_response_image_url(v: &Value) -> Option<&str> {
    v.get("data")
        .and_then(|d| d.get("image_urls"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)
        .or_else(|| {
            v.get("data")
                .and_then(|d| d.get("image_url"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            v.get("data")
                .and_then(|d| d.get("images"))
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("url"))
                .and_then(Value::as_str)
        })
}

pub(super) fn minimax_response_image_base64(v: &Value) -> Option<&str> {
    v.get("data")
        .and_then(|d| d.get("image_base64"))
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)
        .or_else(|| {
            v.get("data")
                .and_then(|d| d.get("image_base64"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            v.get("data")
                .and_then(|d| d.get("images"))
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("b64_json").or_else(|| item.get("base64")))
                .and_then(Value::as_str)
        })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn google_edit(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    quality: Option<&str>,
    _n: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    let mut parts = vec![json!({"text": format!(
        "Edit this image. Size hint: {size}. {}{}",
        instruction,
        quality.map(|q| format!(" Quality: {q}.")).unwrap_or_default()
    )})];
    let (image_bytes, image_mime) = load_image_bytes(client, image, max_input_bytes)?;
    parts.push(json!({"inline_data": {
        "mime_type": image_mime,
        "data": STANDARD.encode(image_bytes)
    }}));
    if let Some(mask_source) = mask {
        let (mask_bytes, mask_mime) = load_image_bytes(client, mask_source, max_input_bytes)?;
        parts.push(json!({"inline_data": {
            "mime_type": mask_mime,
            "data": STANDARD.encode(mask_bytes)
        }}));
        parts.push(json!({"text": "Second image is mask guidance."}));
    }
    let body = json!({
        "contents": [{"parts": parts}],
        "generationConfig": {"responseModalities": ["TEXT", "IMAGE"]}
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
            truncate(&v.to_string(), 400)
        ));
    }
    if let Some(parts) = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(b64) = part
                .get("inlineData")
                .or_else(|| part.get("inline_data"))
                .and_then(|i| i.get("data"))
                .and_then(|d| d.as_str())
            {
                let bytes = STANDARD
                    .decode(b64)
                    .map_err(|err| format!("decode google image base64 failed: {err}"))?;
                ensure_parent_dir(output_path)?;
                std::fs::write(output_path, bytes)
                    .map_err(|err| format!("write output failed: {err}"))?;
                return Ok(());
            }
        }
    }
    Err(format!(
        "google response contains no image payload: {}",
        truncate(&v.to_string(), 400)
    ))
}
