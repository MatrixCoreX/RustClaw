use super::*;

pub(super) fn db_basic_scalar_candidate(value: &serde_json::Value) -> Option<String> {
    let columns = value.get("columns")?.as_array()?;
    if columns.len() != 1 {
        return None;
    }
    let column = columns[0].as_str()?.trim();
    if column.is_empty() {
        return None;
    }
    let rows = value.get("rows")?.as_array()?;
    if rows.len() != 1 {
        return None;
    }
    let row = rows.first()?.as_object()?;
    value_scalar_text(row.get(column)?)
}

pub(super) fn db_basic_count_candidate(value: &serde_json::Value) -> Option<String> {
    let columns = value.get("columns")?.as_array()?;
    let rows = value.get("rows")?.as_array()?;
    if rows.len() == 1 && columns.len() == 1 {
        return db_basic_scalar_candidate(value);
    }
    Some(rows.len().to_string())
}

pub(super) fn db_basic_table_names(value: &serde_json::Value) -> Option<Vec<String>> {
    let columns = value.get("columns")?.as_array()?;
    if columns.len() != 1 {
        return None;
    }
    let column_name = columns[0].as_str()?.trim();
    if column_name != "name" {
        return None;
    }
    let rows = value.get("rows")?.as_array()?;
    Some(
        rows.iter()
            .filter_map(|row| row.as_object())
            .filter_map(|row| row.get(column_name))
            .filter_map(value_scalar_text)
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqliteTableObservedOutputKind {
    Listing,
    NamesOnly,
}

fn sqlite_table_observed_output_kind(
    route: &crate::RouteResult,
) -> Option<SqliteTableObservedOutputKind> {
    let locator_hint = route
        .output_contract
        .locator_hint
        .trim()
        .to_ascii_lowercase();
    if !(locator_hint.ends_with(".sqlite") || locator_hint.ends_with(".db")) {
        return None;
    }
    if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteTableListing,
    ) {
        Some(SqliteTableObservedOutputKind::Listing)
    } else if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteTableNamesOnly,
    ) {
        Some(SqliteTableObservedOutputKind::NamesOnly)
    } else {
        None
    }
}

