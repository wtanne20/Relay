//! A small MCP client: JSON-RPC 2.0 over a child process's stdin/stdout
//! (one message per line) or over Streamable HTTP (each message a POST, the
//! answer plain JSON or an event stream).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, oneshot};
use tokio_util::sync::CancellationToken;

use super::McpServerConfig;

/// The protocol version Relay asks for. Servers answer with the one they
/// speak; Relay only uses what every version since 2024-11-05 has.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// How long a server gets to start and answer `initialize`. `npx` and `uvx`
/// download the package the first time, which can take a while.
pub const START_TIMEOUT: Duration = Duration::from_secs(90);
const STDERR_KEEP: usize = 30;

/// One tool a server offers, as it described it.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteTool {
    pub name: String,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// The server says the tool doesn't change anything.
    pub read_only: bool,
    /// The server says the tool may delete or overwrite things.
    pub destructive: bool,
}

/// What a tool call gave back, ready for the model.
#[derive(Debug, Clone)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

enum Conn {
    Stdio(StdioConn),
    Http(HttpConn),
}

pub struct Client {
    conn: Conn,
    next_id: AtomicU64,
    /// The server's name and version from `initialize`.
    pub server_info: Value,
    /// Hints for the model the server sent with `initialize`, if any.
    pub instructions: Option<String>,
    /// Signalled when the server says its tool list changed.
    pub tools_changed: Arc<Notify>,
}

impl Client {
    /// Starts (or reaches) the server and does the `initialize` handshake.
    pub async fn connect(
        cfg: &McpServerConfig,
        data_dir: &Path,
        http: &reqwest::Client,
    ) -> Result<Self> {
        let tools_changed = Arc::new(Notify::new());
        let conn = match cfg.transport() {
            super::Transport::Stdio => {
                Conn::Stdio(StdioConn::spawn(cfg, data_dir, tools_changed.clone())?)
            }
            super::Transport::Http => Conn::Http(HttpConn::new(cfg, http.clone())?),
        };
        let mut client = Self {
            conn,
            next_id: AtomicU64::new(1),
            server_info: Value::Null,
            instructions: None,
            tools_changed,
        };
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "Relay", "version": env!("CARGO_PKG_VERSION") },
            }),
            START_TIMEOUT,
            None,
        );
        let res = match init.await {
            Ok(v) => v,
            Err(e) => {
                let tail = client.stderr_tail();
                client.close().await;
                return Err(if tail.is_empty() {
                    e
                } else {
                    e.context(format!("the server printed:\n{tail}"))
                });
            }
        };
        if let Conn::Http(h) = &client.conn {
            let v = res
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            *lock(&h.protocol) = Some(v.to_string());
        }
        client.server_info = res.get("serverInfo").cloned().unwrap_or(Value::Null);
        client.instructions = res
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty());
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    /// Every tool the server offers (following pages).
    pub async fn list_tools(&self) -> Result<Vec<RemoteTool>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let res = self
                .request("tools/list", params, Duration::from_secs(30), None)
                .await?;
            for t in res
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = t.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let hint = |k: &str| {
                    t.pointer(&format!("/annotations/{k}"))
                        .and_then(Value::as_bool)
                };
                let mut schema = t.get("inputSchema").cloned().unwrap_or_else(|| json!({}));
                if !schema.is_object() {
                    schema = json!({});
                }
                if schema.get("type").is_none() {
                    schema["type"] = json!("object");
                }
                if schema.get("properties").is_none() {
                    schema["properties"] = json!({});
                }
                let read_only = hint("readOnlyHint").unwrap_or(false);
                out.push(RemoteTool {
                    name: name.to_string(),
                    title: t
                        .get("title")
                        .or_else(|| t.pointer("/annotations/title"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    input_schema: schema,
                    read_only,
                    destructive: !read_only && hint("destructiveHint").unwrap_or(true),
                });
            }
            cursor = res
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Calls one tool. Errors from the server's side of the call (bad
    /// arguments, a failed action) come back as a result with `is_error`.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<CallResult> {
        let res = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                timeout,
                Some(cancel),
            )
            .await?;
        Ok(format_result(&res))
    }

    /// Whether the connection is still usable (the process is running).
    pub fn alive(&self) -> bool {
        match &self.conn {
            Conn::Stdio(s) => !s.closed.load(Ordering::SeqCst),
            Conn::Http(_) => true,
        }
    }

    /// The last lines the server wrote to stderr (stdio servers only).
    pub fn stderr_tail(&self) -> String {
        match &self.conn {
            Conn::Stdio(s) => lock(&s.stderr)
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
            Conn::Http(_) => String::new(),
        }
    }

    /// Stops the server (stdio) or ends the session (HTTP).
    pub async fn close(&self) {
        match &self.conn {
            Conn::Stdio(s) => s.close().await,
            Conn::Http(h) => h.close().await,
        }
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let never = CancellationToken::new();
        let cancel = cancel.unwrap_or(&never);
        let fut = async {
            match &self.conn {
                Conn::Stdio(s) => s.request(id, &msg).await,
                Conn::Http(h) => h.request(id, &msg).await,
            }
        };
        let outcome = tokio::select! {
            r = tokio::time::timeout(timeout, fut) => match r {
                Ok(r) => r,
                Err(_) => Err(anyhow!("the server didn't answer within {} seconds", timeout.as_secs())),
            },
            _ = cancel.cancelled() => Err(anyhow!("cancelled by user")),
        };
        if outcome.is_err() {
            if let Conn::Stdio(s) = &self.conn {
                lock(&s.pending).remove(&id);
            }
            if method != "initialize" {
                let _ = self
                    .notify(
                        "notifications/cancelled",
                        json!({ "requestId": id, "reason": "timed out or stopped" }),
                    )
                    .await;
            }
        }
        outcome
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &self.conn {
            Conn::Stdio(s) => s.send(&msg).await,
            Conn::Http(h) => h.notify(&msg).await,
        }
    }
}

