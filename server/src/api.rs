//! REST routes, authentication middleware and the shared application state.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{FromRequest, Path as UrlPath, Query, Request, State, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::agent::{AgentManager, ApprovalError, Decision, StartError};
use crate::config::{
    CACHE_TYPES, Config, DEFAULT_SYSTEM_PROMPT, SharedConfig, UpdateError, constant_time_eq,
};
use crate::db::{self, Db};
use crate::model_manager::ModelManager;
use crate::push::Pusher;
use crate::setup::{self, Hardware, Tailscale};

pub struct AppInner {
    pub cfg: Arc<SharedConfig>,
    /// GPUs found at startup.
    pub hardware: Hardware,
    /// This PC's Tailscale identity, if Tailscale is running.
    pub tailscale: Option<Tailscale>,
    pub db: Db,
    pub model: ModelManager,
    pub agent: Arc<AgentManager>,
    pub pusher: Pusher,
    /// Cancelled when Relay is shutting down (closes WebSockets).
    pub shutdown: CancellationToken,
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// When a client other than this PC (the phone) last made an
    /// authenticated request. Persisted in `<data>/phone_seen`.
    pub phone_seen: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    pub data_dir: std::path::PathBuf,
    /// Model downloads from Hugging Face.
    pub downloads: Arc<crate::hub::Downloads>,
}

/// Reads the persisted "phone last seen" time.
pub fn load_phone_seen(data_dir: &Path) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = std::fs::read_to_string(data_dir.join("phone_seen")).ok()?;
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// Records a request from another device, at most once a minute.
fn note_remote_client(state: &AppInner, ip: std::net::IpAddr) {
    if ip.is_loopback() {
        return;
    }
    let now = chrono::Utc::now();
    let mut seen = state.phone_seen.lock().unwrap_or_else(|e| e.into_inner());
    if seen.is_some_and(|t| (now - t).num_seconds() < 60) {
        return;
    }
    *seen = Some(now);
    let path = state.data_dir.join("phone_seen");
    tokio::spawn(async move {
        if let Err(e) = tokio::fs::write(&path, now.to_rfc3339()).await {
            tracing::debug!("could not save {}: {e}", path.display());
        }
    });
}

pub type AppState = Arc<AppInner>;

// ---------- errors ----------

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("internal error: {e:#}");
        ApiError::Internal(format!("{e:#}"))
    }
}

impl From<StartError> for ApiError {
    fn from(e: StartError) -> Self {
        match e {
            StartError::NotFound => ApiError::NotFound("session"),
            StartError::AlreadyRunning => ApiError::Conflict(e.to_string()),
            StartError::Invalid(m) => ApiError::BadRequest(m),
            StartError::Busy(m) => ApiError::Conflict(m),
            StartError::Internal(e) => e.into(),
        }
    }
}

impl From<UpdateError> for ApiError {
    fn from(e: UpdateError) -> Self {
        match e {
            UpdateError::Invalid(m) => ApiError::BadRequest(m),
            UpdateError::Save(e) => e.into(),
        }
    }
}

impl From<ApprovalError> for ApiError {
    fn from(e: ApprovalError) -> Self {
        ApiError::Conflict(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// JSON body extractor whose rejections use our `{"error": ...}` shape.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(JsonBody(v)),
            Err(rej) => Err(ApiError::BadRequest(rej.body_text())),
        }
    }
}

// ---------- router ----------

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/info", get(info))
        .route("/pairing", get(pairing))
        .route("/model", get(model_status))
        .route("/model/restart", post(model_restart))
        .route("/models", get(list_models))
        .route("/models/tags", axum::routing::put(set_model_tags))
        .route("/models/import", post(import_model))
        .route("/hub/search", get(hub_search))
        .route("/hub/files", get(hub_files))
        .route("/hub/downloads", get(hub_downloads).post(hub_download))
        .route("/hub/downloads/{id}", axum::routing::delete(hub_cancel))
        .route("/settings", get(get_settings).patch(patch_settings))
        .route(
            "/system-prompt",
            get(get_system_prompt).put(put_system_prompt),
        )
        .route("/skills", get(list_skills))
        .route("/recent", get(recent))
        .route("/mockups", post(create_mockup))
        .route("/mockups/{id}", get(get_mockup))
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{id}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route(
            "/projects/{id}/sessions",
            get(list_sessions).post(create_session),
        )
        .route(
            "/sessions/{id}",
            get(get_session).patch(patch_session).delete(delete_session),
        )
        .route("/sessions/{id}/messages", get(list_messages))
        .route("/sessions/{id}/prompt", post(send_prompt))
        .route("/sessions/{id}/stop", post(stop_task))
        .route("/sessions/{id}/approval", post(answer_approval))
        .route("/sessions/{id}/ws", get(ws_upgrade))
        .route(
            "/push-tokens",
            post(add_push_token).delete(remove_push_token),
        )
        .fallback(|| async { ApiError::NotFound("route") })
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        // Outside auth so browser preflight requests (which carry no token) get an answer.
        .layer(cors())
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Origins allowed to call the API from a browser engine: the Relay desktop
/// app's window and the local Expo dev server. Every request still needs the
/// token; this only lets those pages read the responses.
pub const ALLOWED_ORIGINS: &[&str] = &[
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://localhost:8081",
    "http://127.0.0.1:8081",
];

