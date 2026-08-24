use super::*;

pub(super) fn language_from_memory_preferences_map(prefs: &Map<String, Value>) -> Option<String> {
    let pairs: Vec<_> = prefs.iter().collect();
    for (k, v) in pairs.into_iter().rev() {
        let kt = k.trim();
        if kt == "response_language" || kt == "language" {
            if let Some(s) = v.as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub(super) fn language_from_json_value(v: &Value) -> Option<String> {
    v.get("language")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.to_ascii_lowercase() != "unknown")
}

pub(super) fn language_infer_schema() -> &'static Value {
    LANGUAGE_INFER_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(LANGUAGE_INFER_SCHEMA_RAW)
            .expect("language_infer schema must be valid JSON")
    })
}

pub(super) fn image_describe_schema() -> &'static Value {
    IMAGE_DESCRIBE_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_DESCRIBE_SCHEMA_RAW)
            .expect("image_describe schema must be valid JSON")
    })
}

pub(super) fn image_compare_schema() -> &'static Value {
    IMAGE_COMPARE_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_COMPARE_SCHEMA_RAW)
            .expect("image_compare schema must be valid JSON")
    })
}

pub(super) fn image_screenshot_summary_schema() -> &'static Value {
    IMAGE_SCREENSHOT_SUMMARY_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_SCREENSHOT_SUMMARY_SCHEMA_RAW)
            .expect("image_screenshot_summary schema must be valid JSON")
    })
}

pub(super) fn image_text_extraction_schema() -> &'static Value {
    IMAGE_TEXT_EXTRACTION_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_TEXT_EXTRACTION_SCHEMA_RAW)
            .expect("image_text_extraction schema must be valid JSON")
    })
}

pub(super) fn schema_requires_field(schema: &Value, name: &str) -> bool {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|fields| fields.iter().any(|field| field.as_str() == Some(name)))
        .unwrap_or(false)
}

pub(super) fn schema_declared_fields(schema: &Value) -> Option<&serde_json::Map<String, Value>> {
    schema.get("properties")?.as_object()
}

pub(super) fn schema_allows_additional_properties(schema: &Value) -> bool {
    schema
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

pub(super) fn schema_string_is_valid(schema: &Value, name: &str, value: &str) -> bool {
    let property = match schema.get("properties").and_then(|v| v.get(name)) {
        Some(property) => property,
        None => return false,
    };
    if property.get("type").and_then(|v| v.as_str()) != Some("string") {
        return false;
    }
    let min_length = property
        .get("minLength")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    value.chars().count() >= min_length
}

pub(super) fn parse_schema_validated_json_object(
    raw: &str,
    schema: &Value,
    label: &str,
) -> Result<Value, String> {
    let trimmed = raw.trim();
    let candidate = if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        value
    } else {
        let start = trimmed
            .find('{')
            .ok_or_else(|| format!("{label} is not valid JSON"))?;
        let end = trimmed
            .rfind('}')
            .ok_or_else(|| format!("{label} is not valid JSON"))?;
        if end <= start {
            return Err(format!("{label} is not valid JSON"));
        }
        serde_json::from_str::<Value>(&trimmed[start..=end])
            .map_err(|err| format!("{label} JSON parse failed: {err}"))?
    };
    validate_value_against_schema(&candidate, schema, "$")
        .map_err(|err| format!("{label} schema invalid: {err}"))?;
    Ok(candidate)
}