/// Turns a `tools/call` result into text for the model.
pub fn format_result(res: &Value) -> CallResult {
    let mut parts: Vec<String> = Vec::new();
    for c in res
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match c.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(t) = c.get("text").and_then(Value::as_str) {
                    parts.push(t.to_string());
                }
            }
            "image" | "audio" => {
                let mime = c.get("mimeType").and_then(Value::as_str).unwrap_or("?");
                parts.push(format!(
                    "[{} ({mime}) not shown]",
                    c["type"].as_str().unwrap_or("media")
                ));
            }
            "resource" => {
                let r = &c["resource"];
                if let Some(t) = r.get("text").and_then(Value::as_str) {
                    parts.push(t.to_string());
                } else if let Some(u) = r.get("uri").and_then(Value::as_str) {
                    parts.push(format!("[resource {u}]"));
                }
            }
            "resource_link" => {
                let u = c.get("uri").and_then(Value::as_str).unwrap_or("");
                let n = c.get("name").and_then(Value::as_str).unwrap_or(u);
                parts.push(format!("[link: {n} {u}]"));
            }
            _ => {}
        }
    }
    if parts.is_empty()
        && let Some(s) = res.get("structuredContent")
    {
        parts.push(s.to_string());
    }
    let mut text = parts.join("\n");
    if text.trim().is_empty() {
        text = "(the tool returned nothing)".into();
    }
    CallResult {
        text: crate::tools::truncate_middle(&text, super::MAX_RESULT_CHARS),
        is_error: res.get("isError").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// A JSON-RPC message that answers request `id`: its result, or its error.
fn response_for(msg: &Value) -> Option<(u64, Result<Value>)> {
    let id = msg.get("id")?.as_u64()?;
    if msg.get("method").is_some() {
        return None;
    }
    if let Some(e) = msg.get("error") {
        let text = e
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        let code = e.get("code").and_then(Value::as_i64).unwrap_or(0);
        return Some((
            id,
            Err(anyhow!(
                "the server answered with an error ({code}): {text}"
            )),
        ));
    }
    Some((id, Ok(msg.get("result").cloned().unwrap_or(Value::Null))))
}

/// Relay's answer to a request the server sends it (it offers no client
/// features, but must answer pings).
fn answer_server_request(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method")?.as_str()?;
    Some(match method {
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        "roots/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "roots": [] } }),
        _ => json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("Relay doesn't support {method}") }
        }),
    })
}

// ---------- stdio ----------

