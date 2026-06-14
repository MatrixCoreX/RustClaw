use super::*;

pub(super) fn rewrite_sqlite_table_listing_plan_to_db_basic(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let route_requested_listing = route_result.is_some_and(route_requests_sqlite_table_listing);
    let sqlite_text_read_path = actions
        .iter()
        .find(|action| action_is_text_read_of_sqlite_path(action))
        .and_then(sqlite_locator_path_from_action);
    if !route_requested_listing && sqlite_text_read_path.is_none() {
        return actions;
    }
    let Some(db_path) = route_result
        .and_then(|route| sqlite_locator_path_for_route(route, auto_locator_path))
        .or(sqlite_text_read_path)
    else {
        return actions;
    };
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !action_should_be_sqlite_table_query(action) {
            continue;
        }
        *action = AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "list_tables",
                "db_path": db_path,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_table_listing_to_db_basic");
    }
    rewritten
}

pub(super) fn sqlite_locator_path_from_action(action: &AgentAction) -> Option<String> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            ["db_path", "path"]
                .into_iter()
                .filter_map(|key| args.get(key).and_then(Value::as_str))
                .map(str::trim)
                .find(|path| {
                    let lower = path.to_ascii_lowercase();
                    lower.ends_with(".sqlite") || lower.ends_with(".db")
                })
                .map(ToString::to_string)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => None,
    }
}

pub(super) fn action_can_serve_sqlite_schema_version_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
            }
            if skill == "read_file" || skill == "run_cmd" {
                return true;
            }
            if skill != "system_basic" {
                return false;
            }
            args.get("action")
                .and_then(Value::as_str)
                .map(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "read_range"
                            | "read"
                            | "read_file"
                            | "run_cmd"
                            | "extract_field"
                            | "extract_fields"
                            | "schema_version"
                            | "sqlite_schema_version"
                    )
                })
                .unwrap_or(true)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

pub(super) fn value_is_schema_version_field(value: &Value) -> bool {
    value
        .as_str()
        .map(str::trim)
        .map(|field| field.eq_ignore_ascii_case("schema_version"))
        .unwrap_or(false)
}

pub(super) fn action_should_be_sqlite_schema_version_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
            }
            if skill != "system_basic" {
                return false;
            }
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            match action_name.to_ascii_lowercase().as_str() {
                "schema_version" | "sqlite_schema_version" => true,
                "extract_field" => args
                    .get("field_path")
                    .is_some_and(value_is_schema_version_field),
                "extract_fields" => args
                    .get("field_paths")
                    .and_then(Value::as_array)
                    .filter(|fields| fields.len() == 1)
                    .and_then(|fields| fields.first())
                    .is_some_and(value_is_schema_version_field),
                _ => false,
            }
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

pub(super) fn rewrite_sqlite_schema_version_plan_to_db_basic(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let route_path =
        route_result.and_then(|route| sqlite_locator_path_for_route(route, auto_locator_path));
    let route_requests_schema_version =
        route_result.is_some_and(route_requests_sqlite_schema_version);
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !(action_should_be_sqlite_schema_version_query(action)
            || (route_requests_schema_version
                && action_can_serve_sqlite_schema_version_query(action)))
        {
            continue;
        }
        let Some(db_path) = route_path
            .clone()
            .or_else(|| sqlite_locator_path_from_action(action))
        else {
            continue;
        };
        *action = AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "schema_version",
                "db_path": db_path,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_schema_version_to_db_basic");
    }
    rewritten
}

pub(super) fn rewrite_sqlite_count_query_to_requested_schema_column(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
            action
        else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("db_basic") {
            continue;
        }
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("sqlite_query");
        if !action_name.eq_ignore_ascii_case("sqlite_query") {
            continue;
        }
        let Some(sql) = args.get("sql").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some(db_path) = args.get("db_path").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some((table, suffix)) = parse_sqlite_count_star_query(sql) else {
            continue;
        };
        if sqlite_count_suffix_has_grouping(&suffix) {
            continue;
        }
        let Some(column) = requested_schema_column_for_sqlite_count_rewrite(
            route,
            user_text,
            original_user_text,
            db_path,
            &table,
            &suffix,
        ) else {
            continue;
        };
        let rewritten_sql = format!(
            "SELECT {} FROM {}{}",
            quote_sqlite_identifier(&column),
            quote_sqlite_identifier(&table),
            suffix
        );
        args["sql"] = Value::String(rewritten_sql);
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_count_query_to_requested_schema_column");
    }
    rewritten
}

