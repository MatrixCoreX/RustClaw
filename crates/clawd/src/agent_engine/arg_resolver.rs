use serde_json::Value;
use std::path::Path;

use super::LoopState;
use crate::IntentOutputContract;

pub(super) fn rewrite_run_cmd_with_written_aliases(
    command: &str,
    loop_state: &LoopState,
) -> String {
    if loop_state.written_file_aliases.is_empty() {
        return command.to_string();
    }
    let mut rewritten = command.to_string();
    for token in command.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| matches!(c, '"' | '\''));
        if trimmed.is_empty() {
            continue;
        }
        for (alias, effective) in &loop_state.written_file_aliases {
            if trimmed == alias || trimmed == alias.trim_start_matches("./") {
                rewritten = rewritten.replace(token, &token.replace(trimmed, effective));
                break;
            }
        }
    }
    rewritten
}

pub(super) fn rewrite_tool_path_with_written_aliases(
    tool: &str,
    args: &mut Value,
    loop_state: &LoopState,
) {
    if !matches!(tool, "read_file" | "remove_file") || loop_state.written_file_aliases.is_empty() {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    let Some(path) = obj.get("path").and_then(|v| v.as_str()) else {
        return;
    };
    let normalized = path.trim().trim_start_matches("./");
    let Some(effective) = loop_state.written_file_aliases.get(normalized) else {
        return;
    };
    obj.insert("path".to_string(), Value::String(effective.clone()));
}

/// §7.1 把 normalizer 给出的 OutputContract 关键字段格式化成可读多行块，
/// 注入到 chat 的 `recent_execution_context` 顶部，让下游 chat skill prompt
/// 能拿到 "answer-shape spec"（response_shape / semantic_kind / locator_hint /
/// must_include_tokens / no_paraphrase 等约束）作为硬锚点，避免再因为
/// "normalizer 已经标了 existence_with_path、chat 却答成段落描述" 而失败。
///
/// 返回 None 表示当前 contract 没有任何可对下游有约束力的字段（比如默认值
/// 全是 None / Free / 无 hint）—— 此时不注入，避免在 ad-hoc 调用上加噪声。
pub(super) fn render_output_contract_for_chat(
    contract: &IntentOutputContract,
) -> Option<String> {
    use crate::{OutputResponseShape, OutputSemanticKind};
    let mut lines = Vec::with_capacity(8);
    let shape_name = contract.response_shape.as_str();
    let semantic_name = contract.semantic_kind.as_str();
    let locator_kind_name = contract.locator_kind.as_str();
    let delivery_intent_name = contract.delivery_intent.as_str();

    let has_meaningful_shape = !matches!(contract.response_shape, OutputResponseShape::Free);
    let has_meaningful_semantic = !matches!(contract.semantic_kind, OutputSemanticKind::None);
    let has_locator_hint = !contract.locator_hint.trim().is_empty();
    if !has_meaningful_shape
        && !has_meaningful_semantic
        && !has_locator_hint
        && !contract.delivery_required
        && !contract.requires_content_evidence
    {
        return None;
    }

    lines.push(format!("response_shape: {shape_name}"));
    lines.push(format!("semantic_kind: {semantic_name}"));
    if locator_kind_name != "none" {
        lines.push(format!("locator_kind: {locator_kind_name}"));
    }
    if delivery_intent_name != "none" {
        lines.push(format!("delivery_intent: {delivery_intent_name}"));
    }
    if has_locator_hint {
        lines.push(format!("locator_hint: {}", contract.locator_hint.trim()));
    }
    if contract.delivery_required {
        lines.push("delivery_required: true".to_string());
    }
    if contract.requires_content_evidence {
        lines.push("requires_content_evidence: true".to_string());
    }

    // 按 semantic_kind 给 must_include 约束 —— 这是 §7.1 verifier 的对偶物，
    // 在 chat-prompt 端先用文字告知"必须出现什么 token"，verifier 端再硬拦截。
    match contract.semantic_kind {
        OutputSemanticKind::ExistenceWithPath => {
            lines.push(
                "must_include_tokens: yes/no token (有/没有/不存在/yes/no/exists/missing) AND a real filesystem path substring".to_string(),
            );
            lines.push("no_paraphrase: do NOT replace the question with \"this is a systemd file\" / \"看起来像\" 类描述句".to_string());
        }
        OutputSemanticKind::ScalarPathOnly => {
            lines.push("must_include_tokens: a single filesystem path literal, no surrounding prose".to_string());
        }
        OutputSemanticKind::ScalarCount => {
            lines.push("must_include_tokens: an integer literal (count); the integer MUST come from the observed evidence, not paraphrased".to_string());
        }
        OutputSemanticKind::QuantityComparison => {
            lines.push("must_include_tokens: \"<winner> 更多/更大/更高 (or 相同)\" + both sides' numeric values".to_string());
        }
        OutputSemanticKind::HiddenEntriesCheck => {
            lines.push("must_include_tokens: yes/no token (有/没有) about whether hidden entries exist".to_string());
        }
        OutputSemanticKind::ServiceStatus => {
            lines.push("must_include_tokens: a status word (running / active / inactive / stopped / failed) reflecting the observed evidence".to_string());
        }
        OutputSemanticKind::RecentScalarEqualityCheck => {
            lines.push("must_include_tokens: yes/no equality verdict (是/否/相同/不同/yes/no) referencing the two observed values".to_string());
        }
        _ => {}
    }
    Some(lines.join("\n"))
}

pub(super) fn attach_recent_execution_context_to_chat_args(
    args: &mut Value,
    loop_state: &LoopState,
    original_user_request: &str,
    output_contract: Option<&IntentOutputContract>,
) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    if obj.contains_key("recent_execution_context") {
        return;
    }
    let mut context_lines = Vec::new();
    // §7.1 output_contract 注入：放在最顶端，比 original_user_request 还高优先级，
    // 因为它告诉 chat skill "回答必须长成什么样"——最强的硬锚点。
    // 仅当 contract 非默认值时才注入（render_output_contract_for_chat 内部判定），
    // 避免给无 contract 的 ad-hoc 调用引入噪声。
    if let Some(contract) = output_contract {
        if let Some(rendered) = render_output_contract_for_chat(contract) {
            context_lines.push(format!(
                "output_contract (authoritative answer-shape spec from normalizer; treat as a hard contract — your final reply MUST satisfy all listed must_include_tokens / no_paraphrase / response_shape rules; if the observed evidence cannot support these tokens, say so explicitly instead of paraphrasing):\n{rendered}"
            ));
        }
    }
    // §nl-fix-2026-04-19 act_find_service_file 失败链路的根因修复：planner 给
    // chat-transform 步生成的 args.text 通常是 "用一句简短的中文回答用户问题，依据
    // 是这条观察输出：{{last_output}}" 这种通用模板，**字面里完全不带原始用户请
    // 求**。chat skill 收到的 User Message 因此只剩抽象的"用户问题"+ last_output，
    // 容易脱离原问题语义自由发挥（例：用户问"有没有 + 路径"，chat 却回"这是 systemd
    // 单元文件"）。这里把本轮真实的 user 请求字面置顶注入到 chat 的执行上下文，
    // 让 chat 系统 prompt 里的 "treat original_user_request as the verbatim user
    // question" 规则有锚点可抓。trim 后空才跳过，避免在 ad-hoc 调用（无 user 请求
    // 上下文）时塞入空字符串造成歧义。
    let user_req = original_user_request.trim();
    if !user_req.is_empty() {
        context_lines.push(format!(
            "original_user_request (verbatim user request for this turn; treat as the authoritative user question that args.text refers to when args.text uses generic phrasing like \"用户问题\"/\"原问题\"/\"user question\"):\n{user_req}"
        ));
    }
    if let Some(path) = loop_state.last_written_file_path.as_deref() {
        context_lines.push(format!("last_written_file_path: {path}"));
    }
    if let Some(path) = loop_state.output_vars.get("last_file_path") {
        context_lines.push(format!("last_file_path: {path}"));
    }
    if let Some(path) = loop_state.output_vars.get("last_read_file_path") {
        context_lines.push(format!("last_read_file_path: {path}"));
    }
    if let Some(output) = loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        context_lines.push(format!("last_output: {}", crate::truncate_for_log(output)));
    }
    // Multi-step intra-turn bridge: when the LLM plan contains multiple observation steps
    // (e.g. `[read_file(乙), read_file(甲)]`), `last_output` only carries the last step's
    // output (甲), and earlier steps' real outputs (乙) would be silently dropped before the
    // chat-skill sees them. Surface every prior OK observation step so multi-evidence
    // questions ("读一下乙的开头，然后顺手说甲是干什么的", "对比文件 A 和 B") can ground
    // on full evidence. Skip the last step (already in `last_output`) and skip non-evidence
    // steps (chat / respond) to avoid feeding the chat-skill its own prior reply.
    {
        use crate::executor::StepExecutionStatus;
        let evidence_steps: Vec<(usize, &crate::executor::StepExecutionResult)> = loop_state
            .executed_step_results
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                matches!(s.status, StepExecutionStatus::Ok)
                    && !s.skill.eq_ignore_ascii_case("chat")
                    && !s.skill.eq_ignore_ascii_case("respond")
                    && s.output
                        .as_deref()
                        .map(|o| !o.trim().is_empty())
                        .unwrap_or(false)
            })
            .collect();
        if evidence_steps.len() > 1 {
            // All but the final evidence step (final one is already exposed via last_output).
            let prior = &evidence_steps[..evidence_steps.len() - 1];
            let mut prior_lines = Vec::with_capacity(prior.len());
            for (idx, step) in prior {
                let out = step.output.as_deref().unwrap_or("");
                prior_lines.push(format!(
                    "step[{}] skill={}: {}",
                    idx + 1,
                    step.skill,
                    crate::truncate_for_log(out.trim())
                ));
            }
            context_lines.push(format!(
                "prior_step_outputs (earlier observation steps in current turn; treat as authoritative observation evidence the same way as last_output):\n{}",
                prior_lines.join("\n")
            ));
        }
    }
    // Cross-turn bridge: when current turn references prior turns ("上一个文件 / 上上个 /
    // 那个文件 / 甲 / 乙" / "对比" / "比较" / "用 X 解释 Y"), the chat-skill LLM only sees
    // intra-turn last_output above. Append the task-level recent_execution_context (rendered
    // from past turns' tasks) so chat skill can ground its answer in earlier turns' outputs.
    if let Some(cross_turn) = loop_state
        .output_vars
        .get("cross_turn_recent_execution_context")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        context_lines.push(format!(
            "cross_turn_recent_execution_context (prior turns in this conversation; treat as authoritative observation evidence the same way as last_output):\n{cross_turn}"
        ));
    }
    if context_lines.is_empty() {
        return;
    }
    obj.insert(
        "recent_execution_context".to_string(),
        Value::String(context_lines.join("\n")),
    );
}