pub(super) fn validate_value_against_schema(
    value: &Value,
    schema: &Value,
    path: &str,
) -> Result<(), String> {
    if let Some(kind) = schema.get("type").and_then(|v| v.as_str()) {
        match kind {
            "object" => {
                let object = value
                    .as_object()
                    .ok_or_else(|| format!("{path}: expected object"))?;
                let declared_fields = schema_declared_fields(schema);
                if !schema_allows_additional_properties(schema) {
                    let declared = declared_fields
                        .ok_or_else(|| format!("{path}: schema missing properties"))?;
                    if let Some(extra) = object.keys().find(|key| !declared.contains_key(*key)) {
                        return Err(format!("{path}.{extra}: unexpected field"));
                    }
                }
                if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
                    for field in required.iter().filter_map(|v| v.as_str()) {
                        if !object.contains_key(field) {
                            return Err(format!("{path}.{field}: missing required field"));
                        }
                    }
                }
                if let Some(properties) = declared_fields {
                    for (field, property_schema) in properties {
                        if let Some(field_value) = object.get(field) {
                            validate_value_against_schema(
                                field_value,
                                property_schema,
                                &format!("{path}.{field}"),
                            )?;
                        }
                    }
                }
            }
            "array" => {
                let items = value
                    .as_array()
                    .ok_or_else(|| format!("{path}: expected array"))?;
                if let Some(item_schema) = schema.get("items") {
                    for (idx, item) in items.iter().enumerate() {
                        validate_value_against_schema(
                            item,
                            item_schema,
                            &format!("{path}[{idx}]"),
                        )?;
                    }
                }
            }
            "string" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| format!("{path}: expected string"))?;
                let min_length = schema
                    .get("minLength")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                if s.chars().count() < min_length {
                    return Err(format!("{path}: shorter than minLength {min_length}"));
                }
            }
            other => return Err(format!("{path}: unsupported schema type {other}")),
        }
    }
    Ok(())
}

pub(super) fn parse_structured_narrative_action_output(
    action: &str,
    raw: &str,
) -> Option<StructuredNarrativeActionOutput> {
    let (schema, label) = match action {
        "describe" => (image_describe_schema(), "image describe output"),
        "compare" => (image_compare_schema(), "image compare output"),
        "screenshot_summary" => (
            image_screenshot_summary_schema(),
            "image screenshot summary output",
        ),
        "extract_text" => (
            image_text_extraction_schema(),
            "image text extraction output",
        ),
        _ => return None,
    };
    let candidate = parse_schema_validated_json_object(raw, schema, label).ok()?;
    match action {
        "describe" => serde_json::from_value::<ImageDescribeOut>(candidate)
            .ok()
            .map(StructuredNarrativeActionOutput::Describe),
        "compare" => serde_json::from_value::<ImageCompareOut>(candidate)
            .ok()
            .map(StructuredNarrativeActionOutput::Compare),
        "screenshot_summary" => serde_json::from_value::<ImageScreenshotSummaryOut>(candidate)
            .ok()
            .map(StructuredNarrativeActionOutput::ScreenshotSummary),
        "extract_text" => serde_json::from_value::<ImageTextExtractionOut>(candidate)
            .ok()
            .filter(|output| !output.pages.is_empty())
            .map(|mut output| {
                for page in &mut output.pages {
                    page.text = normalize_extracted_text_newlines(&page.text);
                }
                StructuredNarrativeActionOutput::ExtractText(output)
            }),
        _ => None,
    }
}

