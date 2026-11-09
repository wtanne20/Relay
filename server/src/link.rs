//! Device links: a device dials *out* to the brain and keeps one WebSocket
//! open. The brain sends tool requests down that socket and the device
//! answers them with its own host router (the same file + shell endpoints
//! `relay host` serves over HTTP). Nothing listens on the device, so there is
//! no port to open, no address to guess and no firewall rule to add: the
//! device only has to reach the brain over Tailscale.
//!
//! Wire format (JSON text frames):
//! - brain -> device: `{"t":"req","id":7,"m":"GET","p":"/list?path=.","f":"/home/me/app","b":"<base64>"}`
//!   (`f` is the project folder on the device, `b` the request body; both optional)
//! - device -> brain: `{"t":"res","id":7,"s":200,"b":"<base64>"}`
//!
//! Either side may close the socket at any time. Requests still waiting when
//! the link drops fail at once with "device disconnected", so the agent never
//! hangs on a device that went away.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use axum::extract::ws::{Message as WsMessage, WebSocket};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// How often each side pings, and how long silence means the link is dead.
const PING_EVERY: Duration = Duration::from_secs(15);
const DEAD_AFTER: Duration = Duration::from_secs(45);

#[derive(Debug, Serialize, Deserialize)]
struct ReqFrame {
    t: String,
    id: u64,
    m: String,
    p: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    f: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    b: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResFrame {
    t: String,
    id: u64,
    s: u16,
    #[serde(default)]
    b: String,
}

/// A response that came back over a link.
#[derive(Debug, Clone)]
pub struct LinkResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

// ---------------------------------------------------------------- brain side

/// One connected device.
pub struct Link {
    tx: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<LinkResponse>>>,
    next: AtomicU64,
    generation: u64,
    pub connected_at: chrono::DateTime<chrono::Utc>,
    /// The address the device connected from (its Tailscale IP).
    pub remote: String,
}

impl Link {
    /// Sends one request to the device and waits for its answer.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        folder: Option<&str>,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<LinkResponse> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let frame = ReqFrame {
            t: "req".into(),
            id,
            m: method.into(),
            p: path.into(),
            f: folder.map(str::to_string),
            b: body.map(|b| B64.encode(b)),
        };
        if self.tx.send(serde_json::to_string(&frame)?).is_err() {
            self.forget(id);
            bail!("device disconnected");
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => bail!("device disconnected"),
            Err(_) => {
                self.forget(id);
                bail!("device did not answer within {}s", timeout.as_secs())
            }
        }
    }

    fn forget(&self, id: u64) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }

    fn complete(&self, res: ResFrame) {
        let waiter = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&res.id);
        if let Some(w) = waiter {
            let body = B64.decode(res.b.as_bytes()).unwrap_or_default();
            let _ = w.send(LinkResponse { status: res.s, body });
        }
    }
}

/// Every device currently connected to this brain, by device id.
#[derive(Default)]
pub struct LinkHub {
    links: Mutex<HashMap<String, Arc<Link>>>,
    generation: AtomicU64,
}

impl LinkHub {
    pub fn get(&self, device_id: &str) -> Option<Arc<Link>> {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(device_id)
            .cloned()
    }

    pub fn is_connected(&self, device_id: &str) -> bool {
        self.get(device_id).is_some()
    }

    /// Drops a device's link (for example when it is removed).
    pub fn disconnect(&self, device_id: &str) {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(device_id);
    }