fn cors() -> tower_http::cors::CorsLayer {
    use axum::http::{HeaderValue, Method};
    let origins: Vec<HeaderValue> = ALLOWED_ORIGINS
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(600))
}

/// Requires `Authorization: Bearer <token>` on every request. WebSocket
/// upgrades may instead pass `?token=` for clients that cannot set headers.
async fn auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let expected_token = state.cfg.read(|l| l.config.token.clone());
    let expected = expected_token.as_bytes();
    let header_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(|s| s.trim().to_string());
    let query_token = if req.uri().path().ends_with("/ws") {
        req.uri().query().and_then(|q| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix("token=").map(|t| t.to_string()))
        })
    } else {
        None
    };
    let ok = header_token
        .or(query_token)
        .is_some_and(|t| constant_time_eq(t.as_bytes(), expected));
    if !ok {
        return ApiError::Unauthorized.into_response();
    }
    if let Some(axum::extract::ConnectInfo(addr)) = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        note_remote_client(&state, addr.ip());
    }
    next.run(req).await
}

// ---------- info & model ----------

async fn info(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let projects = s.db.call(|c| Ok(db::list_projects(c)?.len())).await?;
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    Ok(Json(json!({
        "name": "relay",
        "version": env!("CARGO_PKG_VERSION"),
        "hostname": host,
        "os": std::env::consts::OS,
        "started_at": s.started_at.to_rfc3339(),
        "uptime_seconds": (chrono::Utc::now() - s.started_at).num_seconds(),
        "projects": projects,
        "push_enabled": s.pusher.enabled(),
        "max_steps": s.cfg.read(|l| l.config.agent.max_steps),
        "model": s.model.status(),
        "gpus": s.hardware.gpus,
        "tailscale": s.tailscale,
        "phone_last_seen": s
            .phone_seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|t| t.to_rfc3339()),
    })))
}

/// What the phone needs to pair: the address (Tailscale when available),
/// the relay:// link and the QR code as SVG. Lets the desktop app show the
/// pairing code.
async fn pairing(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let (bind, token) = s
        .cfg
        .read(|l| (l.config.bind.clone(), l.config.token.clone()));
    let url = setup::pairing_url(s.tailscale.as_ref(), crate::port_of(&bind));
    let payload = setup::pairing_payload(&url, &token);
    let svg =
        setup::qr_svg(&payload, 256).map_err(|e| ApiError::Internal(format!("QR code: {e}")))?;
    Ok(Json(
        json!({ "url": url, "link": payload, "svg": svg, "tailscale": s.tailscale.is_some() }),
    ))
}

// ---------- settings ----------

/// Top-level config sections the app may change. The token, bind address
/// and shell stay config.toml-only.
const EDITABLE_SECTIONS: &[&str] = &[
    "agent",
    "sampling",
    "llama",
    "push",
    "disabled_skills",
    "projects_folder",
    "models_folder",
];

fn settings_view(c: &Config) -> Value {
    json!({
        "agent": c.agent,
        "sampling": c.sampling,
        "llama": c.llama,
        "push": c.push,
        "disabled_skills": c.disabled_skills,
        "projects_folder": c.projects_folder,
        "models_folder": c.models_folder,
    })
}

async fn get_settings(State(s): State<AppState>) -> Json<Value> {
    let c = s.cfg.get();
    let tools: Vec<Value> = crate::tools::catalog()
        .into_iter()
        .map(|(name, description)| json!({ "name": name, "description": description }))
        .collect();
    Json(json!({
        "settings": settings_view(&c),
        "defaults": settings_view(&Config::default()),
        "tools": tools,
        "cache_types": CACHE_TYPES,
        "gpus": s.hardware.gpus,
    }))
}

