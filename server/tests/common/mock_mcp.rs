//! A tiny MCP server for tests, over stdio (run by re-starting the test
//! binary, see `mock_mcp_stdio_server` in integration.rs) or Streamable
//! HTTP (an axum router).
#![allow(dead_code)]

use std::io::{BufRead, Write};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};

/// The answer to one JSON-RPC message (None for notifications).
pub fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method")?.as_str()?;
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": { "listChanged": true } },
            "serverInfo": { "name": "mock-mcp", "version": "1.0.0" },
            "instructions": "Mock server for tests."
        }),
        "tools/list" => {
            // Two pages, to check Relay follows the cursor.
            if msg.pointer("/params/cursor").is_none() {
                json!({
                    "tools": [{
                        "name": "echo",
                        "description": "Repeat text back.",
                        "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]},
                        "annotations": {"readOnlyHint": true}
                    }],
                    "nextCursor": "page2"
                })
            } else {
                json!({
                    "tools": [
                        {
                            "name": "add_note",
                            "description": "Save a note.",
                            "inputSchema": {"type": "object", "properties": {"note": {"type": "string"}}}
                        },
                        {
                            "name": "fail",
                            "description": "Always fails.",
                            "inputSchema": {"type": "object"}
                        }
                    ]
                })
            }
        }
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            match name {
                "echo" => json!({
                    "content": [{"type": "text", "text": format!("echo: {}", args["text"].as_str().unwrap_or(""))}]
                }),
                "add_note" => json!({
                    "content": [{"type": "text", "text": format!("saved note: {}", args["note"].as_str().unwrap_or(""))}]
                }),
                "fail" => {
                    json!({ "content": [{"type": "text", "text": "it broke"}], "isError": true })
                }
                _ => {
                    return Some(json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": {"code": -32602, "message": format!("unknown tool {name}")}
                    }));
                }
            }
        }
        "ping" => json!({}),
        _ => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "method not found"}
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Serves MCP on stdin/stdout until stdin closes. Writes straight to the
/// process's stdout (not through the test harness's captured print!).
pub fn serve_stdio() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(answer) = handle(&msg) {
            let _ = writeln!(out, "{answer}");
            let _ = out.flush();
        }
    }
}

/// Streamable HTTP at `/mcp`. Needs `Authorization: Bearer secret`.
/// Tool calls are answered as an event stream, everything else as JSON.
pub fn router() -> Router {
    Router::new().route("/mcp", post(http_handler))
}

async fn http_handler(headers: HeaderMap, body: String) -> Response {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer secret") {
        return (StatusCode::UNAUTHORIZED, "no").into_response();
    }
    let Ok(msg) = serde_json::from_str::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad json").into_response();
    };
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if method != "initialize"
        && headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) != Some("sess-1")
    {
        return (StatusCode::BAD_REQUEST, "no session").into_response();
    }
    let Some(answer) = handle(&msg) else {
        return StatusCode::ACCEPTED.into_response();
    };
    if method == "tools/call" {
        // A ping from the server first, then the answer.
        let sse = format!(
            "event: message\ndata: {}\n\nevent: message\ndata: {answer}\n\n",
            json!({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"})
        );
        return ([("content-type", "text/event-stream")], sse).into_response();
    }
    let mut resp = axum::Json(answer).into_response();
    if method == "initialize" {
        resp.headers_mut()
            .insert("mcp-session-id", "sess-1".parse().expect("header"));
    }
    resp
}