pub(super) fn normalize_extracted_text_newlines(text: &str) -> String {
    text.replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub(super) fn render_structured_narrative_action_output(
    output: &StructuredNarrativeActionOutput,
    _response_language: Option<&str>,
    include_visible_text: bool,
) -> String {
    match output {
        StructuredNarrativeActionOutput::Describe(out) => {
            let summary = out.summary.trim();
            let visible_text = out
                .visible_text
                .iter()
                .map(|text| text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>();
            if !include_visible_text || visible_text.is_empty() {
                return summary.to_string();
            }
            format!("{summary}\n\n{}", visible_text.join("\n"))
        }
        StructuredNarrativeActionOutput::Compare(out) => out.summary.trim().to_string(),
        StructuredNarrativeActionOutput::ScreenshotSummary(out) => out.purpose.trim().to_string(),
        StructuredNarrativeActionOutput::ExtractText(out) => out
            .pages
            .iter()
            .map(|page| page.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

pub(super) fn parse_language_choice_from_json_value(v: &Value) -> Option<String> {
    let schema = language_infer_schema();
    let object = v.as_object()?;
    if !schema_allows_additional_properties(schema) {
        let declared_fields = schema_declared_fields(schema)?;
        if object.keys().any(|key| !declared_fields.contains_key(key)) {
            return None;
        }
    }
    if schema_requires_field(schema, "language") && !object.contains_key("language") {
        return None;
    }
    let language = object.get("language")?.as_str()?.trim();
    if !schema_string_is_valid(schema, "language", language) {
        return None;
    }
    language_from_json_value(v)
}

pub(super) fn parse_language_choice_from_llm(raw: &str) -> Option<String> {
    let t = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return parse_language_choice_from_json_value(&v);
    }
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    if end <= start {
        return None;
    }
    let slice = &t[start..=end];
    let v: Value = serde_json::from_str(slice).ok()?;
    parse_language_choice_from_json_value(&v)
}

pub(super) fn preferred_prompt_vendor(cfg: &RootConfig) -> &str {
    cfg.llm
        .selected_vendor
        .as_deref()
        .or(cfg.image_vision.default_vendor.as_deref())
        .unwrap_or("default")
}

pub(super) fn load_language_infer_prompt_template(
    workspace_root: &Path,
    prompt_vendor: &str,
) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor,
        "prompts/language_infer_prompt.md",
        DEFAULT_LANGUAGE_INFER_PROMPT_TEMPLATE,
    )
    .0
}

/// When explicit language args are absent, derive target language from generic runner `context`,
/// injected `args._memory`, and (last resort) an OpenAI-compatible chat pass over `_memory.context`.
pub(super) fn resolve_effective_response_language(
    cfg: &RootConfig,
    workspace_root: &Path,
    args_obj: &Map<String, Value>,
    runner_context: Option<&Value>,
    task_timeout_seconds: u64,
) -> Option<String> {
    if let Some(s) = non_empty_str_from_value(args_obj.get("response_language")) {
        return Some(s);
    }
    if let Some(s) = non_empty_str_from_value(args_obj.get("language")) {
        return Some(s);
    }
    if let Some(ctx) = runner_context.and_then(|c| c.as_object()) {
        if let Some(s) = non_empty_str_from_value(ctx.get("response_language"))
            .or_else(|| non_empty_str_from_value(ctx.get("language")))
        {
            return Some(s);
        }
    }
    let Some(mem_obj) = args_obj.get("_memory").and_then(|m| m.as_object()) else {
        return None;
    };
    if let Some(s) = non_empty_str_from_value(mem_obj.get("lang_hint")) {
        return Some(s);
    }
    if let Some(prefs) = mem_obj.get("preferences").and_then(|p| p.as_object()) {
        if let Some(s) = language_from_memory_preferences_map(prefs) {
            return Some(s);
        }
    }
    let snippets = mem_obj
        .get("context")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    if snippets.is_empty() || snippets == "<none>" {
        return None;
    }
    let infer_timeout = task_timeout_seconds.clamp(10, 90).min(30);
    infer_language_from_memory_snippets_llm(cfg, workspace_root, snippets, infer_timeout)
}

pub(super) fn infer_language_from_memory_snippets_llm(
    cfg: &RootConfig,
    workspace_root: &Path,
    memory_snippets: &str,
    infer_timeout_secs: u64,
) -> Option<String> {
    let template =
        load_language_infer_prompt_template(workspace_root, preferred_prompt_vendor(cfg));
    let prompt = template.replace("__MEMORY_SNIPPETS__", memory_snippets);
    let t = infer_timeout_secs.clamp(5, 45).min(25);
    for vk in vendor_order(None, cfg.image_vision.default_vendor.as_deref()) {
        let Ok((vname, vcfg)) = resolve_vendor_config(cfg, vk) else {
            continue;
        };
        if check_api_key(vname, &vcfg.api_key).is_err() {
            continue;
        }
        let model = vcfg.model.trim();
        if model.is_empty() {
            continue;
        }
        if let Ok(out) =
            openai_compat_chat_rewrite(&vcfg, model, &prompt, t, vk == VendorKind::Mimo)
        {
            if let Some(lang) = parse_language_choice_from_llm(&out) {
                return Some(lang);
            }
        }
    }
    None
}

pub(super) fn build_prompt(
    workspace_root: &Path,
    prompt_vendor: &str,
    action: &str,
    detail_level: &str,
    schema: Option<&Value>,
    response_language: Option<&str>,
    user_instruction: Option<&str>,
) -> String {
    let template = load_image_vision_prompt_template(workspace_root, prompt_vendor);
    let mut task_instruction =
        action_instruction(workspace_root, prompt_vendor, action, detail_level, schema);
    if let Some(extra) = user_instruction {
        task_instruction.push_str("\n\nAdditional user instruction:\n");
        task_instruction.push_str(extra);
    }
    let schema_hint = schema
        .map(|s| s.to_string())
        .unwrap_or_else(|| "none".to_string());
    let language_hint = response_language
        .map(|s| {
            load_prompt_fragment(
                workspace_root,
                prompt_vendor,
                "prompts/image_vision_language_hint_with_target.md",
                DEFAULT_IMAGE_VISION_LANGUAGE_HINT_WITH_TARGET_TEMPLATE,
            )
            .replace("__RESPONSE_LANGUAGE__", s)
        })
        .unwrap_or_else(|| {
            load_prompt_fragment(
                workspace_root,
                prompt_vendor,
                "prompts/image_vision_language_hint_default.md",
                DEFAULT_IMAGE_VISION_LANGUAGE_HINT_DEFAULT_TEMPLATE,
            )
        });
    template
        .replace("__ACTION__", action)
        .replace("__DETAIL_LEVEL__", detail_level)
        .replace("__TASK_INSTRUCTION__", &task_instruction)
        .replace("__SCHEMA_HINT__", &schema_hint)
        .replace("__LANGUAGE_HINT__", &language_hint)
}

pub(super) fn structured_output_retry_prompt(original_prompt: &str) -> String {
    format!(
        "{original_prompt}\n\nThe previous response did not satisfy the required JSON schema. Reinspect the source image and return exactly one valid JSON object with every required field. The `visible_text` array order already records reading order: do not add ordinal numbers, bullets, Markdown prefixes, checkboxes, or other line-start markers to unmarked source lines. Preserve a line-start marker only when it is visibly present in the source image."
    )
}

pub(super) fn load_image_output_rewrite_prompt_template(
    workspace_root: &Path,
    prompt_vendor: &str,
) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor,
        "prompts/image_output_rewrite_prompt.md",
        include_str!("../../../../prompts/layers/overlays/image_output_rewrite_prompt.md"),
    )
    .0
}

