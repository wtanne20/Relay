//! Device hosts: a Relay running on another machine (e.g. a phone with a
//! Proot Linux) that exposes its own file system and shell to the PC's
//! agent. One binary, two roles: `relay serve` is the brain (model, agent
//! loop, history), `relay host` is the hands (files + commands on the
//! device). Usually the device dials in to the brain and keeps a link open
//! ([`crate::link`]); an older "direct" device is called over Tailscale with
//! a per-device bearer token instead. Either way every path is confined to
//! the host's root folder (or the project folder inside it) by the same
//! [`crate::tools::Sandbox`] used for local projects.
//!
//! Commands run as *jobs* (`POST /run` -> id, then poll `GET /run/{id}`)
//! rather than one long HTTP request, so long builds survive connection
//! drops and the PC can cancel them. The device never sees the model, the
//! session history or the PC's token.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::constant_time_eq;
use crate::db::Db;
use crate::tools::{self, Access, Sandbox, ToolOutput};

// ---------------------------------------------------------------- registry

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// Base URL the PC reaches the host at (e.g. http://100.x.y.z:8766).
    pub url: String,
    /// Bearer token the host was started with.
    pub token: String,
    /// Reported by the host: "linux", "android", ...
    pub os: String,
    /// Folder the host is rooted at (its project).
    pub root: String,
    /// Shell used by run_command on the device.
    pub shell: String,
    /// Last time the PC successfully reached the host (RFC 3339).
    pub last_seen: String,
    /// CPU architecture reported by the device ("x86_64", "aarch64", ...).
    #[serde(default)]
    pub arch: String,
    /// The device's own computer name.
    #[serde(default)]
    pub hostname: String,
}

/// `url` of a device that dials in to the brain (instead of the brain
/// calling it). Such a device is reachable only while its link is up.
pub const LINK_URL: &str = "link:";

impl Device {
    /// True for a device that connects out to the brain over a link.
    pub fn is_link(&self) -> bool {
        self.url.starts_with(LINK_URL)
    }
}

/// Registered devices, stored in the PC's relay.db.
pub struct DeviceRegistry {
    db: Db,
}

