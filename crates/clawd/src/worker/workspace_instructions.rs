use serde_json::{json, Value};

use crate::AppState;

use self::discovery::{discover_workspace_instructions, enabled_for_payload, InstructionSource};

mod discovery;

const PROMPT_LOGICAL_PATH: &str = "prompts/context_workspace_instructions.md";
const PROMPT_KIND: &str = "workspace_instructions";
const PROMPT_PLACEHOLDER: &str = "__WORKSPACE_INSTRUCTION_CONTEXT__";

pub(super) struct PreparedWorkspaceInstructions {
    pub(super) rendered_context: Option<String>,
    pub(super) attribution: Value,
}

pub(super) fn prepare_workspace_instructions(
    state: &AppState,
    payload: &Value,
) -> anyhow::Result<Option<PreparedWorkspaceInstructions>> {
    let config = &state.reload_ctx.workspace_instructions;
    if !enabled_for_payload(config, payload) {
        return Ok(None);
    }
    let discovery =
        discover_workspace_instructions(&state.skill_rt.workspace_root, config, payload)?;
    let (rendered_context, wrapper_prompt) = if discovery.rendered_sources.is_empty() {
        (None, None)
    } else {
        let (rendered, attribution) =
            crate::task_context_builder::render_context_projection_prompt(
                state,
                PROMPT_LOGICAL_PATH,
                PROMPT_KIND,
                PROMPT_PLACEHOLDER,
                &discovery.rendered_sources,
            )?;
        (Some(rendered), Some(attribution))
    };
    let source_count = discovery.sources.len();
    let injected_source_count = discovery
        .sources
        .iter()
        .filter(|source| source.injected_bytes > 0)
        .count();
    let prompt_truncation_count = discovery
        .sources
        .iter()
        .filter(|source| {
            source.file_budget_truncated
                || source.total_budget_truncated
                || source.status == "omitted_file_limit"
        })
        .count();
    let prompt_skip_count = discovery
        .sources
        .iter()
        .filter(|source| {
            matches!(
                source.status,
                "invalid_utf8" | "unreadable" | "missing" | "not_file"
            )
        })
        .count();
    let source_bytes_total = discovery
        .sources
        .iter()
        .map(|source| source.source_bytes)
        .sum::<u64>();
    let loaded_bytes_total = discovery
        .sources
        .iter()
        .map(|source| source.loaded_bytes)
        .sum::<usize>();
    let injected_bytes_total = discovery
        .sources
        .iter()
        .map(|source| source.injected_bytes)
        .sum::<usize>();
    let sources = discovery
        .sources
        .iter()
        .map(source_attribution)
        .collect::<Vec<_>>();
    let prompts = wrapper_prompt.iter().cloned().collect::<Vec<_>>();
    let template_char_count = wrapper_prompt
        .as_ref()
        .and_then(|prompt| prompt.get("template_char_count"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let rendered_char_count = wrapper_prompt
        .as_ref()
        .and_then(|prompt| prompt.get("rendered_char_count"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Ok(Some(PreparedWorkspaceInstructions {
        rendered_context,
        attribution: json!({
            "schema_version": 1,
            "observation_kind": "context_prompt_attribution",
            "source_kind": "workspace_instructions",
            "data_only": true,
            "instruction_authority": "model_context_only",
            "routing_authority": false,
            "permission_authority": false,
            "response_template_authority": false,
            "working_directory_status": discovery.cwd_status,
            "relative_working_directory": discovery.relative_cwd,
            "source_count": source_count,
            "injected_source_count": injected_source_count,
            "prompt_count": prompts.len(),
            "prompt_truncation_count": prompt_truncation_count,
            "prompt_skip_count": prompt_skip_count,
            "template_char_count": template_char_count,
            "rendered_char_count": rendered_char_count,
            "source_bytes_total": source_bytes_total,
            "loaded_bytes_total": loaded_bytes_total,
            "injected_bytes_total": injected_bytes_total,
            "max_total_bytes": config.max_total_bytes,
            "max_file_bytes": config.max_file_bytes,
            "max_files": config.max_files,
            "prompts": prompts,
            "sources": sources,
        }),
    }))
}

fn source_attribution(source: &InstructionSource) -> Value {
    json!({
        "schema_version": 1,
        "prompt_kind": PROMPT_KIND,
        "source_layer": source.source_layer,
        "logical_path": source.logical_path,
        "resolved_source": "workspace_file",
        "content_sha256": source.content_sha256,
        "digest_scope": source.digest_scope,
        "source_bytes": source.source_bytes,
        "loaded_bytes": source.loaded_bytes,
        "injected_bytes": source.injected_bytes,
        "status": source.status,
        "truncated": source.file_budget_truncated || source.total_budget_truncated,
        "file_budget_truncated": source.file_budget_truncated,
        "total_budget_truncated": source.total_budget_truncated,
        "depth": source.depth,
        "precedence": source.precedence,
    })
}

#[cfg(test)]
#[path = "workspace_instructions_tests.rs"]
mod tests;
