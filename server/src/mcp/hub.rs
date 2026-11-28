//! Keeps one connection per enabled MCP server: starts them when Relay
//! starts or the settings change, remembers their tools, reconnects a server
//! that stopped, and runs tool calls on them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::client::{Client, RemoteTool};
use super::{McpServerConfig, exposed_name};
use crate::tools::ToolOutput;

/// How long a new task waits for servers that are still starting.
const READY_WAIT: Duration = Duration::from_secs(15);
/// A server that failed to start is tried again by a new task after this.
const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    /// Turned off in settings.
    Off,
    Starting,
    Ready,
    /// Couldn't start, or stopped; see `error`.
    Error,
}

/// One of a server's tools, for the app and the model.
#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    /// The server's own name for it.
    pub name: String,
    /// What the model calls it: `<server>__<tool>`.
    pub exposed_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
    pub destructive: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub state: ServerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tools: Vec<ToolInfo>,
    /// The server's name and version, as it reported them.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub server_info: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
}

impl ServerStatus {
    fn new(state: ServerState) -> Self {
        Self {
            state,
            error: None,
            tools: Vec::new(),
            server_info: Value::Null,
            instructions: None,
            connected_at: None,
        }
    }
}

struct Slot {
    cfg: Mutex<McpServerConfig>,
    status: Mutex<ServerStatus>,
    client: Mutex<Option<Arc<Client>>>,
    /// Held while connecting, so only one connection attempt runs.
    connecting: tokio::sync::Mutex<()>,
    failed_at: Mutex<Option<Instant>>,
    /// Ends the slot's background work when it's replaced or removed.
    stop: CancellationToken,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether two configs need different connections (notes, trust and tool
/// switches don't).
fn same_connection(a: &McpServerConfig, b: &McpServerConfig) -> bool {
    a.name == b.name
        && a.command == b.command
        && a.args == b.args
        && a.env == b.env
        && a.cwd == b.cwd
        && a.url == b.url
        && a.headers == b.headers
}

fn tool_info(server: &str, t: RemoteTool) -> ToolInfo {
    ToolInfo {
        exposed_name: exposed_name(server, &t.name),
        name: t.name,
        title: t.title,
        description: t.description,
        input_schema: t.input_schema,
        read_only: t.read_only,
        destructive: t.destructive,
    }
}

pub struct McpHub {
    data_dir: PathBuf,
    http: reqwest::Client,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}

impl McpHub {
    pub fn new(data_dir: PathBuf) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("Relay/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Arc::new(Self {
            data_dir,
            http,
            slots: Mutex::default(),
        })
    }

    /// Matches the running servers to the settings: starts new or changed
    /// ones and stops removed or turned-off ones.
    pub fn sync(self: &Arc<Self>, servers: &[McpServerConfig]) {
        let mut stale = Vec::new();
        let mut start = Vec::new();
        {
            let mut slots = lock(&self.slots);
            slots.retain(|name, slot| {
                let keep = servers
                    .iter()
                    .any(|s| s.enabled && &s.name == name && same_connection(s, &lock(&slot.cfg)));
                if !keep {
                    stale.push(slot.clone());
                }
                keep
            });
            for s in servers.iter().filter(|s| s.enabled) {
                if let Some(slot) = slots.get(&s.name) {
                    // Same connection: only notes or switches changed.
                    *lock(&slot.cfg) = s.clone();
                    continue;
                }
                let slot = Arc::new(Slot {
                    cfg: Mutex::new(s.clone()),
                    status: Mutex::new(ServerStatus::new(ServerState::Starting)),
                    client: Mutex::new(None),
                    connecting: tokio::sync::Mutex::new(()),
                    failed_at: Mutex::new(None),
                    stop: CancellationToken::new(),
                });
                slots.insert(s.name.clone(), slot.clone());
                start.push(slot);
            }
        }
        for slot in stale {
            slot.stop.cancel();
            let client = lock(&slot.client).take();
            if let Some(c) = client {
                tokio::spawn(async move { c.close().await });
            }
        }
        for slot in start {
            let hub = self.clone();
            tokio::spawn(async move { hub.connect(&slot).await });
        }
    }