fn rewrite_path_field(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    match obj.get("path").and_then(|v| v.as_str()) {
        Some(current) if current == auto_locator_path => false,
        Some(current) => {
            // AUTO_LOCATOR 只是 turn 级"默认 path 兜底"，仅当 LLM 给的 path 看起来是
            // 猜测/无效（不是已存在的文件/目录）时才能覆盖。已显式且存在的具体路径
            // （例如 plan 中的 read_file(README.md) 与 read_file(service_notes.md) 这种
            // 多目标 read 链路）必须保留 LLM 原值，否则会把多步 read 全部 rewrite 成同一个
            // auto_locator path，导致下游 chat 拿到重复内容、看不到第二个目标的真实输出。
            let trimmed = current.trim();
            if !trimmed.is_empty() && Path::new(trimmed).exists() {
                return false;
            }
            obj.insert(
                "path".to_string(),
                Value::String(auto_locator_path.to_string()),
            );
            true
        }
        None => false,
    }
}

fn rewrite_root_field(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    match obj.get("root").and_then(|v| v.as_str()) {
        Some(current) if current == auto_locator_path => false,
        Some(current) => {
            // 与 rewrite_path_field 同义：已存在的真实 root（如 LLM 显式给的 fs_search.find_path
            // root="/home/.../docs"）不该被 turn 级 AUTO_LOCATOR 默认值覆盖。
            let trimmed = current.trim();
            if !trimmed.is_empty() && Path::new(trimmed).exists() {
                return false;
            }
            obj.insert(
                "root".to_string(),
                Value::String(auto_locator_path.to_string()),
            );
            true
        }
        None => false,
    }
}

