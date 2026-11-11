//! Streaming client for llama-server's OpenAI-compatible
//! /v1/chat/completions endpoint, including streamed tool-call deltas.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::db::ToolCall;

/// No bytes for this long means llama-server is stuck. Prompt processing of
/// a long context can take minutes before the first token arrives.
const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Content(String),
    Reasoning(String),
    /// A piece of a tool call's arguments as the model writes them, so the
    /// app can show a file being written live.
    ToolArgs {
        index: usize,
        name: String,
        text: String,
    },
}

#[derive(Debug, Default, Clone)]
pub struct Turn {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    /// Tokens in the prompt, as llama-server counted them.
    pub prompt_tokens: Option<u64>,
    /// Tokens the model wrote, and how fast (llama-server's own timings).
    pub gen_tokens: Option<u64>,
    pub gen_per_sec: Option<f64>,
    /// How fast the prompt was read (tokens per second).
    pub prompt_per_sec: Option<f64>,
}

pub enum StreamResult {
    Complete(Turn),
    /// Cancelled mid-stream; holds what was received so far.
    Cancelled(Turn),
}

/// Splits a byte stream into SSE `data:` payloads. Handles events split
/// across chunks, including in the middle of a UTF-8 sequence.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                out.push(data.trim_start().to_string());
            }
        }
        out
    }

    /// A final line without a trailing newline.
    pub fn finish(&mut self) -> Vec<String> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        self.buf.push(b'\n');
        self.feed(&[])
    }
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates `choices[0].delta` objects into a complete turn.
#[derive(Default)]
pub struct TurnBuilder {
    turn: Turn,
    calls: Vec<PartialCall>,
}

impl TurnBuilder {
    /// Applies one parsed chunk. Returns text deltas to forward to clients.
    pub fn apply(&mut self, chunk: &Value) -> Result<Vec<Delta>> {
        if let Some(err) = chunk.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            bail!("llama-server error: {msg}");
        }
        if let Some(n) = prompt_tokens(chunk) {
            self.turn.prompt_tokens = Some(n);
        }
        if let Some(n) = chunk
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
        {
            self.turn.gen_tokens = Some(n);
        }
        if let Some(t) = chunk.get("timings") {
            let num = |k: &str| t.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
            if let Some(n) = t.get("predicted_n").and_then(Value::as_u64) {
                self.turn.gen_tokens = Some(n);
            }
            if let Some(v) = num("predicted_per_second") {
                self.turn.gen_per_sec = Some(v);
            }
            // A fully cached prompt reads 0 tokens; its speed means nothing.
            if t.get("prompt_n").and_then(Value::as_u64).unwrap_or(0) > 0
                && let Some(v) = num("prompt_per_second")
            {
                self.turn.prompt_per_sec = Some(v);
            }
        }
        let mut deltas = Vec::new();
        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return Ok(deltas);
        };
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.turn.finish_reason = Some(fr.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(deltas);
        };
        if let Some(r) = delta.get("reasoning_content").and_then(Value::as_str)
            && !r.is_empty()
        {
            self.turn.reasoning.push_str(r);
            deltas.push(Delta::Reasoning(r.to_string()));
        }
        if let Some(c) = delta.get("content").and_then(Value::as_str)
            && !c.is_empty()
        {
            self.turn.content.push_str(c);
            deltas.push(Delta::Content(c.to_string()));
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
            for tc in tcs {
                let id = tc.get("id").and_then(Value::as_str).unwrap_or("");
                let index = match tc.get("index").and_then(Value::as_u64) {
                    Some(i) => i as usize,
                    None => {
                        // No index: a new id starts a new call, otherwise continue the last.
                        let last_id = self.calls.last().map(|c| c.id.as_str());
                        if self.calls.is_empty() || (!id.is_empty() && last_id != Some(id)) {
                            self.calls.len()
                        } else {
                            self.calls.len() - 1
                        }
                    }
                };
                while self.calls.len() <= index {
                    self.calls.push(PartialCall::default());
                }
                let call = &mut self.calls[index];
                if !id.is_empty() && call.id.is_empty() {
                    call.id = id.to_string();
                }
                if let Some(f) = tc.get("function") {
                    if let Some(n) = f.get("name").and_then(Value::as_str) {
                        call.name.push_str(n);
                    }
                    let piece = match f.get("arguments") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(other) => other.to_string(),
                    };
                    call.arguments.push_str(&piece);
                    if !piece.is_empty() && !call.name.is_empty() {
                        deltas.push(Delta::ToolArgs {
                            index,
                            name: call.name.clone(),
                            text: piece,
                        });
                    }
                }
            }
        }
        Ok(deltas)
    }

    pub fn finish(mut self) -> Turn {
        self.turn.tool_calls = self
            .calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| ToolCall {
                id: if c.id.is_empty() {
                    format!("call_{}", uuid::Uuid::new_v4().simple())
                } else {
                    c.id
                },
                name: c.name,
                arguments: if c.arguments.trim().is_empty() {
                    "{}".into()
                } else {
                    c.arguments
                },
            })
            .collect();
        self.turn
    }

    fn snapshot(&self) -> Turn {
        Turn {
            content: self.turn.content.clone(),
            reasoning: self.turn.reasoning.clone(),
            ..Default::default()
        }
    }
}