pub(super) fn load_image_text_revision_prompt_template(
    workspace_root: &Path,
    prompt_vendor: &str,
) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor,
        "prompts/image_text_revision_prompt.md",
        include_str!("../../../../prompts/layers/overlays/image_text_revision_prompt.md"),
    )
    .0
}

pub(super) fn openai_compat_chat_rewrite(
    vcfg: &VendorConfig,
    model: &str,
    user_prompt: &str,
    timeout_secs: u64,
    include_api_key_header: bool,
) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs.clamp(5, 120)))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let url = format!("{}/chat/completions", trim_trailing_slash(&vcfg.base_url));
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": user_prompt}],
        "temperature": 0.0
    });
    let mut request = client.post(&url).bearer_auth(vcfg.api_key.trim());
    if include_api_key_header {
        request = request.header("api-key", vcfg.api_key.trim());
    }
    let resp = request
        .json(&body)
        .send()
        .map_err(|e| format!("chat completion request failed: {e}"))?;
    if !resp.status().is_success() {
        let t = resp.text().unwrap_or_default();
        return Err(format!("chat completion HTTP error: {t}"));
    }
    let v: Value = resp
        .json()
        .map_err(|e| format!("parse chat response: {e}"))?;
    Ok(v.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

/// Same-turn alignment pass formerly done in `clawd`: rewrite vision text to `response_language`
/// for narrative actions. On failure, returns `vision_output` unchanged.
pub(super) fn maybe_rewrite_image_vision_text_for_target_language(
    cfg: &RootConfig,
    workspace_root: &Path,
    action: &str,
    target_language: Option<&str>,
    vision_output: String,
    task_timeout_seconds: u64,
) -> String {
    let Some(lang) = target_language.map(str::trim).filter(|s| !s.is_empty()) else {
        return vision_output;
    };
    if !matches!(action, "describe" | "compare" | "screenshot_summary") {
        return vision_output;
    }
    if vision_output.trim().is_empty() {
        return vision_output;
    }
    let template =
        load_image_output_rewrite_prompt_template(workspace_root, preferred_prompt_vendor(cfg));
    let prompt = template
        .replace("__TARGET_LANGUAGE__", lang)
        .replace("__ORIGINAL_OUTPUT__", &vision_output);
    let rewrite_timeout = task_timeout_seconds.clamp(10, 90).min(45);
    for vk in vendor_order(None, cfg.image_vision.default_vendor.as_deref()) {
        let Ok((vname, vcfg)) = resolve_vendor_config(cfg, vk) else {
            continue;
        };
        if check_api_key(vname, &vcfg.api_key).is_err() {
            continue;
        }
        let model = vcfg.model.trim();
        if model.is_empty() {
            continue;
        }
        if let Ok(out) = openai_compat_chat_rewrite(
            &vcfg,
            model,
            &prompt,
            rewrite_timeout,
            vk == VendorKind::Mimo,
        ) {
            let t = out.trim();
            if !t.is_empty() {
                return strip_think_blocks(t).trim().to_string();
            }
        }
    }
    vision_output
}

pub(super) fn review_recognized_image_text(
    cfg: &RootConfig,
    workspace_root: &Path,
    recognized_text: String,
    task_timeout_seconds: u64,
) -> (String, Value) {
    let raw_text = recognized_text.trim();
    if raw_text.is_empty() {
        return (recognized_text, json!({"status": "skipped_empty"}));
    }
    let raw_character_count = raw_text.chars().count();
    let chunks = split_image_text_revision_chunks(raw_text, 6_000);
    let template =
        load_image_text_revision_prompt_template(workspace_root, preferred_prompt_vendor(cfg));
    let rewrite_timeout = task_timeout_seconds.clamp(10, 120).min(60);
    let mut reviewed_chunks = Vec::with_capacity(chunks.len());
    let mut selected_provider = None;
    let mut selected_model = None;
    let mut errors = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let prompt = template
            .replace("__CHUNK_INDEX__", &(index + 1).to_string())
            .replace("__CHUNK_COUNT__", &chunks.len().to_string())
            .replace("__RAW_RECOGNIZED_TEXT__", chunk);
        let mut reviewed_chunk = None;
        for vk in vendor_order(None, cfg.image_vision.default_vendor.as_deref()) {
            let Ok((vname, vcfg)) = resolve_vendor_config(cfg, vk) else {
                continue;
            };
            if check_api_key(vname, &vcfg.api_key).is_err() {
                continue;
            }
            let model = vcfg.model.trim();
            if model.is_empty() {
                continue;
            }
            match openai_compat_chat_rewrite(
                &vcfg,
                model,
                &prompt,
                rewrite_timeout,
                vk == VendorKind::Mimo,
            ) {
                Ok(output) => {
                    let output = strip_think_blocks(&output).trim().to_string();
                    if !output.is_empty() {
                        selected_provider.get_or_insert_with(|| vname.to_string());
                        selected_model.get_or_insert_with(|| model.to_string());
                        reviewed_chunk = Some(output);
                        break;
                    }
                    errors.push(format!("{vname}: empty response"));
                }
                Err(error) => errors.push(format!("{vname}: {}", truncate(&error, 240))),
            }
        }
        let Some(reviewed_chunk) = reviewed_chunk else {
            return (
                recognized_text,
                json!({
                    "status": "fallback_raw",
                    "reviewed_by_model": false,
                    "chunk_count": chunks.len(),
                    "diagnostics": errors.into_iter().take(8).collect::<Vec<_>>(),
                }),
            );
        };
        reviewed_chunks.push(reviewed_chunk);
    }
    let reviewed_text = join_image_text_revision_chunks(&chunks, &reviewed_chunks);
    if !image_text_revision_preserves_source(raw_text, &reviewed_text) {
        return (
            recognized_text,
            json!({
                "status": "fallback_raw",
                "reviewed_by_model": false,
                "error_code": "revision_integrity_failed",
                "chunk_count": chunks.len(),
                "raw_character_count": raw_character_count,
                "reviewed_character_count": reviewed_text.chars().count(),
            }),
        );
    }
    (
        reviewed_text.clone(),
        json!({
            "status": "reviewed",
            "reviewed_by_model": true,
            "provider": selected_provider,
            "model": selected_model,
            "chunk_count": chunks.len(),
            "raw_character_count": raw_character_count,
            "reviewed_character_count": reviewed_text.chars().count(),
            "source_language_policy": "preserve_source_language",
            "layout_policy": "semantic_reflow",
        }),
    )
}

