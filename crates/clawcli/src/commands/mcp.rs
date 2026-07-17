use anyhow::{Context, Result};
use serde_json::Value;

use crate::{client, output, resources, McpCommand};

use super::common::get_v1_json;

pub(crate) fn run_mcp(base_url: &str, key: &str, command: &McpCommand) -> Result<()> {
    match command {
        McpCommand::List { json } => run_server_list(base_url, key, None, *json),
        McpCommand::Status { server, json } => {
            run_server_list(base_url, key, server.as_deref(), *json)
        }
        McpCommand::Tools { server, json } => {
            run_tool_list(base_url, key, server.as_deref(), *json)
        }
        McpCommand::Test { server, json } => run_probe(base_url, key, server, *json),
    }
}

fn run_server_list(
    base_url: &str,
    key: &str,
    server: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let mut body = get_v1_json(base_url, key, "/admin/mcp/servers", "mcp_servers")?;
    if let Some(server) = server {
        let matching = body
            .pointer("/data/servers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("server_id").and_then(Value::as_str) == Some(server))
            .cloned()
            .collect::<Vec<_>>();
        if matching.is_empty() {
            anyhow::bail!("mcp_server_not_found");
        }
        body["data"]["servers"] = Value::Array(matching);
    }
    if json_output {
        output::print_json_pretty(&body);
        return Ok(());
    }
    let rows = server_rows(&body, server);
    println!(
        "{}\t{}\t{}\t{}\t{}",
        resources::text("mcp.column.server"),
        resources::text("mcp.column.state"),
        resources::text("mcp.column.transport"),
        resources::text("mcp.column.tools"),
        resources::text("mcp.column.error"),
    );
    for row in rows {
        println!("{}\t{}\t{}\t{}\t{}", row[0], row[1], row[2], row[3], row[4]);
    }
    Ok(())
}

fn run_tool_list(base_url: &str, key: &str, server: Option<&str>, json_output: bool) -> Result<()> {
    let url = format!("{}/admin/mcp/tools", client::base_v1(base_url));
    let client = client::make_client()?;
    let mut request = client.get(&url).header("x-rustclaw-key", key);
    if let Some(server) = server {
        request = request.query(&[("server_id", server)]);
    }
    let response = request.send().context("mcp_tools_request_failed")?;
    let status = response.status();
    let body: Value = response.json().context("mcp_tools_response_invalid")?;
    if !status.is_success() {
        anyhow::bail!("mcp_tools_request_rejected:{status}");
    }
    if json_output {
        output::print_json_pretty(&body);
        return Ok(());
    }
    println!(
        "{}\t{}\t{}\t{}\t{}",
        resources::text("mcp.column.capability"),
        resources::text("mcp.column.server"),
        resources::text("mcp.column.effect"),
        resources::text("mcp.column.risk"),
        resources::text("mcp.column.required"),
    );
    for row in tool_rows(&body) {
        println!("{}\t{}\t{}\t{}\t{}", row[0], row[1], row[2], row[3], row[4]);
    }
    Ok(())
}

fn run_probe(base_url: &str, key: &str, server: &str, json_output: bool) -> Result<()> {
    let mut url = reqwest::Url::parse(&format!("{}/admin/mcp/servers/", client::base_v1(base_url)))
        .context("mcp_probe_url_invalid")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("mcp_probe_url_invalid"))?
        .pop_if_empty()
        .push(server)
        .push("test");
    let response = client::make_client()?
        .post(url)
        .header("x-rustclaw-key", key)
        .send()
        .context("mcp_probe_request_failed")?;
    let status = response.status();
    let body: Value = response.json().context("mcp_probe_response_invalid")?;
    if !status.is_success() {
        anyhow::bail!("mcp_probe_request_rejected:{status}");
    }
    if json_output {
        output::print_json_pretty(&body);
    } else {
        let probe = body.pointer("/data/probe").unwrap_or(&Value::Null);
        println!(
            "{}\t{}\t{}",
            resources::text("mcp.column.server"),
            resources::text("mcp.column.state"),
            resources::text("mcp.column.latency_ms"),
        );
        println!(
            "{}\t{}\t{}",
            probe
                .get("server_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            probe
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            probe
                .get("latency_ms")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        );
    }
    Ok(())
}

fn server_rows(body: &Value, server: Option<&str>) -> Vec<[String; 5]> {
    body.pointer("/data/servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            server
                .is_none_or(|server| item.get("server_id").and_then(Value::as_str) == Some(server))
        })
        .map(|item| {
            [
                string_field(item, "server_id"),
                string_field(item, "state"),
                string_field(item, "transport"),
                item.get("tool_count")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
                    .to_string(),
                string_field(item, "last_error_code"),
            ]
        })
        .collect()
}

fn tool_rows(body: &Value) -> Vec<[String; 5]> {
    body.pointer("/data/tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            [
                string_field(item, "capability"),
                string_field(item, "server_id"),
                item.pointer("/policy/effect")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                item.pointer("/policy/risk_level")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                item.get("required_args")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(","),
            ]
        })
        .collect()
}

fn string_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