/// The prompt size from a chunk's `usage` (OpenAI style) or `timings`
/// (llama-server's own, where cached tokens are counted apart).
fn prompt_tokens(chunk: &Value) -> Option<u64> {
    if let Some(n) = chunk
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64)
    {
        return Some(n);
    }
    let t = chunk.get("timings")?;
    let n = t.get("prompt_n").and_then(Value::as_u64)?;
    Some(n + t.get("cache_n").and_then(Value::as_u64).unwrap_or(0))
}

/// The prompt size from llama-server's "context too long" error, if that's
/// what this is: "request (66986 tokens) exceeds the available context size
/// (65536 tokens)".
pub fn context_overflow(err: &str) -> Option<(u64, Option<u64>)> {
    let lower = err.to_lowercase();
    if !(lower.contains("exceed") && lower.contains("context")) {
        return None;
    }
    // The numbers written just before "tokens".
    let mut numbers = Vec::new();
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("token") {
        let before = rest[..i].trim_end();
        let digits: String = before
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(n) = digits.parse::<u64>() {
            numbers.push(n);
        }
        rest = &rest[i + 5..];
    }
    Some((
        numbers.first().copied().unwrap_or(0),
        numbers.get(1).copied(),
    ))
}

pub async fn stream_chat(
    http: &reqwest::Client,
    url: &str,
    body: &Value,
    cancel: &CancellationToken,
    mut on_delta: impl FnMut(Delta),
) -> Result<StreamResult> {
    let req = http.post(url).json(body).send();
    let mut resp = tokio::select! {
        r = req => r.map_err(|e| anyhow!("could not reach llama-server: {e}"))?,
        _ = cancel.cancelled() => return Ok(StreamResult::Cancelled(Turn::default())),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or(text);
        bail!("llama-server returned {status}: {msg}");
    }

    let mut parser = SseParser::default();
    let mut builder = TurnBuilder::default();
    // For servers that send no timings: time from the first token to the end.
    let mut first_token: Option<std::time::Instant> = None;
    let mut deltas_seen: u64 = 0;
    loop {
        let chunk = tokio::select! {
            c = tokio::time::timeout(IDLE_TIMEOUT, resp.chunk()) => match c {
                Ok(c) => c.map_err(|e| anyhow!("stream from llama-server broke: {e}"))?,
                Err(_) => bail!("llama-server sent nothing for {} minutes", IDLE_TIMEOUT.as_secs() / 60),
            },
            _ = cancel.cancelled() => return Ok(StreamResult::Cancelled(builder.snapshot())),
        };
        let eof = chunk.is_none();
        let payloads = match chunk {
            Some(bytes) => parser.feed(&bytes),
            None => parser.finish(),
        };
        let mut done = false;
        for p in payloads {
            if p == "[DONE]" {
                done = true;
                break;
            }
            if p.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&p)
                .map_err(|e| anyhow!("bad chunk from llama-server ({e}): {p}"))?;
            for d in builder.apply(&v)? {
                first_token.get_or_insert_with(std::time::Instant::now);
                deltas_seen += 1;
                on_delta(d);
            }
        }
        if done || eof {
            break;
        }
    }
    let mut turn = builder.finish();
    if turn.gen_per_sec.is_none()
        && let Some(start) = first_token
    {
        // Each streamed delta is about one token.
        let n = turn.gen_tokens.unwrap_or(deltas_seen);
        let secs = start.elapsed().as_secs_f64();
        if n > 1 && secs > 0.05 {
            turn.gen_per_sec = Some((n - 1) as f64 / secs);
            turn.gen_tokens.get_or_insert(n);
        }
    }
    Ok(StreamResult::Complete(turn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sse_split_across_chunks() {
        let mut p = SseParser::default();
        assert!(p.feed(b"data: {\"a\"").is_empty());
        assert_eq!(
            p.feed(b":1}\n\ndata: [DONE]\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
        // UTF-8 split in the middle of a character.
        let s = "data: é\n".as_bytes();
        assert!(p.feed(&s[..7]).is_empty());
        assert_eq!(p.feed(&s[7..]), vec!["é"]);
    }

    #[test]
    fn builds_streamed_tool_calls() {
        let mut b = TurnBuilder::default();
        let chunks = [
            json!({"choices":[{"delta":{"reasoning_content":"think"}}]}),
            json!({"choices":[{"delta":{"content":"I'll write it."}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"write_file","arguments":""}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"hello.txt\","}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"content\":\"hi\"}"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","function":{"name":"list_dir","arguments":"{}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        ];
        let mut deltas = Vec::new();
        for c in &chunks {
            deltas.extend(b.apply(c).unwrap());
        }
        let t = b.finish();
        assert_eq!(
            deltas[..2],
            [
                Delta::Reasoning("think".into()),
                Delta::Content("I'll write it.".into())
            ]
        );
        let args: String = deltas
            .iter()
            .filter_map(|d| match d {
                Delta::ToolArgs {
                    index: 0,
                    name,
                    text,
                } if name == "write_file" => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(args, "{\"path\":\"hello.txt\",\"content\":\"hi\"}");
        assert_eq!(t.tool_calls.len(), 2);
        assert_eq!(t.tool_calls[0].name, "write_file");
        assert_eq!(
            t.tool_calls[0].arguments,
            "{\"path\":\"hello.txt\",\"content\":\"hi\"}"
        );
        assert_eq!(t.tool_calls[1].id, "c2");
        assert_eq!(t.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn reads_prompt_size() {
        let mut b = TurnBuilder::default();
        b.apply(&json!({"choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":5}}))
            .unwrap();
        assert_eq!(b.finish().prompt_tokens, Some(1200));
        let mut b = TurnBuilder::default();
        b.apply(&json!({"choices":[{"delta":{},"finish_reason":"stop"}],"timings":{"prompt_n":200,"cache_n":1000}}))
            .unwrap();
        assert_eq!(b.finish().prompt_tokens, Some(1200));
    }

    #[test]
    fn reads_speed() {
        let mut b = TurnBuilder::default();
        b.apply(&json!({"choices":[{"delta":{},"finish_reason":"stop"}],"timings":{"prompt_n":200,"cache_n":0,"prompt_per_second":900.5,"predicted_n":120,"predicted_per_second":42.25}}))
            .unwrap();
        let t = b.finish();
        assert_eq!(t.gen_tokens, Some(120));
        assert_eq!(t.gen_per_sec, Some(42.25));
        assert_eq!(t.prompt_per_sec, Some(900.5));
        // A fully cached prompt has no reading speed.
        let mut b = TurnBuilder::default();
        b.apply(&json!({"choices":[],"timings":{"prompt_n":0,"prompt_per_second":1e9,"predicted_n":3,"predicted_per_second":30.0}}))
            .unwrap();
        assert_eq!(b.finish().prompt_per_sec, None);
    }

    #[test]
    fn spots_context_overflow() {
        assert_eq!(
            context_overflow(
                "llama-server returned 400 Bad Request: request (66986 tokens) exceeds the available context size (65536 tokens), try increasing it"
            ),
            Some((66986, Some(65536)))
        );
        assert_eq!(
            context_overflow("llama-server returned 500: out of memory"),
            None
        );
    }

    #[test]
    fn error_chunk_is_reported() {
        let mut b = TurnBuilder::default();
        let e = b
            .apply(&json!({"error":{"message":"context too long"}}))
            .unwrap_err();
        assert!(e.to_string().contains("context too long"));
    }
}