pub(super) fn image_text_revision_preserves_source(raw_text: &str, reviewed_text: &str) -> bool {
    if reviewed_text.trim().is_empty()
        || image_text_numeric_tokens(raw_text) != image_text_numeric_tokens(reviewed_text)
    {
        return false;
    }
    let raw_count = raw_text
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    let reviewed_count = reviewed_text
        .chars()
        .filter(|character| !character.is_whitespace())
        .count();
    if raw_count < 40 {
        return reviewed_count <= raw_count.saturating_mul(2).saturating_add(16);
    }
    reviewed_count.saturating_mul(10) >= raw_count.saturating_mul(6)
        && reviewed_count.saturating_mul(2) <= raw_count.saturating_mul(3)
}

fn image_text_numeric_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_numeric() {
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub(super) fn join_image_text_revision_chunks(
    source_chunks: &[String],
    reviewed_chunks: &[String],
) -> String {
    let mut assembled = String::new();
    for (index, reviewed) in reviewed_chunks.iter().enumerate() {
        assembled.push_str(reviewed.trim());
        if index < source_chunks.len().saturating_sub(1) {
            let boundary = source_chunks[index]
                .chars()
                .rev()
                .take_while(|character| character.is_whitespace())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            assembled.push_str(&boundary);
        }
    }
    assembled.trim().to_string()
}

pub(super) fn split_image_text_revision_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let characters = text.trim().chars().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < characters.len() {
        let mut end = (start + max_chars).min(characters.len());
        if end < characters.len() {
            let floor = start + max_chars / 2;
            if let Some(boundary) = (floor..end)
                .rev()
                .find(|index| characters[*index].is_whitespace())
            {
                end = boundary + 1;
            }
        }
        let chunk = characters[start..end].iter().collect::<String>();
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        start = end;
    }
    chunks
}