impl DeviceRegistry {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> Result<Vec<Device>> {
        self.db
            .call(|c| {
                let mut st = c.prepare(
                    "SELECT id, name, url, token, os, root, shell, last_seen, arch, hostname
                     FROM devices ORDER BY name COLLATE NOCASE",
                )?;
                let rows = st.query_map([], |r| {
                    Ok(Device {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        url: r.get(2)?,
                        token: r.get(3)?,
                        os: r.get(4)?,
                        root: r.get(5)?,
                        shell: r.get(6)?,
                        last_seen: r.get(7)?,
                        arch: r.get(8)?,
                        hostname: r.get(9)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| anyhow::anyhow!("{e}"))
            })
            .await
    }

    /// Adds or replaces a device (re-pairing keeps the same id).
    pub async fn upsert(&self, d: &Device) -> Result<()> {
        let (id, name, url, token, os, root, shell, last_seen) = (
            d.id.clone(),
            d.name.clone(),
            d.url.clone(),
            d.token.clone(),
            d.os.clone(),
            d.root.clone(),
            d.shell.clone(),
            d.last_seen.clone(),
        );
        let (arch, hostname) = (d.arch.clone(), d.hostname.clone());
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO devices (id, name, url, token, os, root, shell, last_seen, arch, hostname)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(id) DO UPDATE SET
                       name = excluded.name, url = excluded.url, token = excluded.token,
                       os = excluded.os, root = excluded.root, shell = excluded.shell,
                       last_seen = excluded.last_seen, arch = excluded.arch,
                       hostname = excluded.hostname",
                    rusqlite::params![
                        id, name, url, token, os, root, shell, last_seen, arch, hostname
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Device>> {
        let id = id.to_string();
        self.db
            .call(move |c| {
                let mut st = c.prepare(
                    "SELECT id, name, url, token, os, root, shell, last_seen, arch, hostname
                     FROM devices WHERE id = ?1",
                )?;
                let mut rows = st.query_map([id], |r| {
                    Ok(Device {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        url: r.get(2)?,
                        token: r.get(3)?,
                        os: r.get(4)?,
                        root: r.get(5)?,
                        shell: r.get(6)?,
                        last_seen: r.get(7)?,
                        arch: r.get(8)?,
                        hostname: r.get(9)?,
                    })
                })?;
                Ok(rows.next().transpose()?)
            })
            .await
    }

    /// The link device whose key this is (the key is what it connects with).
    pub async fn by_key(&self, key: &str) -> Result<Option<Device>> {
        let all = self.list().await?;
        Ok(all
            .into_iter()
            .find(|d| d.is_link() && constant_time_eq(d.token.as_bytes(), key.as_bytes())))
    }

    pub async fn remove(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.db
            .call(move |c| Ok(c.execute("DELETE FROM devices WHERE id = ?1", [id])? > 0))
            .await
    }

    /// Refreshes `last_seen` after a successful call to the host.
    pub fn touch(&self, id: &str) {
        let db = self.db.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let _ = db
                .call(move |c| {
                    c.execute(
                        "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
                        [crate::db::now(), id],
                    )?;
                    Ok(())
                })
                .await;
        });
    }

    /// Seen within the last `n` seconds counts as online.
    pub fn is_online(&self, d: &Device, n: u64) -> bool {
        let Ok(seen) = chrono::DateTime::parse_from_rfc3339(&d.last_seen) else {
            return false;
        };
        chrono::Utc::now()
            .signed_duration_since(seen.with_timezone(&chrono::Utc))
            .num_seconds()
            .abs()
            < n as i64
    }
}

// ---------------------------------------------------------------- host app

/// Header naming the project folder (an absolute path on the device, inside
/// its root) a request works in. Without it, requests work in the root.
pub const FOLDER_HEADER: &str = "x-relay-folder";

/// What a device host needs to know; everything else stays on the PC.
#[derive(Clone)]
pub struct HostConfig {
    pub token: String,
    pub name: String,
    pub root: PathBuf,
    pub shell: String,
    pub default_timeout: u64,
    /// Where this computer keeps its document index; None uses Relay's
    /// usual data folder.
    pub data_dir: Option<PathBuf>,
}

/// The `relay host` router: file + shell tools over HTTP, confined to
/// `root`, guarded by the host's own bearer token.
pub fn host_app(cfg: HostConfig) -> Result<Router> {
    let sandbox = Arc::new(Sandbox::new(&cfg.root, &[])?);
    let data_dir = cfg.data_dir.clone().unwrap_or_else(crate::config::default_data_dir);
    let kb = match crate::knowledge::Index::open(&data_dir.join("knowledge.db")) {
        Ok(i) => Some(i),
        Err(e) => {
            tracing::warn!("document search is unavailable on this computer: {e:#}");
            None
        }
    };
    Ok(Router::new()
        .route("/info", get(info))
        .route("/list", get(list))
        .route("/read", get(read))
        .route("/write", post(write))
        .route("/edit", post(edit))
        .route("/run", post(run_start))
        .route("/run/{id}", get(run_status).delete(run_cancel))
        .route("/kb/status", post(kb_status))
        .route("/kb/search", post(kb_search))
        .route("/kb/read", post(kb_read))
        .route("/kb/forget", post(kb_forget))
        .layer(middleware::from_fn_with_state(
            cfg.clone(),
            auth,
        ))
        .with_state(HostState {
            cfg,
            sandbox,
            runs: Arc::new(std::sync::Mutex::new(HashMap::new())),
            kb,
        }))
}

#[derive(Clone)]
struct HostState {
    cfg: HostConfig,
    sandbox: Arc<Sandbox>,
    runs: Arc<std::sync::Mutex<HashMap<String, RunJob>>>,
    /// This computer's document index.
    kb: Option<Arc<crate::knowledge::Index>>,
}

impl HostState {
    /// The sandbox for one request: the project folder it names (which must
    /// sit inside the device's root), or the root itself.
    fn sandbox_for(&self, headers: &axum::http::HeaderMap) -> ApiResult<Arc<Sandbox>> {
        let Some(folder) = headers
            .get(FOLDER_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|f| !f.is_empty())
        else {
            return Ok(Arc::clone(&self.sandbox));
        };
        let dir = self
            .sandbox
            .resolve_existing(folder, Access::Read)
            .map_err(|_| {
                DeviceError::BadRequest(format!(
                    "the project folder {folder} does not exist on this device, or is outside {}",
                    self.sandbox.root.display()
                ))
            })?;
        if !dir.is_dir() {
            return Err(DeviceError::BadRequest(format!("{folder} is not a folder")));
        }
        Ok(Arc::new(Sandbox::new(&dir, &[])?))
    }
}

// ---------------------------------------------------------------- documents

impl HostState {
    fn kb(&self) -> ApiResult<&Arc<crate::knowledge::Index>> {
        self.kb
            .as_ref()
            .ok_or_else(|| DeviceError::Internal("document search is unavailable on this computer".into()))
    }

    /// A chosen folder as its index key, if it exists inside this device's root.
    fn kb_folder(&self, folder: &str) -> Result<String, String> {
        let dir = self.sandbox.resolve_existing(folder, Access::Read).map_err(|_| {
            format!(
                "{folder} does not exist on this computer, or is outside the folder Relay may use ({})",
                self.sandbox.root.display()
            )
        })?;
        crate::knowledge::folder_key(&dir).map_err(|e| format!("{e:#}"))
    }

    fn kb_folders(&self, folders: &[String]) -> Vec<String> {
        folders.iter().filter_map(|f| self.kb_folder(f).ok()).collect()
    }
}

async fn kb_status(
    State(st): State<HostState>,
    Json(req): Json<crate::knowledge::KbRequest>,
) -> ApiResult<Json<Value>> {
    let kb = st.kb()?;
    let mut out = Vec::new();
    for f in &req.folders {
        match st.kb_folder(f) {
            Ok(key) => {
                let mut s = crate::knowledge::handle_status(kb, vec![key], req.force).await;
                if let Some(mut s) = s.pop() {
                    // Answer with the folder as the brain knows it.
                    s.path = f.clone();
                    out.push(s);
                }
            }
            Err(e) => out.push(crate::knowledge::FolderStatus {
                path: f.clone(),
                error: Some(e),
                ..Default::default()
            }),
        }
    }
    Ok(Json(json!({ "folders": out })))
}

async fn kb_search(
    State(st): State<HostState>,
    Json(mut req): Json<crate::knowledge::KbRequest>,
) -> ApiResult<Json<Value>> {
    let kb = st.kb()?;
    req.folders = st.kb_folders(&req.folders);
    Ok(Json(crate::knowledge::handle_search(kb, req).await?))
}

async fn kb_read(
    State(st): State<HostState>,
    Json(mut req): Json<crate::knowledge::KbRequest>,
) -> ApiResult<Json<Value>> {
    req.folders = st.kb_folders(&req.folders);
    let text = crate::knowledge::handle_read(req)
        .await
        .map_err(|e| DeviceError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({ "text": text })))
}

async fn kb_forget(
    State(st): State<HostState>,
    Json(req): Json<crate::knowledge::KbRequest>,
) -> ApiResult<Json<Value>> {
    let kb = st.kb()?;
    for f in &req.folders {
        if let Ok(key) = st.kb_folder(f) {
            crate::knowledge::handle_forget(kb, key).await?;
        }
    }
    Ok(Json(json!({ "ok": true })))
}

type ApiResult<T> = Result<T, DeviceError>;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for DeviceError {
    fn from(e: anyhow::Error) -> Self {
        let msg = e.to_string();
        // Sandbox denials and bad paths are client errors, not server faults.
        if msg.contains("access denied")
            || msg.contains("outside the project")
            || msg.contains("invalid path")
            || msg.contains("is a directory")
        {
            return DeviceError::BadRequest(msg);
        }
        tracing::error!("device host error: {e:#}");
        DeviceError::Internal(format!("{e:#}"))
    }
}

impl From<std::io::Error> for DeviceError {
    fn from(e: std::io::Error) -> Self {
        DeviceError::Internal(e.to_string())
    }
}

impl IntoResponse for DeviceError {
    fn into_response(self) -> Response {
        let status = match &self {
            DeviceError::BadRequest(_) => axum::http::StatusCode::BAD_REQUEST,
            DeviceError::Unauthorized => axum::http::StatusCode::UNAUTHORIZED,
            DeviceError::NotFound(_) => axum::http::StatusCode::NOT_FOUND,
            DeviceError::Conflict(_) => axum::http::StatusCode::CONFLICT,
            DeviceError::Internal(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

/// Requires `Authorization: Bearer <token>`.
async fn auth(State(token): State<HostConfig>, req: Request, next: Next) -> Response {
    let expected = token.token.as_bytes();
    let got = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(|s| s.trim());
    if got.is_some_and(|t| constant_time_eq(t.as_bytes(), expected)) {
        return next.run(req).await;
    }
    DeviceError::Unauthorized.into_response()
}

// ---------------------------------------------------------------- endpoints

async fn info(State(st): State<HostState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "name": st.cfg.name,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "root": st.sandbox.root.to_string_lossy(),
        "hostname": crate::link::hostname(),
        "shell": st.cfg.shell,
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

async fn list(State(st): State<HostState>, req: Request) -> ApiResult<Json<Value>> {
    let sandbox = st.sandbox_for(req.headers())?;
    let path = query_path(&req).unwrap_or_else(|| ".".into());
    let dir = sandbox.resolve_existing(&path, Access::Read)?;
    if !dir.is_dir() {
        return Err(DeviceError::BadRequest(format!("{path} is not a directory")));
    }
    let mut rd = tokio::fs::read_dir(&dir).await?;
    let mut entries: Vec<(bool, String, u64)> = Vec::new();
    while let Some(e) = rd.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        let meta = e.metadata().await.ok();
        entries.push((
            meta.as_ref().is_some_and(|m| m.is_dir()),
            name,
            meta.map(|m| m.len()).unwrap_or(0),
        ));
    }
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    Ok(Json(Value::Array(
        entries
            .into_iter()
            .map(|(is_dir, name, size)| json!({ "name": name, "dir": is_dir, "size": size }))
            .collect(),
    )))
}

async fn read(State(st): State<HostState>, req: Request) -> ApiResult<Response> {
    let sandbox = st.sandbox_for(req.headers())?;
    let path = query_path(&req).ok_or_else(|| DeviceError::BadRequest("missing ?path=".into()))?;
    let p = sandbox.resolve_existing(&path, Access::Read)?;
    if p.is_dir() {
        return Err(DeviceError::BadRequest(format!("{path} is a directory")));
    }
    let bytes = tokio::fs::read(&p).await?;
    let mime = if p.extension().is_some_and(|e| e == "png") {
        "image/png"
    } else if p.extension().is_some_and(|e| e == "jpg" || e == "jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    };
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let disp = format!("attachment; filename=\"{name}\"");
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_DISPOSITION, disp.as_str()),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct WriteBody {
    path: String,
    content: String,
}

async fn write(
    State(st): State<HostState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<WriteBody>,
) -> ApiResult<Json<Value>> {
    let sandbox = st.sandbox_for(&headers)?;
    let path = sandbox.resolve_for_write(&body.path)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Re-check after creating directories (defends against races with links).
    let path = sandbox.resolve_for_write(&body.path)?;
    tokio::fs::write(&path, &body.content).await?;
    Ok(Json(json!({
        "ok": true,
        "path": sandbox.display(&path),
        "bytes": body.content.len(),
    })))
}

#[derive(Deserialize)]
struct EditBody {
    path: String,
    old_text: String,
    new_text: String,
}

async fn edit(
    State(st): State<HostState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<EditBody>,
) -> ApiResult<Json<Value>> {
    let sandbox = st.sandbox_for(&headers)?;
    if body.old_text.is_empty() {
        return Err(DeviceError::BadRequest("old_text must not be empty".into()));
    }
    let path = sandbox.resolve_existing(&body.path, Access::Write)?;
    let original = tokio::fs::read_to_string(&path).await?;
    // Models often send \n while files use \r\n; try both (same as local edit).
    let (text, old) = if original.contains(&body.old_text) {
        (original.clone(), body.old_text.clone())
    } else if original.contains("\r\n") && !body.old_text.contains("\r\n") {
        (original.clone(), body.old_text.replace('\n', "\r\n"))
    } else {
        (original.clone(), body.old_text.clone())
    };
    let count = text.matches(&old).count();
    match count {
        0 => {
            return Err(DeviceError::BadRequest(format!(
                "old_text not found in {}",
                sandbox.display(&path)
            )))
        }
        1 => {}
        n => {
            return Err(DeviceError::Conflict(format!(
                "old_text appears {n} times; include more context so it is unique"
            )))
        }
    }
    let new = if old != body.old_text {
        body.new_text.replace('\n', "\r\n")
    } else {
        body.new_text.clone()
    };
    let updated = text.replacen(&old, &new, 1);
    tokio::fs::write(&path, updated).await?;
    Ok(Json(json!({
        "ok": true,
        "path": sandbox.display(&path),
    })))
}

/// The `?path=` query parameter, URL-decoded.
fn query_path(req: &Request) -> Option<String> {
    let q = req.uri().query()?;
    let raw = q
        .split('&')
        .find_map(|kv| kv.strip_prefix("path="))?
        .to_string();
    let mut out = String::new();
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
                let byte = u8::from_str_radix(hex, 16).ok()?;
                out.push(byte as char);
                i += 3;
            }
            b'+' => {
                out.push(' ');
                i += 1;
            }
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- run jobs

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Done,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub state: RunState,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone)]
struct RunJob {
    cancel: CancellationToken,
    result: Arc<tokio::sync::Mutex<Option<RunResult>>>,
    created: Instant,
}

/// Finished jobs are kept for an hour (a late poll still gets the answer).
const RUN_KEEP: Duration = Duration::from_secs(3600);

#[derive(Deserialize)]
struct RunBody {
    command: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

async fn run_start(
    State(st): State<HostState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<RunBody>,
) -> ApiResult<Json<Value>> {
    let sandbox = st.sandbox_for(&headers)?;
    let timeout = body
        .timeout_seconds
        .unwrap_or(st.cfg.default_timeout)
        .clamp(1, tools::MAX_COMMAND_TIMEOUT);
    let id = uuid::Uuid::new_v4().simple().to_string();
    let job = RunJob {
        cancel: CancellationToken::new(),
        result: Arc::new(tokio::sync::Mutex::new(None)),
        created: Instant::now(),
    };
    {
        let mut m = st.runs.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        m.retain(|_, j| now.duration_since(j.created) < RUN_KEEP);
        m.insert(id.clone(), job.clone());
    }
    let worker = Arc::clone(&job.result);
    let cancel = job.cancel.clone();
    let shell = st.cfg.shell.clone();
    let root = sandbox.root.clone();
    tokio::spawn(async move {
        let result = tools::run_shell(
            &shell,
            &body.command,
            &root,
            Duration::from_secs(timeout),
            &cancel,
        )
        .await;
        let r = match result {
            Ok(r) => RunResult {
                state: if r.timed_out {
                    RunState::TimedOut
                } else if r.cancelled {
                    RunState::Cancelled
                } else if r.exit_code == Some(0) {
                    RunState::Done
                } else {
                    RunState::Failed
                },
                exit_code: r.exit_code,
                stdout: r.stdout,
                stderr: r.stderr,
            },
            Err(e) => RunResult {
                state: RunState::Failed,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("error: {e:#}"),
            },
        };
        *worker.lock().await = Some(r);
    });
    Ok(Json(json!({ "id": id })))
}

async fn run_status(
    State(st): State<HostState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let job = st
        .runs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or_else(|| DeviceError::NotFound(format!("no run {id}")))?;
    match job.result.lock().await.clone() {
        Some(r) => Ok(Json(json!({
            "id": id,
            "state": r.state,
            "exit_code": r.exit_code,
            "stdout": r.stdout,
            "stderr": r.stderr,
        }))),
        None => Ok(Json(json!({ "id": id, "state": "running" }))),
    }
}

async fn run_cancel(
    State(st): State<HostState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let job = st
        .runs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .ok_or_else(|| DeviceError::NotFound(format!("no run {id}")))?;
    job.cancel.cancel();
    Ok(Json(json!({ "ok": true, "id": id })))
}

// ---------------------------------------------------------------- PC client

/// The PC side: calls a device host the same way the agent calls local
/// tools, so a session can work in the device's folder transparently. The
/// host is reached either over HTTP (a `relay host` the PC dials) or over
/// the link the device opened to the brain.
pub struct DeviceClient {
    transport: Transport,
    /// The project folder on the device (absolute), or the device root.
    folder: Option<String>,
}

enum Transport {
    Http {
        http: reqwest::Client,
        base: String,
        token: String,
    },
    Link(Option<Arc<crate::link::Link>>),
}

/// How long one request to a device may take (commands are polled jobs,
/// so this only bounds a single file read, write or poll).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

impl DeviceClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            transport: Transport::Http {
                http: reqwest::Client::builder()
                    // Fail fast when the device is unreachable (a dropped
                    // Tailscale link) instead of waiting out the full request
                    // timeout; the agent must never hang on a dead device.
                    .connect_timeout(Duration::from_secs(5))
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .unwrap_or_default(),
                base: url.trim_end_matches('/').to_string(),
                token: token.to_string(),
            },
            folder: None,
        }
    }

    /// The client for a registered device: over its link when it dials in,
    /// over HTTP otherwise.
    pub fn for_device(d: &Device, links: &crate::link::LinkHub) -> Self {
        if d.is_link() {
            Self { transport: Transport::Link(links.get(&d.id)), folder: None }
        } else {
            Self::new(&d.url, &d.token)
        }
    }

    /// Works in this folder on the device (an absolute path inside its root).
    pub fn in_folder(mut self, folder: Option<&str>) -> Self {
        self.folder = folder.map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
        self
    }

    /// One request; returns the body of a 2xx answer, an error otherwise.
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Vec<u8>> {
        let (status, bytes) = match &self.transport {
            Transport::Link(None) => {
                return Err(anyhow::anyhow!(
                    "the device is offline (it is not connected to this brain right now)"
                ));
            }
            Transport::Link(Some(link)) => {
                let body = body.map(|b| b.to_string().into_bytes());
                let r = link
                    .request(method, path, self.folder.as_deref(), body, REQUEST_TIMEOUT)
                    .await?;
                (r.status, r.body)
            }
            Transport::Http { http, base, token } => {
                let m = axum::http::Method::from_bytes(method.as_bytes())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let mut req = http
                    .request(m, format!("{base}{path}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"));
                if let Some(f) = &self.folder {
                    req = req.header(FOLDER_HEADER, f.as_str());
                }
                if let Some(b) = body {
                    req = req.json(&b);
                }
                let resp = req.send().await.map_err(|e| anyhow_err(&e))?;
                let status = resp.status().as_u16();
                let bytes = resp.bytes().await.map_err(|e| anyhow_err(&e))?.to_vec();
                (status, bytes)
            }
        };
        if (200..300).contains(&status) {
            return Ok(bytes);
        }
        let msg = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| format!("the device answered {status}"));
        Err(anyhow::anyhow!("{msg}"))
    }