    /// Runs one device's socket until it closes. A newer connection from the
    /// same device replaces this one.
    pub async fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        device_id: String,
        remote: String,
        shutdown: CancellationToken,
        on_seen: impl Fn() + Send + 'static,
    ) {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let link = Arc::new(Link {
            tx,
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            generation,
            connected_at: chrono::Utc::now(),
            remote,
        });
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device_id.clone(), Arc::clone(&link));
        tracing::info!("device {device_id} connected from {}", link.remote);
        on_seen();

        let mut socket = socket;
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        let mut last_heard = tokio::time::Instant::now();
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                out = rx.recv() => match out {
                    Some(text) => {
                        if socket.send(WsMessage::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                msg = socket.recv() => match msg {
                    Some(Ok(WsMessage::Text(t))) => {
                        last_heard = tokio::time::Instant::now();
                        match serde_json::from_str::<ResFrame>(t.as_str()) {
                            Ok(res) if res.t == "res" => link.complete(res),
                            _ => tracing::debug!("device {device_id}: ignoring frame"),
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => last_heard = tokio::time::Instant::now(),
                },
                _ = ping.tick() => {
                    if last_heard.elapsed() > DEAD_AFTER {
                        tracing::warn!("device {device_id} went quiet; dropping its link");
                        break;
                    }
                    if socket.send(WsMessage::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                    on_seen();
                }
            }
        }
        // Only remove the entry if a newer connection hasn't replaced it.
        {
            let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
            if links.get(&device_id).is_some_and(|l| l.generation == generation) {
                links.remove(&device_id);
            }
        }
        // Waiting requests fail now ("device disconnected") instead of timing out.
        link.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        on_seen();
        tracing::info!("device {device_id} disconnected");
    }
}

// ---------------------------------------------------------------- device side

/// What a device keeps after pairing (saved as `device.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceConfig {
    /// The brain, e.g. `http://100.101.102.103:8765`.
    pub brain_url: String,
    /// The brain's computer name, for display.
    #[serde(default)]
    pub brain_name: String,
    /// This device's id in the brain's device list.
    pub device_id: String,
    /// The key this device presents when it connects (revoked by removing it).
    pub device_key: String,
    /// The brain's API token, so this machine's window can open the chats.
    #[serde(default)]
    pub api_token: String,
    /// The folder the agent may work in on this machine (default: home).
    pub root: PathBuf,
    /// This device's name in the brain's list.
    pub name: String,
    /// Shell for run_command.
    #[serde(default = "default_shell")]
    pub shell: String,
}

fn default_shell() -> String {
    if cfg!(windows) { "cmd".into() } else { "sh".into() }
}

impl DeviceConfig {
    pub fn path_in(dir: &std::path::Path) -> PathBuf {
        dir.join("device.toml")
    }

    pub fn load(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = format!(
            "# Relay: this computer is linked to a brain. Delete this file to unlink.\n{}",
            toml::to_string(self)?
        );
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

/// The state of this device's link, for the desktop window and the CLI.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LinkState {
    Connecting,
    Connected,
    /// The brain is unreachable; retrying.
    Offline { message: String },
    /// The brain no longer knows this device (it was removed).
    Revoked,
}

/// The ws:// address of a brain's link endpoint.
fn link_url(brain: &str) -> String {
    let b = brain.trim_end_matches('/');
    let rest = b
        .strip_prefix("https://")
        .map(|r| format!("wss://{r}"))
        .or_else(|| b.strip_prefix("http://").map(|r| format!("ws://{r}")))
        .unwrap_or_else(|| format!("ws://{b}"));
    format!("{rest}/devices/link")
}

/// Keeps this device linked to its brain until `shutdown` is cancelled:
/// connects, answers requests with the host router, and reconnects with
/// backoff whenever the link drops. `on_state` hears every change.
pub async fn run_device_link(
    cfg: DeviceConfig,
    shutdown: CancellationToken,
    on_state: impl Fn(LinkState) + Send + Sync + 'static,
) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TMsg;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    // The host router answers the brain's requests in-process. It still
    // wants a bearer token; this one never leaves the process.
    let inner_token = crate::config::generate_token();
    let router = crate::device::host_app(crate::device::HostConfig {
        token: inner_token.clone(),
        name: cfg.name.clone(),
        root: cfg.root.clone(),
        shell: cfg.shell.clone(),
        default_timeout: 120,
    })?;

    let url = link_url(&cfg.brain_url);
    let mut backoff = Duration::from_secs(1);
    loop {
        if shutdown.is_cancelled() {
            return Ok(());
        }
        on_state(LinkState::Connecting);
        let mut req = url.as_str().into_client_request()?;
        req.headers_mut().insert(
            "authorization",
            format!("Bearer {}", cfg.device_key).parse()?,
        );
        req.headers_mut()
            .insert("x-relay-device", cfg.device_id.parse()?);
        let connect = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(req),
        );
        let ws = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            r = connect => r,
        };
        let ws = match ws {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(resp)))
                if resp.status() == 401 || resp.status() == 403 =>
            {
                on_state(LinkState::Revoked);
                tracing::warn!("the brain refused this device's key (it was removed)");
                // Keep checking slowly: re-pairing writes a new key.
                tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                }
                continue;
            }
            Ok(Err(e)) => {
                on_state(LinkState::Offline { message: format!("{e}") });
                tracing::info!("brain not reachable ({e}); retrying in {}s", backoff.as_secs());
                tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
            Err(_) => {
                on_state(LinkState::Offline { message: "timed out connecting".into() });
                tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        backoff = Duration::from_secs(1);
        on_state(LinkState::Connected);
        tracing::info!("linked to the brain at {}", cfg.brain_url);

        let (mut sink, mut stream) = ws.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<TMsg>();
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        let mut last_heard = tokio::time::Instant::now();
        let reason = loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    let _ = sink.send(TMsg::Close(None)).await;
                    return Ok(());
                }
                out = out_rx.recv() => {
                    if let Some(m) = out
                        && sink.send(m).await.is_err()
                    {
                        break "send failed".to_string();
                    }
                }
                msg = stream.next() => match msg {
                    Some(Ok(TMsg::Text(t))) => {
                        last_heard = tokio::time::Instant::now();
                        let Ok(frame) = serde_json::from_str::<ReqFrame>(t.as_str()) else {
                            continue;
                        };
                        if frame.t != "req" {
                            continue;
                        }
                        let router = router.clone();
                        let token = inner_token.clone();
                        let out = out_tx.clone();
                        tokio::spawn(async move {
                            let res = answer(router, &token, frame).await;
                            if let Ok(text) = serde_json::to_string(&res) {
                                let _ = out.send(TMsg::Text(text.into()));
                            }
                        });
                    }
                    Some(Ok(TMsg::Close(_))) | None => break "the brain closed the link".to_string(),
                    Some(Err(e)) => break format!("{e}"),
                    Some(Ok(_)) => last_heard = tokio::time::Instant::now(),
                },
                _ = ping.tick() => {
                    if last_heard.elapsed() > DEAD_AFTER {
                        break "the brain went quiet".to_string();
                    }
                    if sink.send(TMsg::Ping(Vec::new().into())).await.is_err() {
                        break "ping failed".to_string();
                    }
                }
            }
        };
        tracing::info!("link dropped ({reason}); reconnecting");
        on_state(LinkState::Offline { message: reason });
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