struct StdioConn {
    stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
    child: tokio::sync::Mutex<tokio::process::Child>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

/// The program to start. On Windows `npx`, `uvx` and many other launchers
/// are `.cmd` files that only a shell finds, so Relay looks them up itself.
pub fn resolve_command(cmd: &str) -> PathBuf {
    let p = PathBuf::from(cmd);
    if !cfg!(windows) || p.components().count() > 1 || p.extension().is_some() {
        return p;
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{cmd}{}", ext.to_ascii_lowercase()));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    p
}

impl StdioConn {
    fn spawn(cfg: &McpServerConfig, data_dir: &Path, tools_changed: Arc<Notify>) -> Result<Self> {
        let program = resolve_command(&cfg.command);
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let cwd = if cfg.cwd.is_empty() {
            data_dir.to_path_buf()
        } else {
            PathBuf::from(&cfg.cwd)
        };
        if cwd.is_dir() {
            cmd.current_dir(cwd);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().with_context(|| {
            format!(
                "could not start '{}'. Is it installed and on the PATH?",
                cfg.command
            )
        })?;
        crate::process_guard::tie_to_relay(&child);
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let stderr_pipe = child.stderr.take().context("no stderr")?;
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let stderr: Arc<Mutex<VecDeque<String>>> = Arc::default();
        let name = cfg.name.clone();

        {
            let stderr = stderr.clone();
            let name = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr_pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("mcp {name}: {line}");
                    let mut q = lock(&stderr);
                    if q.len() == STDERR_KEEP {
                        q.pop_front();
                    }
                    q.push_back(line.chars().take(500).collect());
                }
            });
        }
        {
            let pending = pending.clone();
            let closed = closed.clone();
            let stdin = stdin.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    let line = match lines.next_line().await {
                        Ok(Some(l)) => l,
                        Ok(None) | Err(_) => break,
                    };
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(line) else {
                        tracing::debug!("mcp {name}: not JSON on stdout: {line}");
                        continue;
                    };
                    let msgs = match msg {
                        Value::Array(a) => a,
                        m => vec![m],
                    };
                    for msg in msgs {
                        if let Some((id, r)) = response_for(&msg) {
                            if let Some(tx) = lock(&pending).remove(&id) {
                                let _ = tx.send(r);
                            }
                        } else if let Some(answer) = answer_server_request(&msg) {
                            let mut w = stdin.lock().await;
                            let mut s = answer.to_string();
                            s.push('\n');
                            let _ = w.write_all(s.as_bytes()).await;
                            let _ = w.flush().await;
                        } else if msg.get("method").and_then(Value::as_str)
                            == Some("notifications/tools/list_changed")
                        {
                            tools_changed.notify_one();
                        }
                    }
                }
                closed.store(true, Ordering::SeqCst);
                for (_, tx) in lock(&pending).drain() {
                    let _ = tx.send(Err(anyhow!("the server stopped")));
                }
                tracing::info!("mcp {name}: server exited");
            });
        }
        Ok(Self {
            stdin,
            child: tokio::sync::Mutex::new(child),
            pending,
            closed,
            stderr,
        })
    }

    async fn send(&self, msg: &Value) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            bail!("the server stopped");
        }
        let mut s = msg.to_string();
        s.push('\n');
        let mut w = self.stdin.lock().await;
        w.write_all(s.as_bytes())
            .await
            .context("could not write to the server")?;
        w.flush().await?;
        Ok(())
    }

    async fn request(&self, id: u64, msg: &Value) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        if let Err(e) = self.send(msg).await {
            lock(&self.pending).remove(&id);
            return Err(e);
        }
        rx.await.map_err(|_| anyhow!("the server stopped"))?
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        // Closing stdin asks the server to exit; then it's killed.
        {
            let mut w = self.stdin.lock().await;
            let _ = w.shutdown().await;
        }
        let mut child = self.child.lock().await;
        if tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }
}

// ---------- Streamable HTTP ----------

struct HttpConn {
    http: reqwest::Client,
    url: String,
    headers: reqwest::header::HeaderMap,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
}