/// Recursively merges `patch` into `base` (JSON merge patch, except that
/// null is not allowed as a value).
fn merge(base: &mut Value, patch: &Value, path: &str) -> ApiResult<()> {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v, &child)?,
                    None => return Err(ApiError::BadRequest(format!("unknown setting '{child}'"))),
                }
            }
            Ok(())
        }
        (_, Value::Null) => Err(ApiError::BadRequest(format!("'{path}' may not be null"))),
        (Value::Object(_), _) => Err(ApiError::BadRequest(format!("'{path}' must be an object"))),
        (slot, v) => {
            *slot = v.clone();
            Ok(())
        }
    }
}

/// Applies a partial settings update, e.g. `{"sampling": {"temperature": 0.2}}`.
/// Changes to the `llama` section restart llama-server; everything else
/// applies from the next prompt.
async fn patch_settings(
    State(s): State<AppState>,
    JsonBody(patch): JsonBody<Value>,
) -> ApiResult<Json<Value>> {
    let Value::Object(obj) = &patch else {
        return Err(ApiError::BadRequest("expected a JSON object".into()));
    };
    if let Some(k) = obj
        .keys()
        .find(|k| !EDITABLE_SECTIONS.contains(&k.as_str()))
    {
        return Err(ApiError::BadRequest(format!(
            "'{k}' cannot be changed from the app"
        )));
    }
    let current = s.cfg.get();
    let mut view = settings_view(&current);
    merge(&mut view, &patch, "")?;
    #[derive(Deserialize)]
    struct Sections {
        agent: crate::config::AgentConfig,
        sampling: crate::config::SamplingConfig,
        llama: crate::config::LlamaConfig,
        push: crate::config::PushConfig,
        disabled_skills: Vec<String>,
        projects_folder: String,
        models_folder: String,
    }
    let next: Sections = serde_json::from_value(view)
        .map_err(|e| ApiError::BadRequest(format!("invalid settings: {e}")))?;
    if next.llama.model_path != current.llama.model_path && !next.llama.model_path.trim().is_empty()
    {
        let p = Path::new(next.llama.model_path.trim());
        let is_gguf = p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
        if !is_gguf || !p.is_file() {
            return Err(ApiError::BadRequest(format!(
                "model must be an existing .gguf file on the PC: {}",
                p.display()
            )));
        }
    }
    if next.llama.llama_server_path != current.llama.llama_server_path
        && !next.llama.llama_server_path.trim().is_empty()
    {
        let mut l = next.llama.clone();
        l.llama_server_path = l.llama_server_path.trim().to_string();
        if !l.executable().is_file() {
            return Err(ApiError::BadRequest(format!(
                "llama-server not found at {}",
                l.executable().display()
            )));
        }
    }
    let projects_folder = if next.projects_folder.trim().is_empty() {
        String::new()
    } else if next.projects_folder == current.projects_folder {
        current.projects_folder.clone()
    } else {
        prepare_folder(&next.projects_folder).await?
    };
    let models_folder = if next.models_folder.trim().is_empty() {
        String::new()
    } else if next.models_folder == current.models_folder {
        current.models_folder.clone()
    } else {
        prepare_folder(&next.models_folder).await?
    };
    let restart = next.llama != current.llama;
    let updated = s.cfg.update(move |c| {
        c.agent = next.agent;
        c.sampling = next.sampling;
        c.llama = next.llama;
        c.push = next.push;
        c.disabled_skills = next.disabled_skills;
        c.projects_folder = projects_folder;
        c.models_folder = models_folder;
    })?;
    if restart {
        s.model.restart().await;
    }
    Ok(Json(json!({
        "settings": settings_view(&updated),
        "model_restarting": restart,
    })))
}

async fn get_system_prompt(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let path = s.cfg.read(|l| l.system_prompt_path());
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_SYSTEM_PROMPT.to_string(),
        Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", path.display()).into()),
    };
    Ok(Json(json!({
        "content": content,
        "default": DEFAULT_SYSTEM_PROMPT,
        "path": path.to_string_lossy(),
    })))
}

#[derive(Deserialize)]
struct SystemPrompt {
    content: String,
}

/// Replaces system_prompt.md. An empty body restores the default.
async fn put_system_prompt(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<SystemPrompt>,
) -> ApiResult<Json<Value>> {
    if body.content.len() > 200_000 {
        return Err(ApiError::BadRequest(
            "system prompt is too long (max 200,000 bytes)".into(),
        ));
    }
    let content = if body.content.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        body.content
    };
    let path = s.cfg.read(|l| l.system_prompt_path());
    tokio::fs::write(&path, &content)
        .await
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Json(json!({ "content": content })))
}