/// Runs one request through the host router and packs the answer.
async fn answer(router: axum::Router, token: &str, f: ReqFrame) -> ResFrame {
    use tower::ServiceExt;
    let fail = |s: u16, msg: String| ResFrame {
        t: "res".into(),
        id: f.id,
        s,
        b: B64.encode(json!({ "error": msg }).to_string()),
    };
    let body = match f.b.as_deref().map(|b| B64.decode(b.as_bytes())) {
        Some(Ok(b)) => b,
        Some(Err(_)) => return fail(400, "bad request body".into()),
        None => Vec::new(),
    };
    let mut builder = axum::http::Request::builder()
        .method(f.m.as_str())
        .uri(if f.p.starts_with('/') { f.p.clone() } else { format!("/{}", f.p) })
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    if let Some(folder) = &f.f {
        builder = builder.header(crate::device::FOLDER_HEADER, folder.as_str());
    }
    let req = match builder.body(axum::body::Body::from(body)) {
        Ok(r) => r,
        Err(e) => return fail(400, format!("bad request: {e}")),
    };
    let res = match router.oneshot(req).await {
        Ok(r) => r,
        Err(e) => return fail(500, format!("{e}")),
    };
    let status = res.status().as_u16();
    match axum::body::to_bytes(res.into_body(), 64 * 1024 * 1024).await {
        Ok(bytes) => ResFrame { t: "res".into(), id: f.id, s: status, b: B64.encode(bytes) },
        Err(e) => fail(500, format!("reading response: {e}")),
    }
}