pub(super) fn strip_think_blocks(text: &str) -> String {
    static THINK_RE: OnceLock<Regex> = OnceLock::new();
    THINK_RE
        .get_or_init(|| Regex::new(r"(?is)<think>.*?</think>").expect("think regex compiles"))
        .replace_all(text, "")
        .to_string()
}

pub(super) fn action_instruction(
    workspace_root: &Path,
    prompt_vendor: &str,
    action: &str,
    detail_level: &str,
    schema: Option<&Value>,
) -> String {
    match action {
        "describe" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_describe.md",
            DEFAULT_IMAGE_VISION_ACTION_DESCRIBE_TEMPLATE,
        )
        .replace("__DETAIL_LEVEL__", detail_level),
        "compare" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_compare.md",
            DEFAULT_IMAGE_VISION_ACTION_COMPARE_TEMPLATE,
        ),
        "screenshot_summary" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_screenshot_summary.md",
            DEFAULT_IMAGE_VISION_ACTION_SCREENSHOT_SUMMARY_TEMPLATE,
        ),
        "extract" => {
            if let Some(s) = schema {
                load_prompt_fragment(
                    workspace_root,
                    prompt_vendor,
                    "prompts/image_vision_action_extract_with_schema.md",
                    DEFAULT_IMAGE_VISION_ACTION_EXTRACT_WITH_SCHEMA_TEMPLATE,
                )
                .replace("__SCHEMA__", &s.to_string())
            } else {
                load_prompt_fragment(
                    workspace_root,
                    prompt_vendor,
                    "prompts/image_vision_action_extract_default.md",
                    DEFAULT_IMAGE_VISION_ACTION_EXTRACT_DEFAULT_TEMPLATE,
                )
            }
        }
        "extract_text" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_extract_text.md",
            DEFAULT_IMAGE_VISION_ACTION_EXTRACT_TEXT_TEMPLATE,
        ),
        _ => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_fallback.md",
            DEFAULT_IMAGE_VISION_ACTION_FALLBACK_TEMPLATE,
        ),
    }
}