async fn list_skills(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let (roots, disabled) = s
        .cfg
        .read(|l| (l.skill_roots(), l.config.disabled_skills.clone()));
    let roots_out: Vec<String> = roots
        .iter()
        .map(|r| r.to_string_lossy().into_owned())
        .collect();
    let found = tokio::task::spawn_blocking(move || crate::skills::discover(&roots))
        .await
        .map_err(|e| anyhow::anyhow!("skill scan failed: {e}"))?;
    let skills: Vec<Value> = found
        .iter()
        .map(|k| {
            json!({
                "name": k.name,
                "description": k.description,
                "path": k.path.to_string_lossy(),
                "enabled": !disabled.contains(&k.name),
            })
        })
        .collect();
    Ok(Json(json!({ "skills": skills, "roots": roots_out })))
}

async fn list_models(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let (dirs, current) = s
        .cfg
        .read(|l| (l.model_dirs(), l.config.llama.model_path.clone()));
    let dirs_out: Vec<String> = dirs
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    let cur = current.clone();
    let models = tokio::task::spawn_blocking(move || {
        let mut found = crate::setup::discover_models(&dirs);
        // The configured model may live outside the scanned folders.
        let p = Path::new(&cur);
        if !cur.is_empty() && p.is_file() && !found.iter().any(|m| m.path == cur) {
            found.insert(
                0,
                crate::setup::ModelFile {
                    path: cur.clone(),
                    name: p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    size_bytes: p.metadata().map(|m| m.len()).unwrap_or(0),
                },
            );
        }
        found
    })
    .await
    .map_err(|e| anyhow::anyhow!("model scan failed: {e}"))?;
    let tags = s.cfg.read(|l| l.config.model_tags.clone());
    let models: Vec<Value> = models
        .into_iter()
        .map(|m| {
            let (t, custom) = match tags.get(&m.name) {
                Some(t) => (t.clone(), true),
                None => (setup::guess_model_tags(&m.name), false),
            };
            json!({
                "path": m.path,
                "name": m.name,
                "size_bytes": m.size_bytes,
                "tags": t,
                "custom_tags": custom,
            })
        })
        .collect();
    Ok(Json(json!({
        "models": models,
        "current": current,
        "dirs": dirs_out,
        "vram_mb": s.hardware.total_vram_mb(),
    })))
}

#[derive(Deserialize)]
struct ModelTags {
    /// The model's file name, as listed by GET /models.
    name: String,
    /// Replaces the model's tags. `null` goes back to the guessed tags.
    tags: Option<Vec<String>>,
}

async fn set_model_tags(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<ModelTags>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(ApiError::BadRequest(
            "name must be a model file name".into(),
        ));
    }
    let tags = body.tags.map(|t| {
        let mut out: Vec<String> = Vec::new();
        for tag in t {
            let tag: String = tag.trim().chars().take(24).collect();
            if !tag.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&tag)) {
                out.push(tag);
            }
        }
        out.truncate(8);
        out
    });
    let shown = tags
        .clone()
        .unwrap_or_else(|| setup::guess_model_tags(&name));
    s.cfg.update(move |c| match tags {
        Some(t) => {
            c.model_tags.insert(name, t);
        }
        None => {
            c.model_tags.remove(&name);
        }
    })?;
    Ok(Json(json!({ "tags": shown })))
}

// ---------- model downloads (Hugging Face) ----------

#[derive(Deserialize)]
struct HubSearch {
    #[serde(default)]
    q: String,
}

async fn hub_search(Query(q): Query<HubSearch>) -> ApiResult<Json<Value>> {
    let query: String = q.q.trim().chars().take(100).collect();
    let hits = crate::hub::search(&query)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({ "results": hits })))
}

#[derive(Deserialize)]
struct HubFiles {
    repo: String,
}

async fn hub_files(State(s): State<AppState>, Query(q): Query<HubFiles>) -> ApiResult<Json<Value>> {
    if !crate::hub::valid_repo(&q.repo) {
        return Err(ApiError::BadRequest(
            "not a Hugging Face repository name".into(),
        ));
    }
    let files = crate::hub::files(&q.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({
        "repo": q.repo,
        "files": files,
        "vram_mb": s.hardware.total_vram_mb(),
    })))
}