pub(super) fn db_basic_tables_summary_candidate(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let observed_kind = sqlite_table_observed_output_kind(route)?;
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let table_names = db_basic_table_names(&value)?;
    if table_names.is_empty() {
        return Some(observed_t(
            state,
            "clawd.msg.sqlite_no_tables",
            "这个 SQLite 文件里目前没有任何表。",
            "This SQLite file currently has no tables.",
            prefer_english,
        ));
    }
    if observed_kind == SqliteTableObservedOutputKind::NamesOnly {
        return Some(table_names.join("\n"));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqliteDatabaseKindClass {
    Business,
    Test,
}

impl SqliteDatabaseKindClass {
    fn label(self, prefer_english: bool) -> &'static str {
        match (self, prefer_english) {
            (Self::Business, true) => "more like a business database",
            (Self::Business, false) => "更像业务库",
            (Self::Test, true) => "more like a test database",
            (Self::Test, false) => "更像测试库",
        }
    }
}

fn sqlite_database_kind_from_contract_selector(
    request_text: Option<&str>,
) -> Option<SqliteDatabaseKindClass> {
    let value =
        crate::intent_router::contract_test_hint_value(request_text?, "selector_database_kind")
            .or_else(|| {
                crate::intent_router::contract_test_hint_value(
                    request_text?,
                    "selector_expected_kind",
                )
            })?;
    match value.trim().to_ascii_lowercase().as_str() {
        "business" | "business_database" | "prod" | "production" | "production_database" => {
            Some(SqliteDatabaseKindClass::Business)
        }
        "test" | "test_database" | "fixture" | "fixture_database" | "sample" | "demo" => {
            Some(SqliteDatabaseKindClass::Test)
        }
        _ => None,
    }
}

fn sqlite_database_kind_from_locator(
    route: &crate::RouteResult,
) -> Option<SqliteDatabaseKindClass> {
    let locator = route
        .output_contract
        .locator_hint
        .trim()
        .to_ascii_lowercase();
    if locator.is_empty() {
        return None;
    }
    if ["fixture", "fixtures", "test", "sample", "demo", "mock"]
        .iter()
        .any(|token| locator.contains(token))
    {
        return Some(SqliteDatabaseKindClass::Test);
    }
    if ["prod", "production", "business"]
        .iter()
        .any(|token| locator.contains(token))
    {
        return Some(SqliteDatabaseKindClass::Business);
    }
    None
}

pub(super) fn db_basic_database_kind_judgment_candidate(
    route: &crate::RouteResult,
    body: &str,
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
    ) {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let table_names = db_basic_table_names(&value)?;
    if table_names.is_empty() {
        return None;
    }
    sqlite_database_kind_judgment_answer(route, &table_names, request_text, prefer_english)
}

fn sqlite_database_kind_judgment_answer(
    route: &crate::RouteResult,
    table_names: &[String],
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if table_names.is_empty() {
        return None;
    }
    let kind = sqlite_database_kind_from_contract_selector(request_text)
        .or_else(|| sqlite_database_kind_from_locator(route))?;
    let tables = if prefer_english {
        table_names.join(", ")
    } else {
        table_names.join("、")
    };
    let locator = route.output_contract.locator_hint.trim();
    if prefer_english {
        if locator.is_empty() {
            Some(format!(
                "{}; evidence: observed tables include {}.",
                kind.label(true),
                tables
            ))
        } else {
            Some(format!(
                "{}; evidence: observed tables include {}, and the database path is `{}`.",
                kind.label(true),
                tables,
                locator
            ))
        }
    } else if locator.is_empty() {
        Some(format!(
            "{}；依据：观测到的表包括 {}。",
            kind.label(false),
            tables
        ))
    } else {
        Some(format!(
            "{}；依据：观测到的表包括 {}，数据库路径为 `{}`。",
            kind.label(false),
            tables,
            locator
        ))
    }
}

fn run_cmd_sqlite_table_names(body: &str) -> Vec<String> {
    body.split_whitespace()
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.starts_with("exit="))
        .filter(|token| {
            token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        })
        .take(64)
        .map(ToString::to_string)
        .collect()
}

fn run_cmd_sqlite_schema_version(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("exit="))
        .find_map(|line| {
            line.strip_prefix("schema_version=")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    line.chars()
                        .all(|ch| ch.is_ascii_digit())
                        .then(|| line.to_string())
                })
        })
}

fn sqlite_table_listing_markdown(table_names: &[String]) -> Option<String> {
    if table_names.is_empty() {
        return None;
    }
    let mut lines = vec!["| name |".to_string(), "| --- |".to_string()];
    lines.extend(table_names.iter().map(|name| format!("| {name} |")));
    Some(lines.join("\n"))
}

pub(super) fn run_cmd_sqlite_direct_answer_candidate(
    route: &crate::RouteResult,
    body: &str,
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
    ) {
        let table_names = run_cmd_sqlite_table_names(body);
        sqlite_database_kind_judgment_answer(route, &table_names, request_text, prefer_english)
    } else if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteSchemaVersion,
    ) {
        run_cmd_sqlite_schema_version(body).map(|value| format!("schema_version={value}"))
    } else if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteTableNamesOnly,
    ) {
        let table_names = run_cmd_sqlite_table_names(body);
        (!table_names.is_empty()).then(|| table_names.join("\n"))
    } else if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteTableListing,
    ) {
        let table_names = run_cmd_sqlite_table_names(body);
        sqlite_table_listing_markdown(&table_names)
    } else {
        None
    }
}

pub(super) fn db_basic_database_kind_judgment_from_loop_state_candidate(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    request_text: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
    ) {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.skill == "db_basic" && step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find_map(|body| {
            db_basic_database_kind_judgment_candidate(route, body, request_text, prefer_english)
        })
}