pub(super) fn prompt_vendor_name_for_vendor(vendor: VendorKind) -> &'static str {
    match vendor {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "claude",
        VendorKind::Grok => "grok",
        VendorKind::DeepSeek => "deepseek",
        VendorKind::Qwen => "qwen",
        VendorKind::MiniMax => "minimax",
        VendorKind::Mimo => "mimo",
    }
}

pub(super) fn load_prompt_fragment(
    workspace_root: &Path,
    vendor: &str,
    relative_path: &str,
    default_template: &str,
) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        vendor,
        relative_path,
        default_template,
    )
    .0
}

pub(super) fn load_image_vision_prompt_template(workspace_root: &Path, vendor: &str) -> String {
    load_prompt_fragment(
        workspace_root,
        vendor,
        "prompts/image_vision_prompt.md",
        DEFAULT_IMAGE_VISION_PROMPT_TEMPLATE,
    )
}

const DEFAULT_IMAGE_VISION_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_prompt.md");
const DEFAULT_IMAGE_VISION_LANGUAGE_HINT_WITH_TARGET_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_language_hint_with_target.md");
const DEFAULT_IMAGE_VISION_LANGUAGE_HINT_DEFAULT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_language_hint_default.md");
const DEFAULT_IMAGE_VISION_ACTION_DESCRIBE_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_describe.md");
const DEFAULT_IMAGE_VISION_ACTION_COMPARE_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_compare.md");
const DEFAULT_IMAGE_VISION_ACTION_SCREENSHOT_SUMMARY_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_screenshot_summary.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_WITH_SCHEMA_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_extract_with_schema.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_DEFAULT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_extract_default.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_TEXT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_extract_text.md");
const DEFAULT_IMAGE_VISION_ACTION_FALLBACK_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_fallback.md");
const DEFAULT_LANGUAGE_INFER_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/language_infer_prompt.md");
const LANGUAGE_INFER_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/language_infer.schema.json");
const IMAGE_DESCRIBE_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_describe.schema.json");
const IMAGE_COMPARE_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_compare.schema.json");
const IMAGE_SCREENSHOT_SUMMARY_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_screenshot_summary.schema.json");
const IMAGE_TEXT_EXTRACTION_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_text_extraction.schema.json");

static LANGUAGE_INFER_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_DESCRIBE_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_COMPARE_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_SCREENSHOT_SUMMARY_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_TEXT_EXTRACTION_SCHEMA: OnceLock<Value> = OnceLock::new();

impl StructuredNarrativeActionOutput {
    pub(super) fn to_json_value(&self) -> Value {
        match self {
            StructuredNarrativeActionOutput::Describe(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
            StructuredNarrativeActionOutput::Compare(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
            StructuredNarrativeActionOutput::ScreenshotSummary(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
            StructuredNarrativeActionOutput::ExtractText(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
        }
    }
}