async fn hub_downloads(State(s): State<AppState>) -> Json<Value> {
    Json(json!({ "downloads": s.downloads.list() }))
}

#[derive(Deserialize)]
struct HubDownload {
    repo: String,
    file: String,
}

async fn hub_download(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<HubDownload>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !crate::hub::valid_repo(&body.repo) || !crate::hub::valid_file(&body.file) {
        return Err(ApiError::BadRequest(
            "invalid repository or file name".into(),
        ));
    }
    // Look the file up so only real GGUF files from the repository are fetched.
    let files = crate::hub::files(&body.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let model = files
        .into_iter()
        .find(|f| f.file == body.file)
        .ok_or(ApiError::NotFound("model file"))?;
    let models_dir = relay_models_dir(&s);
    let d = s
        .downloads
        .start(&models_dir, &body.repo, model)
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "download": d }))))
}

/// Where Relay saves every model it downloads or copies: its own models
/// folder (in its data folder unless moved in Settings), never another app's.
fn relay_models_dir(s: &AppState) -> std::path::PathBuf {
    s.cfg.read(|l| l.relay_models_dir())
}

#[derive(Deserialize)]
struct ImportModel {
    /// A .gguf file elsewhere on the PC, e.g. in another app's folder.
    path: String,
}

/// Copies a model file into Relay's own models folder. When the copied file
/// is the model in use, Relay switches to its copy once the copy is done
/// (waiting for running chats to finish first). The original is never moved
/// or changed.
async fn import_model(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<ImportModel>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let src = Path::new(body.path.trim().trim_matches('"')).to_path_buf();
    let is_gguf = src
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
    if !src.is_absolute() || !is_gguf || !src.is_file() {
        return Err(ApiError::BadRequest(format!(
            "not a .gguf file on the PC: {}",
            src.display()
        )));
    }
    let models_dir = relay_models_dir(&s);
    let canon_models = dunce::canonicalize(&models_dir).unwrap_or(models_dir.clone());
    let canon_src = dunce::canonicalize(&src).unwrap_or(src.clone());
    if canon_src.starts_with(&canon_models) {
        return Err(ApiError::BadRequest(
            "that model is already in Relay's models folder".into(),
        ));
    }
    let d = s
        .downloads
        .import(&models_dir, &src)
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;

    let old = src.to_string_lossy().into_owned();
    let id = d.id.clone();
    let st = s.clone();
    tokio::spawn(async move {
        // Wait for the copy, then for any running chat, then switch.
        let new = loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if let Some(dest) = st.downloads.imported_copy_done(&id) {
                break dest;
            }
            let still = st
                .downloads
                .list()
                .iter()
                .any(|d| d.id == id && d.state == crate::hub::DownloadState::Downloading);
            if !still {
                return;
            }
        };
        let (o, n) = (old.clone(), new.clone());
        if let Err(e) = st.db.call(move |c| db::replace_model_path(c, &o, &n)).await {
            tracing::warn!("could not update chats to the copied model: {e:#}");
        }
        let in_use = |st: &AppState| st.cfg.get().llama.model_path.eq_ignore_ascii_case(&old);
        if !in_use(&st) {
            return;
        }
        while st.agent.any_running() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        if !in_use(&st) {
            return;
        }
        let path = new.clone();
        match st.cfg.update(move |c| c.llama.model_path = path) {
            Ok(_) => {
                tracing::info!("switched to Relay's own copy of the model: {new}");
                st.model.switch_to(&new).await;
            }
            Err(e) => tracing::warn!("could not switch to the copied model: {e:#}"),
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "download": d }))))
}

async fn hub_cancel(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    if s.downloads.cancel(&id) {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::NotFound("download"))
    }
}

// ---------- mockups ----------

#[derive(Deserialize)]
struct NewMockup {
    /// The HTML (or SVG) the model wrote.
    source: String,
    #[serde(default)]
    svg: bool,
}

/// Mockup pictures are drawn one at a time.
static MOCKUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Draws a mockup to a picture for the phone. The desktop app shows the HTML
/// live instead.
async fn create_mockup(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<NewMockup>,
) -> ApiResult<Json<Value>> {
    if body.source.trim().is_empty() {
        return Err(ApiError::BadRequest("the mockup is empty".into()));
    }
    if body.source.len() > crate::mockup::MAX_HTML {
        return Err(ApiError::BadRequest(
            "the mockup is too large to draw".into(),
        ));
    }
    let page = crate::mockup::prepare(&body.source, body.svg);
    let size = crate::mockup::size_hint(&body.source);
    let id = crate::mockup::mockup_id(&page, size);
    let dir = s.data_dir.join("mockups");
    let _turn = MOCKUP_LOCK.lock().await;
    crate::mockup::render(&dir, &id, &page, size)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({ "id": id, "width": size.0, "height": size.1 })))
}