// ---------------------------------------------------------------- pairing + discovery

/// What `/devices/claim` hands a newly paired device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimResponse {
    pub device_id: String,
    pub device_key: String,
    pub api_token: String,
    pub brain_name: String,
}

/// This machine's name ("my-laptop"), for device lists and labels.
pub fn hostname() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(h) = std::env::var(var)
            && !h.trim().is_empty()
        {
            return h.trim().to_string();
        }
    }
    for f in ["/etc/hostname", "/proc/sys/kernel/hostname"] {
        if let Ok(h) = std::fs::read_to_string(f)
            && !h.trim().is_empty()
        {
            return h.trim().to_string();
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        let h = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !h.is_empty() {
            return h;
        }
    }
    "this-computer".into()
}

/// Adds `http://` and the default brain port when they are missing.
pub fn normalize_brain_url(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let with_scheme = if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("http://{s}")
    };
    let after = with_scheme.split_once("//").map(|(_, r)| r).unwrap_or(&with_scheme);
    let host = after.split('/').next().unwrap_or(after);
    // A bare IPv6 address has colons but no port; only treat `]:` or a
    // single colon as "has a port".
    let has_port = if host.starts_with('[') {
        host.contains("]:")
    } else {
        host.matches(':').count() == 1
    };
    if has_port { with_scheme } else { format!("{with_scheme}:8765") }
}

/// Pairs this machine with a brain using the 6-digit code the brain shows.
pub async fn claim(
    brain_url: &str,
    code: &str,
    name: &str,
    root: &std::path::Path,
    previous_id: Option<&str>,
) -> Result<ClaimResponse> {
    let brain = normalize_brain_url(brain_url);
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    if code.len() != 6 {
        bail!("the pairing code has 6 digits");
    }
    let body = json!({
        "code": code,
        "name": name,
        "hostname": hostname(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "root": root.to_string_lossy(),
        "previous_id": previous_id,
    });
    let resp = reqwest::Client::new()
        .post(format!("{brain}/devices/claim"))
        .header("accept", "application/json")
        .timeout(Duration::from_secs(15))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("could not reach the brain at {brain}: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the brain refused the code");
        bail!("{msg}");
    }
    Ok(serde_json::from_value(v)?)
}

/// Pairs with a brain and saves the result to `path` (`device.toml`).
/// Re-pairing the same machine keeps its device id, so its projects stay
/// bound to it.
pub async fn pair_and_save(
    path: &std::path::Path,
    brain_url: &str,
    code: &str,
    name: Option<&str>,
    root: Option<&std::path::Path>,
) -> Result<DeviceConfig> {
    let previous = DeviceConfig::load(path);
    let brain = normalize_brain_url(brain_url);
    let root = match root {
        Some(r) => r.to_path_buf(),
        None => previous
            .as_ref()
            .map(|p| p.root.clone())
            .unwrap_or_else(home_dir),
    };
    let root = dunce::canonicalize(&root)
        .map_err(|e| anyhow!("the folder to share ({}) is not usable: {e}", root.display()))?;
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| previous.as_ref().map(|p| p.name.clone()))
        .unwrap_or_else(hostname);
    // Only reuse the id when re-pairing with the same brain.
    let previous_id = previous
        .as_ref()
        .filter(|p| p.brain_url == brain)
        .map(|p| p.device_id.clone());
    let claimed = claim(&brain, code, &name, &root, previous_id.as_deref()).await?;
    let cfg = DeviceConfig {
        brain_url: brain,
        brain_name: claimed.brain_name,
        device_id: claimed.device_id,
        device_key: claimed.device_key,
        api_token: claimed.api_token,
        root,
        name,
        shell: previous.map(|p| p.shell).unwrap_or_else(default_shell),
    };
    cfg.save(path)?;
    Ok(cfg)
}