pub(super) fn rewrite_sqlite_table_probe_to_requested_schema_value(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::SqliteTableListing
                | crate::OutputSemanticKind::SqliteTableNamesOnly
                | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
                | crate::OutputSemanticKind::SqliteSchemaVersion
        )
    {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
            action
        else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("db_basic") {
            continue;
        }
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !action_name.eq_ignore_ascii_case("list_tables") {
            continue;
        }
        let Some(db_path) = args.get("db_path").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some(sql) =
            sqlite_schema_value_query_for_route(route, user_text, original_user_text, db_path)
        else {
            continue;
        };
        args["action"] = Value::String("sqlite_query".to_string());
        args["sql"] = Value::String(sql);
        changed = true;
    }
    if changed {
        info!("plan_rewrite_sqlite_table_probe_to_requested_schema_value");
    }
    rewritten
}

pub(super) fn sqlite_schema_value_query_for_route(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    db_path: &str,
) -> Option<String> {
    let source = [
        route.resolved_intent.as_str(),
        route.output_contract.locator_hint.as_str(),
        user_text,
        original_user_text.unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    let source_tokens = identifier_tokens(&source);
    let table = requested_sqlite_table_for_source(db_path, &source_tokens)?;
    let columns = sqlite_table_columns(db_path, &table)?;
    let filters = sqlite_source_value_filters(db_path, &table, &columns, &source_tokens);
    if filters.is_empty() {
        return None;
    }
    let filter_columns = filters
        .iter()
        .map(|(column, _)| column.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let target_columns = columns
        .into_iter()
        .filter(|column| {
            let lower = column.to_ascii_lowercase();
            identifier_tokens_contain_schema_name(&source_tokens, &lower)
                && !filter_columns.contains(&lower)
        })
        .collect::<Vec<_>>();
    if target_columns.len() != 1 {
        return None;
    }
    let target_column = &target_columns[0];
    let where_sql = filters
        .iter()
        .map(|(column, value)| {
            format!(
                "{} = {}",
                quote_sqlite_identifier(column),
                quote_sqlite_string_literal(value)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT {} FROM {} WHERE {}",
        quote_sqlite_identifier(target_column),
        quote_sqlite_identifier(&table),
        where_sql
    );
    sqlite_query_single_scalar_preview(db_path, &sql).map(|_| sql)
}

pub(super) fn requested_sqlite_table_for_source(
    db_path: &str,
    source_tokens: &std::collections::HashSet<String>,
) -> Option<String> {
    let tables = sqlite_database_table_names(db_path)?;
    let candidates = tables
        .into_iter()
        .filter(|table| identifier_tokens_contain_schema_name(source_tokens, table))
        .collect::<Vec<_>>();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

pub(super) fn sqlite_database_table_names(db_path: &str) -> Option<Vec<String>> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .ok()?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .ok()?
        .filter_map(Result::ok)
        .filter(|table| !table.trim().is_empty())
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

pub(super) fn sqlite_source_value_filters(
    db_path: &str,
    table: &str,
    columns: &[String],
    source_tokens: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    let mut filters = Vec::new();
    for column in columns {
        let lower = column.to_ascii_lowercase();
        if !identifier_tokens_contain_schema_name(source_tokens, &lower) {
            continue;
        }
        let Some(values) = sqlite_distinct_column_values(db_path, table, column, 100) else {
            continue;
        };
        let matches = values
            .into_iter()
            .filter(|value| sqlite_value_mentioned_by_source(value, source_tokens))
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            filters.push((column.clone(), matches[0].clone()));
        }
    }
    filters
}

pub(super) fn sqlite_distinct_column_values(
    db_path: &str,
    table: &str,
    column: &str,
    limit: usize,
) -> Option<Vec<String>> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let sql = format!(
        "SELECT DISTINCT {} FROM {} WHERE {} IS NOT NULL LIMIT {}",
        quote_sqlite_identifier(column),
        quote_sqlite_identifier(table),
        quote_sqlite_identifier(column),
        limit.clamp(1, 500)
    );
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows = stmt
        .query_map([], |row| Ok(sqlite_value_ref_to_string(row.get_ref(0)?)))
        .ok()?
        .filter_map(Result::ok)
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    Some(rows)
}

pub(super) fn sqlite_value_mentioned_by_source(
    value: &str,
    source_tokens: &std::collections::HashSet<String>,
) -> bool {
    let value_tokens = identifier_tokens(&value.to_ascii_lowercase());
    !value_tokens.is_empty()
        && value_tokens
            .iter()
            .all(|token| source_tokens.contains(token))
}

pub(super) fn sqlite_query_single_scalar_preview(db_path: &str, sql: &str) -> Option<String> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let mut stmt = conn.prepare(sql).ok()?;
    if stmt.column_count() != 1 {
        return None;
    }
    let mut rows = stmt.query([]).ok()?;
    let row = rows.next().ok()??;
    let value = sqlite_value_ref_to_string(row.get_ref(0).ok()?)?;
    if rows.next().ok()?.is_some() {
        return None;
    }
    Some(value)
}

pub(super) fn sqlite_value_ref_to_string(value: rusqlite::types::ValueRef<'_>) -> Option<String> {
    match value {
        rusqlite::types::ValueRef::Null => None,
        rusqlite::types::ValueRef::Integer(value) => Some(value.to_string()),
        rusqlite::types::ValueRef::Real(value) => Some(value.to_string()),
        rusqlite::types::ValueRef::Text(value) => {
            Some(String::from_utf8_lossy(value).trim().to_string())
        }
        rusqlite::types::ValueRef::Blob(_) => None,
    }
}

pub(super) fn parse_sqlite_count_star_query(sql: &str) -> Option<(String, String)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r#"(?is)^\s*select\s+count\s*\(\s*\*\s*\)(?:\s+as\s+(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[A-Za-z_][A-Za-z0-9_]*))?\s+from\s+(?:"([^"]+)"|`([^`]+)`|\[([^\]]+)\]|([A-Za-z_][A-Za-z0-9_]*))(?P<suffix>.*?)\s*;?\s*$"#,
        )
        .expect("sqlite count query regex")
    });
    let captures = re.captures(sql)?;
    let table = (1..=4)
        .filter_map(|idx| captures.get(idx).map(|m| m.as_str().trim()))
        .find(|value| !value.is_empty())?
        .to_string();
    let suffix = captures
        .name("suffix")
        .map(|m| m.as_str().trim_end_matches(';').to_string())
        .unwrap_or_default();
    Some((table, suffix))
}