async fn get_mockup(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Response> {
    let id = id.trim_end_matches(".png");
    if !crate::mockup::valid_id(id) {
        return Err(ApiError::NotFound("mockup"));
    }
    let path = s.data_dir.join("mockups").join(format!("{id}.png"));
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound("mockup"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn model_status(State(s): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(s.model.status()).unwrap_or(Value::Null))
}

async fn model_restart(State(s): State<AppState>) -> (StatusCode, Json<Value>) {
    s.model.restart().await;
    (StatusCode::ACCEPTED, Json(json!({ "ok": true })))
}

// ---------- projects ----------

#[derive(Deserialize)]
struct CreateProject {
    name: Option<String>,
    /// An existing folder. When empty, a folder named after the project is
    /// created inside the projects folder from the settings.
    #[serde(default)]
    path: Option<String>,
}

/// Checks that a folder setting is an absolute path and creates it if it
/// doesn't exist yet. Returns the canonical path.
async fn prepare_folder(path: &str) -> ApiResult<String> {
    let trimmed = path.trim().trim_matches('"').to_string();
    if !Path::new(&trimmed).is_absolute() {
        return Err(ApiError::BadRequest(format!(
            "projects folder must be a full path (for example C:\\Users\\me\\Projects): {trimmed}"
        )));
    }
    tokio::fs::create_dir_all(&trimmed)
        .await
        .map_err(|e| ApiError::BadRequest(format!("could not create {trimmed}: {e}")))?;
    validate_folder(&trimmed)
}

/// Turns a project name into a folder name that is valid on Windows.
fn folder_name(name: &str) -> ApiResult<String> {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." || reserved {
        return Err(ApiError::BadRequest(format!(
            "'{name}' can't be used as a folder name; pick another project name"
        )));
    }
    Ok(cleaned)
}

#[derive(Deserialize)]
struct UpdateProject {
    name: Option<String>,
    path: Option<String>,
    instructions: Option<String>,
    /// A model file for the project's chats; "" goes back to the loaded model.
    model: Option<String>,
}

/// `Some("")` clears the choice; otherwise it must be a .gguf file on the PC.
fn validate_model(model: Option<String>) -> ApiResult<Option<Option<String>>> {
    let Some(m) = model else { return Ok(None) };
    let m = m.trim().trim_matches('"').to_string();
    if m.is_empty() {
        return Ok(Some(None));
    }
    let p = Path::new(&m);
    let is_gguf = p
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
    if !is_gguf || !p.is_file() {
        return Err(ApiError::BadRequest(format!(
            "model must be an existing .gguf file on the PC: {m}"
        )));
    }
    Ok(Some(Some(m)))
}

/// Validates and canonicalizes a project folder path.
fn validate_folder(path: &str) -> ApiResult<String> {
    let trimmed = path.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest("path is required".into()));
    }
    let p = Path::new(trimmed);
    if !p.is_absolute() {
        return Err(ApiError::BadRequest(format!(
            "path must be absolute (for example C:\\Users\\me\\code\\app): {trimmed}"
        )));
    }
    if !p.is_dir() {
        return Err(ApiError::BadRequest(format!(
            "folder does not exist on the PC: {trimmed}"
        )));
    }
    let canon =
        dunce::canonicalize(p).map_err(|e| ApiError::BadRequest(format!("{trimmed}: {e}")))?;
    Ok(canon.to_string_lossy().into_owned())
}

async fn list_projects(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let projects = s.db.call(|c| db::list_projects(c)).await?;
    Ok(Json(json!({ "projects": projects })))
}