    /// Stops a server and starts it again.
    pub async fn restart(self: &Arc<Self>, name: &str) -> Option<ServerStatus> {
        let slot = lock(&self.slots).get(name).cloned()?;
        let old = lock(&slot.client).take();
        if let Some(c) = old {
            c.close().await;
        }
        *lock(&slot.status) = ServerStatus::new(ServerState::Starting);
        self.connect(&slot).await;
        Some(lock(&slot.status).clone())
    }

    /// Stops every server (Relay is closing).
    pub async fn shutdown(&self) {
        let slots: Vec<_> = lock(&self.slots).drain().map(|(_, s)| s).collect();
        for s in slots {
            s.stop.cancel();
            let c = lock(&s.client).take();
            if let Some(c) = c {
                c.close().await;
            }
        }
    }

    /// The state of the server called `name`, or Off if it isn't running.
    pub fn status(&self, name: &str) -> ServerStatus {
        lock(&self.slots)
            .get(name)
            .map(|s| {
                let mut st = lock(&s.status).clone();
                // A ready server whose process ended since.
                if st.state == ServerState::Ready
                    && let Some(c) = lock(&s.client).as_ref()
                    && !c.alive()
                {
                    st.state = ServerState::Error;
                    let tail = c.stderr_tail();
                    st.error = Some(if tail.is_empty() {
                        "the server stopped".into()
                    } else {
                        format!("the server stopped. It printed:\n{tail}")
                    });
                }
                st
            })
            .unwrap_or_else(|| ServerStatus::new(ServerState::Off))
    }

    /// Connects once, lists the tools and closes: checks a server before
    /// it's saved.
    pub async fn test(&self, cfg: &McpServerConfig) -> Result<(Vec<ToolInfo>, Value), String> {
        let client = Client::connect(cfg, &self.data_dir, &self.http)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let tools = client.list_tools().await;
        let info = client.server_info.clone();
        client.close().await;
        let tools = tools.map_err(|e| format!("{e:#}"))?;
        Ok((
            tools.into_iter().map(|t| tool_info(&cfg.name, t)).collect(),
            info,
        ))
    }

    async fn connect(self: &Arc<Self>, slot: &Arc<Slot>) {
        let _guard = slot.connecting.lock().await;
        if lock(&slot.client).as_ref().is_some_and(|c| c.alive()) {
            return;
        }
        lock(&slot.status).state = ServerState::Starting;
        let cfg = lock(&slot.cfg).clone();
        let name = cfg.name.clone();
        let result = async {
            let client = Client::connect(&cfg, &self.data_dir, &self.http).await?;
            let tools = client.list_tools().await;
            match tools {
                Ok(t) => Ok((client, t)),
                Err(e) => {
                    client.close().await;
                    Err(e)
                }
            }
        };
        let result = tokio::select! {
            r = result => r,
            _ = slot.stop.cancelled() => return,
        };
        match result {
            Ok((client, tools)) => {
                tracing::info!("mcp {name}: connected, {} tools", tools.len());
                let client = Arc::new(client);
                {
                    let mut st = lock(&slot.status);
                    st.state = ServerState::Ready;
                    st.error = None;
                    st.tools = tools.into_iter().map(|t| tool_info(&name, t)).collect();
                    st.server_info = client.server_info.clone();
                    st.instructions = client.instructions.clone();
                    st.connected_at = Some(crate::db::now());
                }
                *lock(&slot.failed_at) = None;
                *lock(&slot.client) = Some(client.clone());
                if slot.stop.is_cancelled() {
                    lock(&slot.client).take();
                    client.close().await;
                    return;
                }
                // Keep the tool list current when the server says it changed.
                let slot = slot.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = client.tools_changed.notified() => {}
                            _ = slot.stop.cancelled() => return,
                        }
                        if !client.alive() {
                            return;
                        }
                        if let Ok(t) = client.list_tools().await {
                            let name = lock(&slot.cfg).name.clone();
                            lock(&slot.status).tools =
                                t.into_iter().map(|t| tool_info(&name, t)).collect();
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!("mcp {name}: {e:#}");
                let mut st = lock(&slot.status);
                st.state = ServerState::Error;
                st.error = Some(format!("{e:#}"));
                st.tools.clear();
                *lock(&slot.failed_at) = Some(Instant::now());
            }
        }
    }