pub(super) fn sqlite_count_suffix_has_grouping(suffix: &str) -> bool {
    let padded = format!(" {} ", suffix.to_ascii_lowercase());
    padded.contains(" group by ") || padded.contains(" having ")
}

pub(super) fn requested_schema_column_for_sqlite_count_rewrite(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    db_path: &str,
    table: &str,
    sql_suffix: &str,
) -> Option<String> {
    let columns = sqlite_table_columns(db_path, table)?;
    let source = [
        route.resolved_intent.as_str(),
        route.output_contract.locator_hint.as_str(),
        user_text,
        original_user_text.unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    let source_tokens = identifier_tokens(&source);
    let suffix_tokens = identifier_tokens(&sql_suffix.to_ascii_lowercase());
    let candidates = columns
        .into_iter()
        .filter(|column| {
            let lower = column.to_ascii_lowercase();
            identifier_tokens_contain_schema_name(&source_tokens, &lower)
                && !identifier_tokens_contain_schema_name(&suffix_tokens, &lower)
        })
        .collect::<Vec<_>>();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

pub(super) fn sqlite_table_columns(db_path: &str, table: &str) -> Option<Vec<String>> {
    let path = resolve_sqlite_path_for_planner(db_path);
    let conn = rusqlite::Connection::open(path).ok()?;
    let pragma = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let mut stmt = conn.prepare(&pragma).ok()?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .ok()?
        .filter_map(Result::ok)
        .filter(|column| !column.trim().is_empty())
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

pub(super) fn resolve_sqlite_path_for_planner(db_path: &str) -> PathBuf {
    let raw = Path::new(db_path);
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(raw)
    }
}

pub(super) fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(super) fn quote_sqlite_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn identifier_tokens(text: &str) -> std::collections::HashSet<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(super) fn identifier_tokens_contain_schema_name(
    tokens: &std::collections::HashSet<String>,
    schema_name: &str,
) -> bool {
    let normalized = schema_name.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if tokens.contains(&normalized) {
        return true;
    }
    if let Some(singular) = normalized.strip_suffix('s') {
        if singular.len() >= 3 && tokens.contains(singular) {
            return true;
        }
    }
    let plural = format!("{normalized}s");
    tokens.contains(&plural)
}

pub(super) fn split_archive_locator_pair(hint: &str) -> Option<(String, String)> {
    let hint = hint.trim();
    for separator in ["|", "->", "=>"] {
        if let Some((left, right)) = hint.split_once(separator) {
            let left = left.trim();
            let right = right.trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left.to_string(), right.to_string()));
            }
        }
    }
    None
}