async fn create_project(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<CreateProject>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let given = body.path.as_deref().map(str::trim).unwrap_or_default();
    let path = if given.is_empty() {
        let name = body.name.as_deref().map(str::trim).unwrap_or_default();
        if name.is_empty() {
            return Err(ApiError::BadRequest(
                "a project name or folder is required".into(),
            ));
        }
        let root = s.cfg.get().projects_folder.clone();
        if root.trim().is_empty() {
            return Err(ApiError::BadRequest(
                "set a projects folder in Settings first, or choose an existing folder".into(),
            ));
        }
        let dir = Path::new(&root).join(folder_name(name)?);
        // Reuses the folder if it already exists; never removes or moves anything.
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            ApiError::BadRequest(format!("could not create {}: {e}", dir.display()))
        })?;
        validate_folder(&dir.to_string_lossy())?
    } else {
        validate_folder(given)?
    };
    let name = match body.name.as_deref().map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone()),
    };
    let p = path.clone();
    let existing = s.db.call(move |c| db::find_project_by_path(c, &p)).await?;
    if let Some(e) = existing {
        return Err(ApiError::Conflict(format!(
            "that folder is already the project '{}'",
            e.name
        )));
    }
    let project =
        s.db.call(move |c| db::create_project(c, &name, &path))
            .await?;
    Ok((StatusCode::CREATED, Json(json!({ "project": project }))))
}

async fn get_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let project =
        s.db.call(move |c| db::get_project(c, &id))
            .await?
            .ok_or(ApiError::NotFound("project"))?;
    Ok(Json(json!({ "project": project })))
}

async fn update_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<UpdateProject>,
) -> ApiResult<Json<Value>> {
    let name = match body.name.as_deref().map(str::trim) {
        Some("") => return Err(ApiError::BadRequest("name must not be empty".into())),
        other => other.map(str::to_string),
    };
    if id == db::CHATS_PROJECT_ID && (name.is_some() || body.path.is_some()) {
        return Err(ApiError::BadRequest(
            "plain chats have no name or folder to change".into(),
        ));
    }
    let path = body.path.as_deref().map(validate_folder).transpose()?;
    let model = validate_model(body.model)?;
    let instructions = body.instructions;
    if instructions.as_ref().is_some_and(|i| i.len() > 50_000) {
        return Err(ApiError::BadRequest(
            "instructions are too long (max 50,000 bytes)".into(),
        ));
    }
    let project =
        s.db.call(move |c| {
            db::update_project(
                c,
                &id,
                name.as_deref(),
                path.as_deref(),
                instructions.as_deref(),
                model.as_ref().map(|m| m.as_deref()),
            )
        })
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    Ok(Json(json!({ "project": project })))
}

/// Removes the project from Relay only. The folder on disk is never touched.
async fn delete_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    if id == db::CHATS_PROJECT_ID {
        return Err(ApiError::BadRequest("plain chats can't be removed".into()));
    }
    let pid = id.clone();
    let sessions =
        s.db.call(move |c| db::list_session_ids_for_project(c, &pid))
            .await?;
    if sessions.iter().any(|sid| s.agent.is_running(sid)) {
        return Err(ApiError::Conflict(
            "stop the running task in this project first".into(),
        ));
    }
    let deleted = s.db.call(move |c| db::delete_project(c, &id)).await?;
    if !deleted {
        return Err(ApiError::NotFound("project"));
    }
    for sid in &sessions {
        s.agent.forget(sid);
    }
    Ok(Json(json!({ "ok": true })))
}

// ---------- sessions ----------

#[derive(Deserialize, Default)]
struct CreateSession {
    title: Option<String>,
}

#[derive(Deserialize)]
struct PatchSession {
    title: Option<String>,
    /// A model file for this chat; "" follows the project again.
    model: Option<String>,
}

#[derive(Deserialize)]
struct RecentQuery {
    limit: Option<usize>,
}

/// Recent chats across every project and plain chats, for the home screen.
async fn recent(State(s): State<AppState>, Query(q): Query<RecentQuery>) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(30).clamp(1, 200);
    let sessions = s.db.call(move |c| db::recent_sessions(c, limit)).await?;
    Ok(Json(json!({ "sessions": sessions })))
}

async fn list_sessions(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let sessions =
        s.db.call(move |c| {
            if db::get_project(c, &id)?.is_none() {
                return Ok(None);
            }
            Ok(Some(db::list_sessions(c, &id)?))
        })
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    Ok(Json(json!({ "sessions": sessions })))
}

async fn create_session(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    body: axum::body::Bytes,
) -> ApiResult<(StatusCode, Json<Value>)> {
    // The body is optional: an empty request creates a session with a default title.
    let parsed: CreateSession = if body.iter().all(u8::is_ascii_whitespace) {
        CreateSession::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::BadRequest(format!("invalid JSON body: {e}")))?
    };
    // Without a title, the chat is called "New chat" until Relay names it
    // after the first reply.
    let given = parsed
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let auto = given.is_none();
    let title = given.unwrap_or_else(|| "New chat".into());
    let session =
        s.db.call(move |c| {
            if db::get_project(c, &id)?.is_none() {
                return Ok(None);
            }
            Ok(Some(db::create_session(c, &id, &title, auto)?))
        })
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    Ok((StatusCode::CREATED, Json(json!({ "session": session }))))
}

