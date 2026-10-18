//! `splitrail-dashboard mcp`: a Model Context Protocol server on stdio (BZL-137), so an agent can
//! ask where every plan quota stands, which tool and account to spend on next, and today's usage.
//! Answers come from the running dashboard's `status.json` (see `advisor`).
//!
//! Register it with Claude Code: `claude mcp add splitrail -- splitrail-dashboard mcp`.

use chrono::Utc;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn tools() -> Value {
    let empty = json!({"type": "object", "properties": {}});
    json!([
        {
            "name": "quotas",
            "description": "Every AI coding plan window on every machine (Claude per account and devbox, Warp, Kiro, Antigravity, Cursor, Droid...): percent used, exact reset time, and the percent it is on pace to reach by the reset. Sorted by soonest reset.",
            "inputSchema": empty,
        },
        {
            "name": "where_to_spend",
            "description": "Which CLI/account to use next to make the most of quota that would otherwise expire unused: ranks each plan by the share projected to be unused at its weekly/monthly reset, per day until that reset, and marks tools whose window is nearly full (with when they come back).",
            "inputSchema": empty,
        },
        {
            "name": "usage",
            "description": "Current output tokens per second and today's cost per tool across all machines, plus the month's average tk/s against the monthly goal and the rate the rest of the month needs.",
            "inputSchema": empty,
        },
    ])
}

fn call(name: &str) -> Result<Value, String> {
    let status = crate::advisor::read_status().map_err(|e| format!("{e:#}"))?;
    let now = Utc::now();
    match name {
        "quotas" => Ok(crate::advisor::quotas_answer(&status, now)),
        "where_to_spend" => Ok(crate::advisor::where_to_spend_answer(&status, now)),
        "usage" => Ok(crate::advisor::usage_answer(&status, now)),
        other => Err(format!("unknown tool {other}")),
    }
}

/// The reply to one JSON-RPC request, or `None` for a notification.
pub fn handle(request: &Value) -> Option<Value> {
    let id = request.get("id")?.clone();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": request.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "splitrail", "version": env!("CARGO_PKG_VERSION")},
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            match call(name) {
                Ok(answer) => json!({
                    "content": [{"type": "text", "text": serde_json::to_string_pretty(&answer).unwrap_or_default()}],
                    "structuredContent": answer,
                }),
                Err(message) => {
                    json!({"content": [{"type": "text", "text": message}], "isError": true})
                }
            }
        }
        _ => {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("method not found: {method}")}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Serve requests on stdin until it closes.
pub fn run() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(&request),
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
            ),
        };
        if let Some(reply) = reply {
            let _ = writeln!(stdout, "{reply}");
            let _ = stdout.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaks_the_handshake_and_lists_its_tools() {
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}}))
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "splitrail");
        assert!(
            handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none()
        );
        let list = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["quotas", "where_to_spend", "usage"]);
        let missing =
            handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).unwrap();
        assert_eq!(missing["error"]["code"], -32601);
    }
}