pub(super) fn is_supported_archive_path(path: &str) -> bool {
    let path_lower = path.trim().to_ascii_lowercase();
    path_lower.ends_with(".zip") || path_lower.ends_with(".tar.gz") || path_lower.ends_with(".tgz")
}

pub(super) fn archive_format_for_path(path: &str) -> &'static str {
    if path.trim().to_ascii_lowercase().ends_with(".zip") {
        "zip"
    } else {
        "tar.gz"
    }
}

pub(super) fn archive_unpack_pair_for_route(route: &RouteResult) -> Option<(String, String)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchiveUnpack {
        return None;
    }
    let (archive, dest) = split_archive_locator_pair(&route.output_contract.locator_hint)?;
    if !is_supported_archive_path(&archive) {
        return None;
    }
    Some((archive, dest))
}

pub(super) fn rewrite_archive_unpack_run_cmd_to_archive_basic(
    route_result: Option<&RouteResult>,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    let Some((archive, dest)) = archive_unpack_pair_for_route(route) else {
        return actions;
    };
    if actions.iter().any(action_is_archive_basic_unpack) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        let should_rewrite = action_skill_is_run_cmd(action) || action_is_archive_basic(action);
        if !should_rewrite {
            continue;
        }
        *action = AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: serde_json::json!({
                "action": "unpack",
                "archive": archive,
                "dest": dest,
            }),
        };
        changed = true;
        break;
    }
    if changed {
        info!("plan_rewrite_archive_unpack_plan_to_archive_basic");
    }
    rewritten
}

pub(super) fn archive_pack_pair_for_route(route: &RouteResult) -> Option<(String, String)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchivePack
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
    {
        return None;
    }
    let (source, archive) = split_archive_locator_pair(&route.output_contract.locator_hint)?;
    if is_supported_archive_path(&source) || !is_supported_archive_path(&archive) {
        return None;
    }
    Some((source, archive))
}

pub(super) fn archive_pack_pair_for_route_or_text(
    workspace_root: &Path,
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<(String, String)> {
    if let Some(pair) = archive_pack_pair_for_route(route) {
        return Some(pair);
    }
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ArchivePack
            | crate::OutputSemanticKind::FilesystemMutationResult
            | crate::OutputSemanticKind::GeneratedFileDelivery
    ) {
        return None;
    }

    let mut archive_candidates = Vec::<String>::new();
    for candidate in [
        Some(route.output_contract.locator_hint.as_str()),
        auto_locator_path,
        Some(user_text),
        original_user_text,
        Some(route.resolved_intent.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        collect_archive_locator_candidates(&mut archive_candidates, candidate);
    }
    let archive = archive_candidates
        .iter()
        .find(|candidate| is_supported_archive_path(candidate))
        .cloned()?;

    let mut source_candidates = Vec::<String>::new();
    for text in [
        Some(user_text),
        original_user_text,
        Some(route.resolved_intent.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            let candidate = locator.locator_hint.trim();
            if candidate.is_empty()
                || is_supported_archive_path(candidate)
                || source_candidates
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(candidate))
            {
                continue;
            }
            let resolved = resolve_workspace_path(workspace_root, candidate);
            if resolved.exists() {
                source_candidates.push(candidate.to_string());
            }
        }
    }
    let source = source_candidates.into_iter().next()?;
    Some((source, archive))
}

pub(super) fn collect_archive_locator_candidates(out: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if is_supported_archive_path(text)
        && !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(text))
    {
        out.push(text.to_string());
    }
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
    {
        let candidate = locator.locator_hint.trim();
        if is_supported_archive_path(candidate)
            && !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(candidate))
        {
            out.push(candidate.to_string());
        }
    }
    for candidate in crate::delivery_utils::extract_filename_candidates(text) {
        if is_supported_archive_path(&candidate)
            && !out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&candidate))
        {
            out.push(candidate);
        }
    }
}

pub(super) fn action_args(action: &AgentAction) -> Option<&Value> {
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => Some(args),
        _ => None,
    }
}