pub(super) fn rewrite_args_with_auto_locator_path(
    normalized_skill: &str,
    args: &mut Value,
    loop_state: &LoopState,
) -> bool {
    let Some(auto_locator_path) = loop_state
        .output_vars
        .get("auto_locator_path")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return false;
    };
    let auto_path = Path::new(auto_locator_path);
    match normalized_skill {
        "read_file" if auto_path.is_file() => rewrite_path_field(args, auto_locator_path),
        "list_dir" if auto_path.is_dir() => rewrite_path_field(args, auto_locator_path),
        "system_basic" => {
            let action = args
                .as_object()
                .and_then(|obj| obj.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match action {
                "extract_field" | "extract_fields" | "structured_keys" | "read_range"
                    if auto_path.is_file() =>
                {
                    rewrite_path_field(args, auto_locator_path)
                }
                "inventory_dir" | "count_inventory" | "workspace_glance" | "tree_summary"
                    if auto_path.is_dir() =>
                {
                    rewrite_path_field(args, auto_locator_path)
                }
                "find_path" if auto_path.is_dir() => rewrite_root_field(args, auto_locator_path),
                _ => false,
            }
        }
        _ => false,
    }
}

fn replace_double_brace_placeholders(
    input: &str,
    vars: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = input.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn single_brace_key(input: &str) -> Option<&str> {
    if !(input.starts_with('{') && input.ends_with('}')) {
        return None;
    }
    let inner = &input[1..input.len().saturating_sub(1)];
    if inner.is_empty() || inner.contains('{') || inner.contains('}') {
        return None;
    }
    Some(inner)
}

fn angle_bracket_key(input: &str) -> Option<&str> {
    if !(input.starts_with('<') && input.ends_with('>')) {
        return None;
    }
    let inner = &input[1..input.len().saturating_sub(1)];
    if inner.is_empty() || inner.contains('<') || inner.contains('>') {
        return None;
    }
    Some(inner)
}

pub(super) fn resolve_arg_string(input: &str, loop_state: &LoopState) -> String {
    let replaced = replace_double_brace_placeholders(input, &loop_state.output_vars);
    if let Some(key) = single_brace_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
    }
    if let Some(key) = angle_bracket_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
        let normalized_key = key.trim().to_ascii_lowercase();
        if let Some(v) = loop_state.output_vars.get(&normalized_key) {
            return v.clone();
        }
    }
    replaced
}