    /// Reconnects servers whose process stopped (or that failed a while
    /// ago), then waits a little for servers still starting.
    async fn wake(self: &Arc<Self>) {
        let slots: Vec<_> = lock(&self.slots).values().cloned().collect();
        for slot in &slots {
            let dead = lock(&slot.client).as_ref().is_some_and(|c| !c.alive());
            let retry = lock(&slot.status).state == ServerState::Error
                && lock(&slot.failed_at).is_none_or(|t| t.elapsed() > RETRY_AFTER);
            if dead || retry {
                lock(&slot.client).take();
                let hub = self.clone();
                let slot = slot.clone();
                tokio::spawn(async move { hub.connect(&slot).await });
            }
        }
        let deadline = Instant::now() + READY_WAIT;
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let waiting = slots
                .iter()
                .any(|s| lock(&s.status).state == ServerState::Starting);
            if !waiting || Instant::now() > deadline {
                break;
            }
        }
    }

    /// The tools of every ready server that the user hasn't turned off, as
    /// definitions for the model, plus the servers' hints for the system
    /// prompt.
    pub async fn definitions(
        self: &Arc<Self>,
        servers: &[McpServerConfig],
    ) -> (Vec<Value>, String) {
        if !servers.iter().any(|s| s.enabled) {
            return (Vec::new(), String::new());
        }
        self.wake().await;
        let mut defs = Vec::new();
        let mut hints = String::new();
        for cfg in servers.iter().filter(|s| s.enabled) {
            let st = self.status(&cfg.name);
            if st.state != ServerState::Ready {
                continue;
            }
            for t in st.tools.iter().filter(|t| cfg.tool_allowed(&t.name)) {
                let mut desc = t.description.trim().to_string();
                if desc.is_empty() {
                    desc = t.title.clone().unwrap_or_else(|| t.name.clone());
                }
                defs.push(json!({
                    "type": "function",
                    "function": {
                        "name": t.exposed_name,
                        "description": format!("[{} MCP server] {desc}", cfg.name),
                        "parameters": t.input_schema,
                    }
                }));
            }
            if let Some(i) = &st.instructions {
                let i: String = i.chars().take(2000).collect();
                hints.push_str(&format!("\n\n## {} (MCP server)\n{i}", cfg.name));
            }
        }
        (defs, hints)
    }

    /// The server config and tool behind a name the model used, if it's an
    /// MCP tool of an enabled server.
    pub fn lookup(
        &self,
        servers: &[McpServerConfig],
        exposed: &str,
    ) -> Option<(McpServerConfig, ToolInfo)> {
        let server = super::server_of(exposed)?;
        let cfg = servers.iter().find(|s| s.enabled && s.name == server)?;
        let tool = self
            .status(server)
            .tools
            .into_iter()
            .find(|t| t.exposed_name == exposed)?;
        Some((cfg.clone(), tool))
    }

    /// Runs one tool call on its server.
    pub async fn call(
        self: &Arc<Self>,
        cfg: &McpServerConfig,
        tool: &ToolInfo,
        arguments: &str,
        cancel: &CancellationToken,
    ) -> ToolOutput {
        let err = |s: String| ToolOutput {
            content: format!("error: {s}"),
            is_error: true,
        };
        if !cfg.tool_allowed(&tool.name) {
            return err(format!(
                "the user turned off {} in Relay's settings",
                tool.exposed_name
            ));
        }
        let args: Value = match serde_json::from_str(if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        }) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) => return err("arguments must be a JSON object".into()),
            Err(e) => return err(format!("arguments are not valid JSON ({e})")),
        };
        let Some(slot) = lock(&self.slots).get(&cfg.name).cloned() else {
            return err(format!("the {} MCP server is turned off", cfg.name));
        };
        let client = lock(&slot.client).clone().filter(|c| c.alive());
        let client = match client {
            Some(c) => c,
            None => {
                self.connect(&slot).await;
                match lock(&slot.client).clone() {
                    Some(c) => c,
                    None => {
                        let why = lock(&slot.status).error.clone().unwrap_or_default();
                        return err(format!("the {} MCP server isn't running. {why}", cfg.name));
                    }
                }
            }
        };
        match client
            .call_tool(&tool.name, args, cfg.timeout(), cancel)
            .await
        {
            Ok(r) => ToolOutput {
                content: r.text,
                is_error: r.is_error,
            },
            Err(e) => err(format!("{e:#}")),
        }
    }
}