pub(super) fn action_is_archive_basic(action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(str::trim)
        .is_some_and(|skill| skill.eq_ignore_ascii_case("archive_basic"))
}

pub(super) fn action_is_archive_basic_pack(action: &AgentAction) -> bool {
    action_is_archive_basic(action)
        && action_args(action)
            .and_then(|args| args.get("action"))
            .and_then(Value::as_str)
            .is_some_and(|action| action.trim().eq_ignore_ascii_case("pack"))
}

pub(super) fn action_is_archive_basic_unpack(action: &AgentAction) -> bool {
    action_is_archive_basic(action)
        && action_args(action)
            .and_then(|args| args.get("action"))
            .and_then(Value::as_str)
            .is_some_and(|action| action.trim().eq_ignore_ascii_case("unpack"))
}

pub(super) fn action_skill_is_run_cmd(action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(str::trim)
        .is_some_and(|skill| skill.eq_ignore_ascii_case("run_cmd"))
}

pub(super) fn run_cmd_command_arg(action: &AgentAction) -> Option<&str> {
    action_args(action)
        .and_then(|args| args.get("command").or_else(|| args.get("cmd")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

pub(super) fn shell_head_limit_from_words(words: &[String]) -> Option<u64> {
    let mut idx = 0;
    while idx < words.len() {
        if !command_basename(&words[idx]).eq_ignore_ascii_case("head") {
            idx += 1;
            continue;
        }
        let mut head_idx = idx + 1;
        while head_idx < words.len() {
            let word = words[head_idx].trim();
            if word.is_empty() {
                head_idx += 1;
                continue;
            }
            if word == "-n" || word == "--lines" {
                return words
                    .get(head_idx + 1)
                    .and_then(|value| parse_shell_line_count(value));
            }
            if let Some(value) = word.strip_prefix("-n") {
                return parse_shell_line_count(value);
            }
            if let Some(value) = word.strip_prefix("--lines=") {
                return parse_shell_line_count(value);
            }
            if let Some(value) = word.strip_prefix('-') {
                return parse_shell_line_count(value);
            }
            break;
        }
        return Some(10);
    }
    None
}

pub(super) fn process_basic_ps_limit_from_command(command: &str) -> Option<u64> {
    let words = shell_like_words(command);
    let first = words.first().map(|word| command_basename(word))?;
    if !first.eq_ignore_ascii_case("ps") {
        return None;
    }

    let lower = command.to_ascii_lowercase();
    if words.iter().any(|word| {
        matches!(
            command_basename(word).to_ascii_lowercase().as_str(),
            "awk" | "sed" | "grep" | "egrep" | "fgrep" | "xargs" | "cut" | "kill" | "pkill"
        )
    }) {
        return None;
    }
    if words.iter().any(|word| {
        matches!(
            word.as_str(),
            "-p" | "--pid" | "-C" | "--ppid" | "--quick-pid"
        ) || word.starts_with("--pid=")
            || word.starts_with("--ppid=")
            || word.starts_with("--quick-pid=")
    }) {
        return None;
    }

    let head_limit = shell_head_limit_from_words(&words);
    let sorted_by_cpu = lower.contains("--sort=-%cpu")
        || lower.contains("--sort=-pcpu")
        || (lower.contains("sort") && (lower.contains("%cpu") || lower.contains("pcpu")));
    let process_cpu_columns =
        lower.contains("%cpu") || lower.contains("pcpu") || lower.contains("ps aux");
    if !sorted_by_cpu && !(head_limit.is_some() && process_cpu_columns) {
        return None;
    }

    let limit = head_limit
        .map(|rows| rows.saturating_sub(1).max(1))
        .unwrap_or(10)
        .clamp(1, 50);
    Some(limit)
}

pub(super) fn rewrite_process_ps_run_cmd_to_process_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !process_basic_available_for_plan(state) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !action_skill_is_run_cmd(action) {
            continue;
        }
        let Some(args) = action_args(action) else {
            continue;
        };
        let Some(command) = run_cmd_command_arg(action) else {
            continue;
        };
        if args
            .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            == Some(true)
            || should_preserve_user_supplied_shell_command(command, user_text, original_user_text)
        {
            continue;
        }
        let Some(limit) = process_basic_ps_limit_from_command(command) else {
            continue;
        };
        *action = AgentAction::CallSkill {
            skill: "process_basic".to_string(),
            args: serde_json::json!({
                "action": "ps",
                "limit": limit,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_process_ps_run_cmd_to_process_basic");
    }
    rewritten
}

pub(super) fn split_shell_sequence_command_with_policy(
    command: &str,
    split_conditionals: bool,
) -> Option<Vec<String>> {
    if command.contains("<<") {
        return None;
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut command_sub_depth = 0usize;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            if quote != Some('\'') {
                escaped = true;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            current.push(ch);
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
            current.push(ch);
            continue;
        }
        if ch == '$' && chars.peek() == Some(&'(') {
            current.push(ch);
            current.push('(');
            chars.next();
            command_sub_depth += 1;
            continue;
        }
        if command_sub_depth > 0 {
            if ch == '(' {
                command_sub_depth += 1;
            } else if ch == ')' {
                command_sub_depth = command_sub_depth.saturating_sub(1);
            }
            current.push(ch);
            continue;
        }
        if ch == '&' && chars.peek() == Some(&'&') && split_conditionals {
            let part = current.trim();
            if part.is_empty() {
                return None;
            }
            parts.push(part.to_string());
            current.clear();
            chars.next();
            continue;
        }
        if ch == ';' || ch == '\n' {
            let part = current.trim();
            if part.is_empty() {
                return None;
            }
            parts.push(part.to_string());
            current.clear();
            continue;
        }
        current.push(ch);
    }
    let part = current.trim();
    if part.is_empty() {
        return None;
    }
    parts.push(part.to_string());
    if parts.len() < 2 || !shell_sequence_parts_can_run_independently(&parts) {
        return None;
    }
    Some(parts)
}

pub(super) fn planner_failure_fallback_first_command(
    command: &str,
    split_conditionals: bool,
) -> Option<String> {
    if !split_conditionals || command.contains("<<") {
        return None;
    }
    let split_at = top_level_shell_or_operator_byte_index(command)?;
    let first = command[..split_at].trim();
    let fallback = command[split_at + 2..].trim();
    if first.is_empty() || fallback.is_empty() {
        return None;
    }
    let parts = vec![first.to_string(), fallback.to_string()];
    if !shell_sequence_parts_can_run_independently(&parts) {
        return None;
    }
    Some(first.to_string())
}

pub(super) fn top_level_shell_or_operator_byte_index(command: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut command_sub_depth = 0usize;
    let mut chars = command.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            if quote != Some('\'') {
                escaped = true;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"' | '`') {
            quote = Some(ch);
            continue;
        }
        if ch == '$' && chars.peek().is_some_and(|(_, next)| *next == '(') {
            chars.next();
            command_sub_depth += 1;
            continue;
        }
        if command_sub_depth > 0 {
            if ch == '(' {
                command_sub_depth += 1;
            } else if ch == ')' {
                command_sub_depth = command_sub_depth.saturating_sub(1);
            }
            continue;
        }
        if ch == '|' && chars.peek().is_some_and(|(_, next)| *next == '|') {
            return Some(idx);
        }
    }
    None
}

pub(super) fn request_text_contains_shell_conditional_operator(text: &str) -> bool {
    text.contains("&&") || text.contains("||")
}

pub(super) fn should_split_planner_introduced_shell_conditionals(
    user_text: &str,
    original_user_text: Option<&str>,
) -> bool {
    !request_text_contains_shell_conditional_operator(user_text)
        && !original_user_text.is_some_and(request_text_contains_shell_conditional_operator)
}

pub(super) fn request_text_contains_command_verbatim(text: &str, command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    text.contains(command)
        || shell_command_surface_for_verbatim_compare(text)
            .contains(&shell_command_surface_for_verbatim_compare(command))
}

pub(super) fn shell_command_surface_for_verbatim_compare(value: &str) -> String {
    value.replace("\\;", ";")
}

pub(super) fn should_preserve_user_supplied_shell_command(
    command: &str,
    user_text: &str,
    original_user_text: Option<&str>,
) -> bool {
    request_text_contains_command_verbatim(user_text, command)
        || original_user_text
            .is_some_and(|text| request_text_contains_command_verbatim(text, command))
}

pub(super) fn shell_sequence_parts_can_run_independently(parts: &[String]) -> bool {
    parts
        .iter()
        .enumerate()
        .all(|(idx, part)| shell_sequence_part_can_run_independently(part, idx + 1 == parts.len()))
}