    pub async fn info(&self) -> Result<Value> {
        self.get("/info").await
    }

    /// Lists a folder; returns the same shape the local list_dir prints.
    pub async fn list_dir(&self, path: &str) -> Result<String> {
        let entries = self.list_entries(path).await?;
        let total = entries.len();
        let mut out = format!("{} ({} entries)\n", path, total);
        for e in entries.into_iter().take(1000) {
            let name = e.get("name").and_then(Value::as_str).unwrap_or("");
            let dir = e.get("dir").and_then(Value::as_bool).unwrap_or(false);
            let size = e.get("size").and_then(Value::as_u64).unwrap_or(0);
            if dir {
                out.push_str(&format!("{name}/\n"));
            } else {
                out.push_str(&format!("{name}  {size}\n"));
            }
        }
        if total > 1000 {
            out.push_str(&format!("... {} more entries not shown\n", total - 1000));
        }
        Ok(out)
    }

    /// A folder's entries as `{name, dir, size}` objects.
    pub async fn list_entries(&self, path: &str) -> Result<Vec<Value>> {
        let v: Value = self.get(&format!("/list?path={}", urlenc(path))).await?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    /// A file's raw bytes.
    pub async fn read_bytes(&self, path: &str) -> Result<Vec<u8>> {
        self.call("GET", &format!("/read?path={}", urlenc(path)), None).await
    }

    pub async fn read_file(&self, path: &str) -> Result<String> {
        let bytes = self.read_bytes(path).await?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(tools::truncate_head(
            &text,
            tools::MAX_READ_CHARS,
            "use start_line/end_line to read the rest",
        ))
    }

    pub async fn write_file(&self, path: &str, content: &str) -> Result<String> {
        let v: Value = self
            .post("/write", json!({ "path": path, "content": content }))
            .await?;
        Ok(format!(
            "wrote {} ({} bytes)",
            v.get("path").and_then(Value::as_str).unwrap_or(path),
            v.get("bytes").and_then(Value::as_u64).unwrap_or(content.len() as u64)
        ))
    }

    pub async fn edit_file(&self, path: &str, old: &str, new: &str) -> Result<String> {
        let v: Value = self
            .post(
                "/edit",
                json!({ "path": path, "old_text": old, "new_text": new }),
            )
            .await?;
        Ok(format!(
            "edited {}",
            v.get("path").and_then(Value::as_str).unwrap_or(path)
        ))
    }

    /// Runs a command on the device and waits for it (bounded by the
    /// device-side timeout). Returns the same text the local run_command
    /// produces, so the model sees no difference.
    pub async fn run_command(&self, command: &str, timeout: u64) -> Result<ToolOutput> {
        let started: Value = self
            .post(
                "/run",
                json!({ "command": command, "timeout_seconds": timeout }),
            )
            .await?;
        let id = started
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let deadline = Instant::now() + Duration::from_secs(timeout.saturating_add(30));
        // A brief Tailscale blip shouldn't kill a long build, but a device
        // that is really gone must fail (not hang). Tolerate ~10s of
        // consecutive poll failures before giving up.
        let mut poll_fail_since: Option<Instant> = None;
        const POLL_GRACE: Duration = Duration::from_secs(10);
        loop {
            let v: Value = match self.get(&format!("/run/{id}")).await {
                Ok(v) => {
                    poll_fail_since = None;
                    v
                }
                Err(e) => {
                    let since = *poll_fail_since.get_or_insert_with(Instant::now);
                    if Instant::now() - since > POLL_GRACE {
                        let _ = self.cancel_run(&id).await;
                        return Err(anyhow::anyhow!("device stopped responding: {e}"));
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };
            let state = v.get("state").and_then(Value::as_str).unwrap_or("running");
            if state != "running" {
                let stdout = v.get("stdout").and_then(Value::as_str).unwrap_or("").to_string();
                let stderr = v.get("stderr").and_then(Value::as_str).unwrap_or("").to_string();
                let exit = v.get("exit_code").and_then(Value::as_i64).map(|n| n as i32);
                let mut out = String::new();
                match state {
                    "timed_out" => {
                        out.push_str(&format!("timed out after {timeout}s; process killed\n"))
                    }
                    "cancelled" => out.push_str("cancelled by user; process killed\n"),
                    _ => {
                        if let Some(c) = exit {
                            out.push_str(&format!("exit_code: {c}\n"));
                        }
                    }
                }
                out.push_str("--- stdout ---\n");
                out.push_str(&stdout);
                if !stdout.ends_with('\n') && !stdout.is_empty() {
                    out.push('\n');
                }
                out.push_str("--- stderr ---\n");
                out.push_str(&stderr);
                let is_error = matches!(state, "timed_out" | "cancelled" | "failed")
                    || exit != Some(0);
                return Ok(ToolOutput {
                    content: tools::truncate_middle(&out, tools::MAX_COMMAND_OUTPUT),
                    is_error,
                });
            }
            if Instant::now() > deadline {
                let _ = self.cancel_run(&id).await;
                return Err(anyhow_err("timed out waiting for the device"));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Cancels a running command on the device.
    pub async fn cancel_run(&self, id: &str) -> Result<()> {
        self.call("DELETE", &format!("/run/{id}"), None).await?;
        Ok(())
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let bytes = self.call("GET", path, None).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn post<T: for<'de> Deserialize<'de>>(&self, path: &str, body: Value) -> Result<T> {
        let bytes = self.call("POST", path, Some(body)).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// One document-index request (`/kb/status`, `/kb/search`, ...).
    pub async fn kb(&self, action: &str, req: &crate::knowledge::KbRequest) -> Result<Value> {
        self.post(&format!("/kb/{action}"), serde_json::to_value(req)?)
            .await
            .map_err(|e| {
                if e.to_string().contains("answered 404") {
                    anyhow::anyhow!("Relay on this computer is too old to search documents; update it")
                } else {
                    e
                }
            })
    }
}

fn anyhow_err<T: std::fmt::Display>(e: T) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Makes the devices table available (idempotent; also called from
/// `db::open_connection` so it exists even before a device is added).
pub fn ensure_schema(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS devices (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            token TEXT NOT NULL,
            os TEXT NOT NULL DEFAULT '',
            root TEXT NOT NULL DEFAULT '',
            shell TEXT NOT NULL DEFAULT '',
            last_seen TEXT NOT NULL DEFAULT ''
        );",
    )?;
    let has = |col: &str| -> Result<bool> {
        let mut st = c.prepare("SELECT name FROM pragma_table_info('devices')")?;
        let names = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names.iter().any(|n| n == col))
    };
    for col in ["arch", "hostname"] {
        if !has(col)? {
            c.execute_batch(&format!(
                "ALTER TABLE devices ADD COLUMN {col} TEXT NOT NULL DEFAULT ''"
            ))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_urls() {
        assert_eq!(urlenc("a b/c"), "a%20b%2Fc");
        assert_eq!(urlenc("C:\\x\\y.txt"), "C%3A%5Cx%5Cy.txt");
    }

    #[tokio::test]
    async fn host_serves_tools_confined_to_root() {
        let base_dir = std::env::temp_dir().join(format!("relay-host-{}", uuid::Uuid::new_v4()));
        let root = base_dir.join("root");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), "hello").unwrap();
        let outside = base_dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("s.txt"), "secret").unwrap();

        let cfg = HostConfig {
            token: "tok".into(),
            name: "test".into(),
            root: root.clone(),
            shell: "sh".into(),
            default_timeout: 30,
            data_dir: Some(std::env::temp_dir().join(format!("relay-host-{}", uuid::Uuid::new_v4()))),
        };
        let app = host_app(cfg).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let base = format!("http://127.0.0.1:{port}");
        let http = reqwest::Client::new();

        // No token -> 401.
        assert_eq!(
            http.get(format!("{base}/list")).send().await.unwrap().status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        // With token -> listing.
        let v: Value = http
            .get(format!("{base}/list"))
            .header(header::AUTHORIZATION, "Bearer tok")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(v.as_array().unwrap().iter().any(|e| e["name"] == "src"));
        // Escape the root -> 400 (denied, never a file read).
        let st = http
            .get(format!("{base}/read?path=../outside/s.txt"))
            .header(header::AUTHORIZATION, "Bearer tok")
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(st, axum::http::StatusCode::BAD_REQUEST);
        // Write + read back.
        http.post(format!("{base}/write"))
            .header(header::AUTHORIZATION, "Bearer tok")
            .json(&json!({ "path": "src/b.txt", "content": "x" }))
            .send()
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(root.join("src/b.txt")).unwrap(), "x");
        // Edit it.
        let st = http
            .post(format!("{base}/edit"))
            .header(header::AUTHORIZATION, "Bearer tok")
            .json(&json!({ "path": "src/b.txt", "old_text": "x", "new_text": "xy" }))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(st, axum::http::StatusCode::OK);
        assert_eq!(std::fs::read_to_string(root.join("src/b.txt")).unwrap(), "xy");
        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[tokio::test]
    async fn run_jobs_complete_and_cancel() {
        let dir = std::env::temp_dir().join(format!("relay-run-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = HostConfig {
            token: "tok".into(),
            name: "test".into(),
            root: dir.clone(),
            shell: if cfg!(windows) { "cmd".into() } else { "sh".into() },
            default_timeout: 30,
            data_dir: Some(dir.join(".relay")),
        };
        let app = host_app(cfg).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let base = format!("http://127.0.0.1:{port}");
        let http = reqwest::Client::new();
        let tok = "Bearer tok";

        // A command that writes a file, then read it back.
        let cmd = "echo hello > out.txt";
        let started: Value = http
            .post(format!("{base}/run"))
            .header("authorization", tok)
            .json(&json!({ "command": cmd, "timeout_seconds": 10 }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let id = started["id"].as_str().unwrap().to_string();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let v = loop {
            let v: Value = http
                .get(format!("{base}/run/{id}"))
                .header("authorization", tok)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if v["state"] != "running" || tokio::time::Instant::now() > deadline {
                break v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert_eq!(v["state"], "done");
        assert_eq!(v["exit_code"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A device that is down must fail fast, not hang the agent.
    #[tokio::test]
    async fn unreachable_device_fails_quickly() {
        // Take a free port, then close it so the client has nothing to reach.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let dc = DeviceClient::new(&format!("http://127.0.0.1:{port}"), "tok");
        let started = tokio::time::Instant::now();
        let err = dc.info().await.expect_err("must fail, not hang");
        let elapsed = started.elapsed();
        let msg = format!("{err:#}");
        // It is a real failure (connection refused / timeout), never a
        // silent success or an empty result.
        assert!(
            !msg.trim().is_empty() && msg.contains("error"),
            "unexpected error: {msg}"
        );
        // Well under the 60s request timeout: a dead link fails in seconds.
        assert!(
            elapsed < Duration::from_secs(20),
            "took {elapsed:?} — an unreachable device must fail fast"
        );
    }
}