pub(super) fn resolve_arg_value(value: &Value, loop_state: &LoopState) -> Value {
    match value {
        Value::String(s) => Value::String(resolve_arg_string(s, loop_state)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_arg_value(v, loop_state))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_arg_value(v, loop_state));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        attach_recent_execution_context_to_chat_args, render_output_contract_for_chat,
        rewrite_args_with_auto_locator_path,
    };
    use crate::agent_engine::LoopState;
    use crate::{IntentOutputContract, OutputResponseShape, OutputSemanticKind};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_arg_resolver_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn auto_locator_rewrites_system_basic_file_path() {
        let root = TempDirGuard::new("readme_file");
        let readme = root.path.join("README.md");
        fs::write(&readme, "# title\n").expect("write readme");
        let readme_path = readme.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), readme_path.clone());
        let mut args = json!({
            "action": "read_range",
            "path": "/tmp/README",
            "mode": "head",
            "n": 20
        });
        assert!(rewrite_args_with_auto_locator_path(
            "system_basic",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(readme_path.as_str())
        );
    }

    #[test]
    fn auto_locator_rewrites_directory_root_for_find_path() {
        let root = TempDirGuard::new("workspace_dir");
        let document = root.path.join("document");
        fs::create_dir_all(&document).expect("create document");
        let document_path = document.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), document_path.clone());
        // 用一个明确不存在的 root，以贴近 AUTO_LOCATOR 的"兜底猜测路径"语义。
        let mut args = json!({
            "action": "find_path",
            "root": "/nonexistent_root_for_auto_locator_test_xyz",
            "name": "manual_note.txt"
        });
        assert!(rewrite_args_with_auto_locator_path(
            "system_basic",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some(document_path.as_str())
        );
    }

    #[test]
    fn attach_chat_context_injects_original_user_request_at_top() {
        // §nl-fix-2026-04-19 act_find_service_file 回归用例：planner 给 chat-transform
        // 步生成的 args.text 是固定模板"用一句简短的中文回答用户问题：{{last_output}}"，
        // 不含原话；本注入必须把真正的 user 请求字面置顶塞进 chat 上下文，否则下游 chat
        // skill 会脱离原问题语义自由发挥（如把"有没有 + 路径"答成"这是什么文件"）。
        let mut loop_state = LoopState::new(2);
        loop_state.last_output = Some("/home/guagua/rustclaw/rustclaw.service".to_string());
        let mut args = json!({
            "text": "用一句简短的中文回答用户问题，依据是这条观察输出：{{last_output}}"
        });
        attach_recent_execution_context_to_chat_args(
            &mut args,
            &loop_state,
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
            None,
        );
        let ctx = args
            .get("recent_execution_context")
            .and_then(|v| v.as_str())
            .expect("recent_execution_context must be injected");
        assert!(
            ctx.starts_with("original_user_request"),
            "original_user_request must be the first field, got: {ctx}"
        );
        assert!(
            ctx.contains("检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"),
            "verbatim user request must appear in context, got: {ctx}"
        );
        assert!(
            ctx.contains("last_output:"),
            "last_output must still be injected after original_user_request, got: {ctx}"
        );
    }

    #[test]
    fn attach_chat_context_omits_original_user_request_when_blank() {
        // 边界：ad-hoc / 后台调用没有 user 请求上下文时，传空串/纯空白不应在
        // context 里塞入空的 original_user_request 字段（避免下游 chat 误解为
        // "用户原话就是空字符串"）。但其它已有信号（last_output 等）正常注入。
        let mut loop_state = LoopState::new(2);
        loop_state.last_output = Some("hello\n".to_string());
        let mut args = json!({"text": "summarize: {{last_output}}"});
        attach_recent_execution_context_to_chat_args(&mut args, &loop_state, "   \t  \n  ", None);
        let ctx = args
            .get("recent_execution_context")
            .and_then(|v| v.as_str())
            .expect("last_output should still drive context injection");
        assert!(
            !ctx.contains("original_user_request"),
            "blank user_text must not produce original_user_request line, got: {ctx}"
        );
        assert!(
            ctx.contains("last_output:"),
            "last_output must still be injected, got: {ctx}"
        );
    }

    #[test]
    fn attach_chat_context_does_not_overwrite_existing_recent_execution_context() {
        // 幂等性回归：上游若已经显式给出 recent_execution_context（罕见但合法），
        // 这里不得覆盖，避免破坏调用方意图。
        let mut loop_state = LoopState::new(2);
        loop_state.last_output = Some("ignored".to_string());
        let mut args = json!({
            "text": "summarize",
            "recent_execution_context": "preset by upstream"
        });
        attach_recent_execution_context_to_chat_args(
            &mut args,
            &loop_state,
            "user asks something",
            None,
        );
        assert_eq!(
            args.get("recent_execution_context").and_then(|v| v.as_str()),
            Some("preset by upstream"),
            "must be idempotent when caller already set the field"
        );
    }

    #[test]
    fn auto_locator_preserves_explicit_existing_path() {
        // F8 回归用例：当 LLM 显式给的 path 是真实存在的具体文件时（典型场景：
        // 多文件 read 链路第二个 read_file），AUTO_LOCATOR 不得覆盖它。
        let root = TempDirGuard::new("explicit_existing");
        let readme = root.path.join("README.md");
        let notes = root.path.join("notes.md");
        fs::write(&readme, "# readme\n").expect("write readme");
        fs::write(&notes, "# notes\n").expect("write notes");
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), notes.display().to_string());
        let mut args = json!({"path": readme.display().to_string()});
        let rewritten = rewrite_args_with_auto_locator_path("read_file", &mut args, &loop_state);
        assert!(!rewritten, "explicit existing path must not be rewritten");
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(readme.display().to_string().as_str())
        );
    }

    #[test]
    fn render_output_contract_returns_none_for_default_contract() {
        // §7.1: 默认 contract（response_shape=Free / semantic_kind=None / 无 hint /
        // 无 delivery_required / 无 requires_content_evidence）对下游 chat skill 没
        // 任何约束力，render 必须返回 None，避免给 ad-hoc 调用注入噪声。
        let contract = IntentOutputContract::default();
        assert!(render_output_contract_for_chat(&contract).is_none());
    }

    #[test]
    fn render_output_contract_for_existence_with_path_includes_must_include_tokens() {
        // §7.1: ExistenceWithPath 是 act_find_service_file 类失败的核心 semantic_kind。
        // contract 必须把 "yes/no token + 路径子串" 这条硬规则字面输出，
        // 让 chat skill prompt 端能看见 must_include_tokens / no_paraphrase。
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            ..IntentOutputContract::default()
        };
        let rendered =
            render_output_contract_for_chat(&contract).expect("non-default contract must render");
        assert!(
            rendered.contains("response_shape: one_sentence"),
            "shape must be in render: {rendered}"
        );
        assert!(
            rendered.contains("semantic_kind: existence_with_path"),
            "semantic_kind must be in render: {rendered}"
        );
        assert!(
            rendered.contains("locator_hint: rustclaw.service"),
            "locator_hint must be surfaced: {rendered}"
        );
        assert!(
            rendered.contains("must_include_tokens:") && rendered.contains("yes/no"),
            "must_include_tokens line for existence_with_path must mention yes/no: {rendered}"
        );
        assert!(
            rendered.contains("no_paraphrase"),
            "no_paraphrase rule must be surfaced for existence_with_path: {rendered}"
        );
    }

    #[test]
    fn attach_chat_context_injects_output_contract_at_top() {
        // §7.1 act_find_service_file 回归用例：normalizer 已经标了
        // existence_with_path + locator_hint=rustclaw.service，但历史上
        // chat skill 完全看不见，把"有没有 + 路径"答成"这是 systemd 文件"。
        // 本注入必须把 contract 字段以最高优先级置顶，比 original_user_request
        // 还要靠前——它告诉 chat 回答必须长成什么样。
        let mut loop_state = LoopState::new(2);
        loop_state.last_output = Some("/home/guagua/rustclaw/rustclaw.service".to_string());
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::OneSentence,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            ..IntentOutputContract::default()
        };
        let mut args = json!({
            "text": "用一句简短的中文回答用户问题，依据是这条观察输出：{{last_output}}"
        });
        attach_recent_execution_context_to_chat_args(
            &mut args,
            &loop_state,
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
            Some(&contract),
        );
        let ctx = args
            .get("recent_execution_context")
            .and_then(|v| v.as_str())
            .expect("recent_execution_context must be injected");
        assert!(
            ctx.starts_with("output_contract"),
            "output_contract must be the first field (highest priority), got: {ctx}"
        );
        let oc_pos = ctx
            .find("output_contract")
            .expect("output_contract block must appear");
        let our_pos = ctx
            .find("original_user_request")
            .expect("original_user_request block must still appear");
        assert!(
            oc_pos < our_pos,
            "output_contract must come before original_user_request, got: {ctx}"
        );
        assert!(
            ctx.contains("semantic_kind: existence_with_path"),
            "semantic_kind line must be in injected ctx: {ctx}"
        );
        assert!(
            ctx.contains("must_include_tokens:") && ctx.contains("yes/no"),
            "must_include_tokens line must mention yes/no for existence_with_path: {ctx}"
        );
        assert!(
            ctx.contains("last_output:"),
            "last_output must still be present: {ctx}"
        );
    }

    #[test]
    fn attach_chat_context_skips_output_contract_when_default() {
        // §7.1: 当 contract 是默认值（response_shape=Free / semantic_kind=None /
        // 无 hint），不应在 ctx 里塞 output_contract 块（render 返回 None），
        // 避免给非 contract-driven 路径加噪声。但其它已有信号正常注入。
        let mut loop_state = LoopState::new(2);
        loop_state.last_output = Some("hello\n".to_string());
        let default_contract = IntentOutputContract::default();
        let mut args = json!({"text": "summarize: {{last_output}}"});
        attach_recent_execution_context_to_chat_args(
            &mut args,
            &loop_state,
            "请总结一下",
            Some(&default_contract),
        );
        let ctx = args
            .get("recent_execution_context")
            .and_then(|v| v.as_str())
            .expect("recent_execution_context must be injected");
        assert!(
            !ctx.contains("output_contract"),
            "default contract must not produce an output_contract block, got: {ctx}"
        );
        assert!(
            ctx.contains("original_user_request"),
            "original_user_request must still be in ctx, got: {ctx}"
        );
    }
}