impl HttpConn {
    fn new(cfg: &McpServerConfig, http: reqwest::Client) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in &cfg.headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .with_context(|| format!("bad header name '{k}'"))?;
            let value = reqwest::header::HeaderValue::from_str(v)
                .with_context(|| format!("bad value for header '{k}'"))?;
            headers.insert(name, value);
        }
        Ok(Self {
            http,
            url: cfg.url.clone(),
            headers,
            session: Mutex::new(None),
            protocol: Mutex::new(None),
        })
    }

    fn post(&self, msg: &Value) -> reqwest::RequestBuilder {
        let mut rb = self
            .http
            .post(&self.url)
            .headers(self.headers.clone())
            .header("Accept", "application/json, text/event-stream")
            .json(msg);
        if let Some(s) = lock(&self.session).clone() {
            rb = rb.header("Mcp-Session-Id", s);
        }
        if let Some(v) = lock(&self.protocol).clone() {
            rb = rb.header("MCP-Protocol-Version", v);
        }
        rb
    }

    async fn send(&self, msg: &Value) -> Result<reqwest::Response> {
        let resp = self
            .post(msg)
            .send()
            .await
            .with_context(|| format!("could not reach {}", self.url))?;
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *lock(&self.session) = Some(s.to_string());
        }
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let body: String = body.chars().take(300).collect();
            let hint = match status.as_u16() {
                401 | 403 => " (check the server's sign-in token in its headers)",
                404 if lock(&self.session).is_some() => " (the session ended; restart the server)",
                405 => {
                    " (this address may be an older SSE-only server, which Relay doesn't support yet)"
                }
                _ => "",
            };
            bail!("the server answered {status}{hint}: {body}");
        }
        Ok(resp)
    }

    async fn notify(&self, msg: &Value) -> Result<()> {
        self.send(msg).await.map(|_| ())
    }

    async fn request(&self, id: u64, msg: &Value) -> Result<Value> {
        let resp = self.send(msg).await?;
        let is_sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !is_sse {
            let v: Value = resp
                .json()
                .await
                .context("the server's answer isn't JSON")?;
            let msgs = match v {
                Value::Array(a) => a,
                m => vec![m],
            };
            return msgs
                .iter()
                .filter_map(response_for)
                .find(|(i, _)| *i == id)
                .map(|(_, r)| r)
                .unwrap_or_else(|| Err(anyhow!("the server's answer has no result")));
        }
        // An event stream: read events until the one answering this request.
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut data = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("the server's stream broke off")?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    if !data.is_empty() {
                        if let Some(r) = self.handle_event(id, &data).await {
                            return r;
                        }
                        data.clear();
                    }
                } else if let Some(d) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(d.strip_prefix(' ').unwrap_or(d));
                }
            }
        }
        if !data.is_empty()
            && let Some(r) = self.handle_event(id, &data).await
        {
            return r;
        }
        bail!("the server ended its stream without an answer")
    }

    async fn handle_event(&self, id: u64, data: &str) -> Option<Result<Value>> {
        let msg: Value = serde_json::from_str(data).ok()?;
        let msgs = match msg {
            Value::Array(a) => a,
            m => vec![m],
        };
        for m in msgs {
            if let Some((i, r)) = response_for(&m) {
                if i == id {
                    return Some(r);
                }
            } else if let Some(answer) = answer_server_request(&m) {
                let _ = self.send(&answer).await;
            }
        }
        None
    }

    async fn close(&self) {
        let Some(s) = lock(&self.session).take() else {
            return;
        };
        let _ = self
            .http
            .delete(&self.url)
            .headers(self.headers.clone())
            .header("Mcp-Session-Id", s)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_results() {
        let r = format_result(&json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image", "data": "..", "mimeType": "image/png"},
                {"type": "resource", "resource": {"uri": "file:///a", "text": "body"}}
            ],
            "isError": false
        }));
        assert_eq!(r.text, "hello\n[image (image/png) not shown]\nbody");
        assert!(!r.is_error);
        let r =
            format_result(&json!({"content": [], "structuredContent": {"a": 1}, "isError": true}));
        assert_eq!(r.text, r#"{"a":1}"#);
        assert!(r.is_error);
        assert!(format_result(&json!({})).text.contains("nothing"));
    }

    #[test]
    fn responses_and_server_requests() {
        let (id, r) =
            response_for(&json!({"jsonrpc": "2.0", "id": 3, "result": {"x": 1}})).unwrap();
        assert_eq!(id, 3);
        assert_eq!(r.unwrap()["x"], 1);
        let (_, r) =
            response_for(&json!({"id": 4, "error": {"code": -32602, "message": "bad"}})).unwrap();
        assert!(r.unwrap_err().to_string().contains("bad"));
        assert!(response_for(&json!({"id": 5, "method": "ping"})).is_none());
        let a = answer_server_request(&json!({"id": 5, "method": "ping"})).unwrap();
        assert_eq!(a["result"], json!({}));
        let a =
            answer_server_request(&json!({"id": 6, "method": "sampling/createMessage"})).unwrap();
        assert_eq!(a["error"]["code"], -32601);
    }
}
