use serde_json::json;

use super::{server_rows, tool_rows};

#[test]
fn machine_server_projection_filters_without_parsing_prose() {
    let body = json!({
        "data": {
            "servers": [
                {"server_id": "alpha", "state": "ready", "transport": "stdio", "tool_count": 2},
                {"server_id": "beta", "state": "degraded", "transport": "streamable_http", "tool_count": 0, "last_error_code": "mcp_ping_timeout"}
            ]
        }
    });
    let rows = server_rows(&body, Some("beta"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], "beta");
    assert_eq!(rows[0][1], "degraded");
    assert_eq!(rows[0][4], "mcp_ping_timeout");
}

#[test]
fn machine_tool_projection_keeps_policy_and_schema_fields() {
    let body = json!({
        "data": {
            "tools": [{
                "capability": "mcp.alpha.lookup",
                "server_id": "alpha",
                "required_args": ["query", "scope"],
                "policy": {"effect": "observe", "risk_level": "low"}
            }]
        }
    });
    let rows = tool_rows(&body);
    assert_eq!(
        rows[0],
        ["mcp.alpha.lookup", "alpha", "observe", "low", "query,scope"]
    );
}
