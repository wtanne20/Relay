//! A scripted stand-in for llama-server, used by the integration tests and by
//! `cargo run --example mock_llama` to try Relay without a GPU.
//!
//! It implements GET /health and a streaming POST /v1/chat/completions that
//! emits OpenAI-style SSE chunks, including fragmented tool-call deltas.
//!
//! Behaviour, based on the last message:
//! - tool result  -> a short final answer
//! - user text mentioning "hello.txt" -> write_file(hello.txt, "hi")
//! - user text starting with "run:"   -> run_command(<rest>)
//! - user text containing "slow"      -> streams text slowly (for stop tests)
//! - user text containing "write code" -> slowly writes a longer app.py
//! - user text containing "edit code"  -> slowly edits app.py
//! - anything else                    -> echoes the prompt
//! - a non-streaming request          -> a chat title, or a summary when
//!   Relay asks for one
//!
//! Streams end with a usage chunk counting a token per 3 bytes of messages,
//! and with `ctx_tokens` set, longer requests are refused the way
//! llama-server refuses them.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct MockOptions {
    /// /health returns 503 for this long after start (simulates model load).
    pub load_ms: u64,
    /// Refuse prompts longer than this many tokens (0 = no limit).
    pub ctx_tokens: u64,
}

struct MockState {
    started: Instant,
    load: Duration,
    requests: AtomicU64,
    ctx: u64,
}

pub fn router(opts: MockOptions) -> Router {
    let state = Arc::new(MockState {
        started: Instant::now(),
        load: Duration::from_millis(opts.load_ms),
        requests: AtomicU64::new(0),
        ctx: opts.ctx_tokens,
    });
    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat))
        .with_state(state)
}

pub async fn serve(listener: tokio::net::TcpListener, opts: MockOptions) {
    let _ = axum::serve(listener, router(opts)).await;
}

async fn health(State(s): State<Arc<MockState>>) -> Response {
    if s.started.elapsed() < s.load {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"error": {"message": "Loading model"}})),
        )
            .into_response();
    }
    axum::Json(json!({"status": "ok"})).into_response()
}

fn chunk(delta: Value, finish: Option<&str>) -> String {
    let v = json!({
        "id": "chatcmpl-mock",
        "object": "chat.completion.chunk",
        "model": "mock",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    });
    format!("data: {v}\n\n")
}

fn tool_call_chunks(id: &str, name: &str, args: &Value) -> Vec<String> {
    let args = args.to_string();
    let mid = args.len() / 2;
    let mid = (mid..args.len())
        .find(|i| args.is_char_boundary(*i))
        .unwrap_or(args.len());
    vec![
        chunk(json!({"content": format!("Calling {name}.")}), None),
        chunk(
            json!({"tool_calls": [{"index": 0, "id": id, "type": "function", "function": {"name": name, "arguments": ""}}]}),
            None,
        ),
        chunk(
            json!({"tool_calls": [{"index": 0, "function": {"arguments": &args[..mid]}}]}),
            None,
        ),
        chunk(
            json!({"tool_calls": [{"index": 0, "function": {"arguments": &args[mid..]}}]}),
            None,
        ),
        chunk(json!({}), Some("tool_calls")),
    ]
}

fn text_chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split_inclusive(' ')
        .map(|w| chunk(json!({"content": w}), None))
        .collect();
    out.push(chunk(json!({}), Some("stop")));
    out
}