async fn get_session(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let session =
        s.db.call(move |c| db::get_session(c, &id))
            .await?
            .ok_or(ApiError::NotFound("session"))?;
    Ok(Json(json!({ "session": session })))
}

async fn patch_session(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<PatchSession>,
) -> ApiResult<Json<Value>> {
    let title = match body.title.as_deref().map(str::trim) {
        Some("") => return Err(ApiError::BadRequest("title must not be empty".into())),
        other => other.map(str::to_string),
    };
    let model = validate_model(body.model)?;
    let session =
        s.db.call(move |c| {
            if db::get_session(c, &id)?.is_none() {
                return Ok(None);
            }
            if let Some(t) = &title {
                db::rename_session(c, &id, t)?;
            }
            if let Some(m) = &model {
                db::set_session_model(c, &id, m.as_deref())?;
            }
            db::get_session(c, &id)
        })
        .await?
        .ok_or(ApiError::NotFound("session"))?;
    Ok(Json(json!({ "session": session })))
}

async fn delete_session(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    if s.agent.is_running(&id) {
        return Err(ApiError::Conflict("stop the running task first".into()));
    }
    let sid = id.clone();
    if !s.db.call(move |c| db::delete_session(c, &sid)).await? {
        return Err(ApiError::NotFound("session"));
    }
    s.agent.forget(&id);
    Ok(Json(json!({ "ok": true })))
}

// ---------- messages & tasks ----------

#[derive(Deserialize)]
struct MessageQuery {
    before: Option<i64>,
    after: Option<i64>,
    limit: Option<usize>,
}

async fn list_messages(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<MessageQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let page =
        s.db.call(move |c| {
            if db::get_session(c, &id)?.is_none() {
                return Ok(None);
            }
            Ok(Some(db::page_messages(c, &id, q.before, q.after, limit)?))
        })
        .await?
        .ok_or(ApiError::NotFound("session"))?;
    Ok(Json(json!({ "messages": page.0, "has_more": page.1 })))
}

#[derive(Deserialize)]
struct Prompt {
    content: String,
}

async fn send_prompt(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<Prompt>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let message = s.agent.start(&id, body.content).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "message": message, "status": "running" })),
    ))
}

async fn stop_task(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let sid = id.clone();
    if s.db
        .call(move |c| db::get_session(c, &sid))
        .await?
        .is_none()
    {
        return Err(ApiError::NotFound("session"));
    }
    let stopping = s.agent.stop(&id);
    Ok(Json(json!({ "stopping": stopping })))
}

#[derive(Deserialize)]
struct ApprovalAnswer {
    tool_call_id: String,
    decision: Decision,
    /// Optional reason passed to the model when denying.
    note: Option<String>,
}

async fn answer_approval(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<ApprovalAnswer>,
) -> ApiResult<Json<Value>> {
    let note = body.note.map(|n| n.chars().take(2000).collect::<String>());
    s.agent
        .resolve_approval(&id, &body.tool_call_id, body.decision, note)?;
    Ok(Json(json!({ "ok": true })))
}

async fn ws_upgrade(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let sid = id.clone();
    if s.db
        .call(move |c| db::get_session(c, &sid))
        .await?
        .is_none()
    {
        return Err(ApiError::NotFound("session"));
    }
    Ok(ws.on_upgrade(move |socket| crate::ws::handle(socket, s, id)))
}

// ---------- push tokens ----------

#[derive(Deserialize)]
struct PushToken {
    token: String,
}

async fn add_push_token(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<PushToken>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let token = body.token.trim().to_string();
    let valid = (token.starts_with("ExponentPushToken[") || token.starts_with("ExpoPushToken["))
        && token.ends_with(']');
    if !valid || token.len() > 200 {
        return Err(ApiError::BadRequest(
            "expected an Expo push token like ExponentPushToken[...]".into(),
        ));
    }
    s.db.call(move |c| db::add_push_token(c, &token)).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "push_enabled": s.pusher.enabled() })),
    ))
}

async fn remove_push_token(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<PushToken>,
) -> ApiResult<Json<Value>> {
    let token = body.token.trim().to_string();
    let removed = s.db.call(move |c| db::remove_push_token(c, &token)).await?;
    Ok(Json(json!({ "ok": true, "removed": removed })))
}