/// The user's home folder (what a device shares by default).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// A brain found on the tailnet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FoundBrain {
    pub name: String,
    pub url: String,
    pub version: String,
    pub os: String,
}

/// Looks for Relay brains among this machine's online Tailscale peers (and
/// on this machine itself) by asking each one's `/hello`.
pub async fn discover_brains() -> Vec<FoundBrain> {
    let mut candidates: Vec<(String, String)> = vec![("this computer".into(), "127.0.0.1".into())];
    if let Some(json) = crate::setup::tailscale_status_json().await {
        candidates.extend(tailscale_peers(&json));
    }
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(1500))
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap_or_default();
    let probes = candidates.into_iter().map(|(label, ip)| {
        let http = http.clone();
        async move {
            let url = format!("http://{ip}:8765");
            let v: Value = http
                .get(format!("{url}/hello"))
                .header("accept", "application/json")
                .send()
                .await
                .ok()?
                .json()
                .await
                .ok()?;
            if v.get("relay").and_then(Value::as_str) != Some("brain") {
                return None;
            }
            Some(FoundBrain {
                name: v
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&label)
                    .to_string(),
                url,
                version: v.get("version").and_then(Value::as_str).unwrap_or("").into(),
                os: v.get("os").and_then(Value::as_str).unwrap_or("").into(),
            })
        }
    });
    let mut found: Vec<FoundBrain> = futures_util::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect();
    found.dedup_by(|a, b| a.url == b.url);
    found
}

/// (host name, IPv4) of each online peer in `tailscale status --json`.
pub fn tailscale_peers(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(peers) = v.get("Peer").and_then(Value::as_object) else {
        return Vec::new();
    };
    peers
        .values()
        .filter(|p| p.get("Online").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|p| {
            let ip = p
                .get("TailscaleIPs")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_str)
                .find(|ip| ip.contains('.'))?
                .to_string();
            let name = p
                .get("HostName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some((name, ip))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_brain_addresses() {
        assert_eq!(normalize_brain_url("100.1.2.3"), "http://100.1.2.3:8765");
        assert_eq!(normalize_brain_url("my-pc:9000/"), "http://my-pc:9000");
        assert_eq!(normalize_brain_url("https://pc.ts.net"), "https://pc.ts.net:8765");
        assert_eq!(normalize_brain_url(" http://pc:8765 "), "http://pc:8765");
    }

    #[test]
    fn link_urls() {
        assert_eq!(link_url("http://pc:8765"), "ws://pc:8765/devices/link");
        assert_eq!(link_url("https://pc:8765/"), "wss://pc:8765/devices/link");
    }

    #[test]
    fn reads_online_tailscale_peers() {
        let json = r#"{
          "Self": {"HostName": "laptop", "TailscaleIPs": ["100.1.1.1"]},
          "Peer": {
            "a": {"HostName": "desktop", "Online": true, "TailscaleIPs": ["100.2.2.2", "fd7a::1"]},
            "b": {"HostName": "old-phone", "Online": false, "TailscaleIPs": ["100.3.3.3"]}
          }
        }"#;
        assert_eq!(tailscale_peers(json), vec![("desktop".to_string(), "100.2.2.2".to_string())]);
    }

    #[test]
    fn device_config_round_trips() {
        let dir = std::env::temp_dir().join(format!("relay-devcfg-{}", uuid::Uuid::new_v4()));
        let path = DeviceConfig::path_in(&dir);
        let cfg = DeviceConfig {
            brain_url: "http://100.2.2.2:8765".into(),
            brain_name: "desktop".into(),
            device_id: "d1".into(),
            device_key: "k".into(),
            api_token: "t".into(),
            root: PathBuf::from("/home/me"),
            name: "laptop".into(),
            shell: "sh".into(),
        };
        cfg.save(&path).unwrap();
        assert_eq!(DeviceConfig::load(&path), Some(cfg));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