async fn chat(State(s): State<Arc<MockState>>, axum::Json(body): axum::Json<Value>) -> Response {
    let n = s.requests.fetch_add(1, Ordering::SeqCst);
    let tokens = serde_json::to_string(&body["messages"])
        .map(|t| t.len())
        .unwrap_or(0) as u64
        / 3;
    if s.ctx > 0 && tokens > s.ctx {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": {"code": 400, "message": format!("request ({tokens} tokens) exceeds the available context size ({} tokens), try increasing it", s.ctx), "type": "exceed_context_size_error"}})),
        )
            .into_response();
    }
    let system = body["messages"][0]["content"].as_str().unwrap_or("");
    if system.starts_with("You keep the running summary") {
        let input = body["messages"][1]["content"].as_str().unwrap_or("");
        let users = input.matches("USER:").count();
        return axum::Json(json!({
            "choices": [{"message": {"role": "assistant", "content": format!("## Goal\nMock summary covering {users} user messages.")}}]
        }))
        .into_response();
    }
    if body.get("stream").and_then(Value::as_bool) != Some(true) {
        // Relay's only non-streaming request is the chat title.
        return axum::Json(json!({
            "choices": [{"message": {"role": "assistant", "content": "\"Mock chat title.\""}}]
        }))
        .into_response();
    }
    let system = body["messages"][0]["content"].as_str().unwrap_or("");
    // Plain chats send no tools; agent chats must.
    if !system.contains("This is a plain chat")
        && body
            .get("tools")
            .and_then(Value::as_array)
            .is_none_or(|t| t.is_empty())
    {
        return (StatusCode::BAD_REQUEST, "expected tool definitions").into_response();
    }
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let last = messages.last().cloned().unwrap_or(Value::Null);
    let role = last.get("role").and_then(Value::as_str).unwrap_or("");
    // Pictures come as content parts; the mock says how many it got.
    let pictures = last
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|p| {
                    p.pointer("/image_url/url")
                        .and_then(Value::as_str)
                        .is_some_and(|u| u.starts_with("data:image/"))
                })
                .count()
        })
        .unwrap_or(0);
    let content = match last.get("content") {
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        Some(v) => v.as_str().unwrap_or("").to_string(),
        None => String::new(),
    };
    let content = if pictures > 0 {
        format!("[{pictures} picture(s)] {content}")
    } else {
        content
    };

    let mut delay = Duration::from_millis(5);
    let mut chunks = vec![chunk(json!({"role": "assistant", "content": null}), None)];
    if role == "tool" {
        let first = content.lines().next().unwrap_or("").to_string();
        chunks.push(chunk(
            json!({"reasoning_content": "The tool finished."}),
            None,
        ));
        chunks.extend(text_chunks(&format!("Done. Tool said: {first}")));
    } else {
        let lower = content.to_lowercase();
        if lower.contains("hello.txt") {
            chunks.push(chunk(
                json!({"reasoning_content": "I should create the file."}),
                None,
            ));
            chunks.extend(tool_call_chunks(
                &format!("call_{n}"),
                "write_file",
                &json!({"path": "hello.txt", "content": "hi"}),
            ));
        } else if let Some(q) = content.strip_prefix("docs:") {
            chunks.extend(tool_call_chunks(
                &format!("call_{n}"),
                "search_documents",
                &json!({"query": q.trim()}),
            ));
        } else if let Some(cmd) = content.strip_prefix("run:") {
            chunks.extend(tool_call_chunks(
                &format!("call_{n}"),
                "run_command",
                &json!({"command": cmd.trim()}),
            ));
        } else if lower.contains("write code") || lower.contains("edit code") {
            delay = Duration::from_millis(30);
            let (name, args) = if lower.contains("edit code") {
                (
                    "edit_file",
                    json!({"path": "app.py", "old_text": "def greet(name):\n    return f\"Hello, {name}!\"\n", "new_text": "def greet(name, excited=False):\n    mark = \"!\" if excited else \".\"\n    return f\"Hello, {name}{mark}\"\n"}),
                )
            } else {
                let body: String = (1..=40)
                    .map(|i| format!("def step_{i}(x):\n    \"\"\"Step {i} of the pipeline.\"\"\"\n    return x + {i}\n\n"))
                    .collect();
                (
                    "write_file",
                    json!({"path": "app.py", "content": format!("def greet(name):\n    return f\"Hello, {{name}}!\"\n\n{body}")}),
                )
            };
            chunks.push(chunk(json!({"content": "Writing the code now."}), None));
            chunks.push(chunk(
                json!({"tool_calls": [{"index": 0, "id": format!("call_{n}"), "type": "function", "function": {"name": name, "arguments": ""}}]}),
                None,
            ));
            let text = args.to_string();
            let mut piece = String::new();
            for c in text.chars() {
                piece.push(c);
                if piece.len() >= 12 {
                    chunks.push(chunk(json!({"tool_calls": [{"index": 0, "function": {"arguments": std::mem::take(&mut piece)}}]}), None));
                }
            }
            if !piece.is_empty() {
                chunks.push(chunk(
                    json!({"tool_calls": [{"index": 0, "function": {"arguments": piece}}]}),
                    None,
                ));
            }
            chunks.push(chunk(json!({}), Some("tool_calls")));
        } else if lower.contains("slow") {
            delay = Duration::from_millis(200);
            chunks.extend(text_chunks(&"word ".repeat(200)));
        } else {
            chunks.push(chunk(json!({"reasoning_content": "Echo."}), None));
            chunks.extend(text_chunks(&format!("You said: {content}")));
        }
    }
    chunks.push(format!(
        "data: {}\n\n",
        json!({"choices": [], "usage": {"prompt_tokens": tokens, "completion_tokens": 10}})
    ));
    chunks.push("data: [DONE]\n\n".to_string());

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(4);
    tokio::spawn(async move {
        for c in chunks {
            tokio::time::sleep(delay).await;
            if tx.send(Ok(c)).await.is_err() {
                return; // client went away (e.g. task stopped)
            }
        }
    });
    let stream = ReceiverStream(rx);
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Minimal mpsc -> Stream adapter (avoids a dependency on tokio-stream).
struct ReceiverStream(tokio::sync::mpsc::Receiver<Result<String, std::io::Error>>);

impl futures_core::Stream for ReceiverStream {
    type Item = Result<String, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
