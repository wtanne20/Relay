//! REST routes, authentication middleware and the shared application state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{
    DefaultBodyLimit, FromRequest, Path as UrlPath, Query, Request, State, WebSocketUpgrade,
};
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
    /// New versions from GitHub releases.
    pub updates: Arc<crate::updates::Updater>,
    /// Paired device hosts (phone Proot Linux and friends).
    pub devices: Arc<crate::device::DeviceRegistry>,
    /// Devices connected to this brain right now (they dial in over Tailscale).
    pub links: Arc<crate::link::LinkHub>,
    /// The current 6-digit code a new device can pair with.
    pub pair_code: std::sync::Mutex<Option<PairCode>>,
    /// The built web UI bundle to serve at `/` (None if not bundled).
    pub web_root: Option<std::path::PathBuf>,
}

/// A short-lived code that lets one new device pair with this brain.
#[derive(Debug, Clone)]
pub struct PairCode {
    pub code: String,
    pub expires: chrono::DateTime<chrono::Utc>,
    /// Wrong guesses so far; the code is dropped after a few.
    pub misses: u32,
}

/// How long a pairing code stays valid, and how many wrong tries it allows.
const PAIR_CODE_MINUTES: i64 = 10;
const PAIR_CODE_MISSES: u32 = 5;

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
        .route("/stats", get(stats))
        .route("/model/restart", post(model_restart))
        .route("/model/load", post(model_load))
        .route("/model/unload", post(model_unload))
        .route("/models", get(list_models))
        .route("/models/tags", axum::routing::put(set_model_tags))
        .route("/models/import", post(import_model))
        .route("/models/move", post(move_model))
        .route("/libraries", get(list_libraries).post(add_library))
        .route("/libraries/default", post(set_default_library))
        .route("/libraries/remove", post(remove_library))
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
        .route("/archived", get(archived))
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
        .route("/projects/{id}/download", get(download_project_file))
        .route(
            "/sessions/{id}",
            get(get_session).patch(patch_session).delete(delete_session),
        )
        .route("/sessions/{id}/messages", get(list_messages))
        .route(
            "/sessions/{id}/prompt",
            post(send_prompt).layer(DefaultBodyLimit::max(PROMPT_BODY_LIMIT)),
        )
        .route("/attachments/{name}", get(get_attachment))
        .route("/updates", get(check_updates))
        .route("/updates/install", post(install_update))
        .route("/sessions/{id}/stop", post(stop_task))
        .route("/sessions/{id}/approval", post(answer_approval))
        .route("/sessions/{id}/ws", get(ws_upgrade))
        .route(
            "/push-tokens",
            post(add_push_token).delete(remove_push_token),
        )
        .route("/devices", get(list_devices).post(add_device))
        .route("/devices/pair-code", post(new_pair_code))
        .route("/devices/claim", post(claim_device))
        .route("/devices/link", get(device_link))
        .route("/devices/{id}", get(get_device).patch(rename_device).delete(remove_device))
        .route("/devices/{id}/ping", post(ping_device))
        .route("/devices/{id}/browse", get(browse_device))
        .route("/hello", get(hello))
        .fallback(|| async { ApiError::NotFound("route") })
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        // Outside auth so browser preflight requests (which carry no token) get an answer.
        .layer(cors())
        .layer(tower_http::trace::TraceLayer::new_for_http())
        // Outermost: serves the web UI to browser navigations and static
        // assets (no token, same origin as the API), passing API calls through.
        .layer(middleware::from_fn_with_state(state.clone(), crate::ui::ui_middleware))
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
    // Open endpoints: discovery, pairing with a code, and the device link
    // (which checks the device's own key).
    if matches!(req.uri().path(), "/hello" | "/devices/claim" | "/devices/link") {
        return next.run(req).await;
    }
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
    let host = crate::link::hostname();
    Ok(Json(json!({
        "name": "relay",
        // This computer runs the model, the agent and the history.
        "role": "brain",
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
    "routing",
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
        "routing": c.routing,
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
        routing: crate::config::RoutingConfig,
    }
    let mut next: Sections = serde_json::from_value(view)
        .map_err(|e| ApiError::BadRequest(format!("invalid settings: {e}")))?;
    // Picking a model in Settings makes it the default for chats too.
    if next.llama.model_path != current.llama.model_path {
        next.llama.default_model = next.llama.model_path.clone();
    }
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
    next.routing.quick_model = next.routing.quick_model.trim().to_string();
    next.routing.heavy_model = next.routing.heavy_model.trim().to_string();
    for (label, path, old) in [
        ("quick model", &next.routing.quick_model, &current.routing.quick_model),
        ("big model", &next.routing.heavy_model, &current.routing.heavy_model),
    ] {
        if !path.is_empty() && path != old && !Path::new(path).is_file() {
            return Err(ApiError::BadRequest(format!(
                "the {label} must be an existing .gguf file on the PC: {path}"
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
    // The unload timer applies without restarting the model.
    let restart = crate::config::LlamaConfig {
        idle_unload_minutes: current.llama.idle_unload_minutes,
        ..next.llama.clone()
    } != current.llama;
    let updated = s.cfg.update(move |c| {
        c.agent = next.agent;
        c.sampling = next.sampling;
        c.llama = next.llama;
        c.push = next.push;
        c.disabled_skills = next.disabled_skills;
        c.projects_folder = projects_folder;
        c.models_folder = models_folder;
        c.routing = next.routing;
    })?;
    if restart {
        s.model.settings_changed().await;
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
                "vision": crate::images::mmproj_for(Path::new(&m.path)).is_some(),
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
    let (files, projector) = crate::hub::files_and_projector(&q.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({
        "repo": q.repo,
        "files": files,
        "vision": projector.is_some(),
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
    /// Library to save into; the default library when absent.
    #[serde(default)]
    library: Option<String>,
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
    let (files, projector) = crate::hub::files_and_projector(&body.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let model = files
        .into_iter()
        .find(|f| f.file == body.file)
        .ok_or(ApiError::NotFound("model file"))?;
    let models_dir = match body.library.as_deref().map(str::trim) {
        Some(l) if !l.is_empty() => find_library(&s, l)?.0,
        _ => relay_models_dir(&s),
    };
    let d = s
        .downloads
        .start(&models_dir, &body.repo, model)
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    // The vision file comes along, so the model can read pictures.
    if let Some(p) = projector {
        let folder = body
            .repo
            .split('/')
            .fold(models_dir.clone(), |acc, part| acc.join(part));
        let have = std::fs::read_dir(&folder).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("mmproj")
            })
        });
        if !have && let Err(e) = s.downloads.start(&models_dir, &body.repo, p) {
            tracing::info!("vision file not downloaded: {e:#}");
        }
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "download": d }))))
}

/// Where Relay saves every model it downloads or copies: its own models
/// folder (in its data folder unless moved in Settings), never another app's.
fn relay_models_dir(s: &AppState) -> std::path::PathBuf {
    s.cfg.read(|l| l.relay_models_dir())
}

// ---------- model libraries ----------

/// (built-in folder, default folder for downloads, extra libraries)
fn library_paths(s: &AppState) -> (PathBuf, PathBuf, Vec<PathBuf>) {
    s.cfg.read(|l| {
        (
            l.data_dir.join("models"),
            l.relay_models_dir(),
            l.config
                .llama
                .models_dirs
                .iter()
                .map(|p| l.resolve(p))
                .collect(),
        )
    })
}

async fn libraries_now(s: &AppState) -> ApiResult<Vec<crate::libraries::Library>> {
    let (builtin, default, extra) = library_paths(s);
    Ok(
        tokio::task::spawn_blocking(move || crate::libraries::list(&builtin, &default, &extra))
            .await
            .map_err(|e| anyhow::anyhow!("listing libraries failed: {e}"))?,
    )
}

async fn list_libraries(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let libraries = libraries_now(&s).await?;
    let current = s.cfg.read(|l| l.config.llama.model_path.clone());
    Ok(Json(
        json!({ "libraries": libraries, "current_model": current }),
    ))
}

#[derive(Deserialize)]
struct LibraryPath {
    path: String,
    /// Also make it the library new downloads go to.
    #[serde(default)]
    make_default: bool,
}

/// Adds a folder (created if needed) as a model library.
async fn add_library(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<LibraryPath>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let wanted = body.path.trim().trim_matches('"');
    if crate::libraries::is_other_apps_folder(Path::new(wanted)) {
        return Err(ApiError::BadRequest(
            "that folder belongs to another app. Pick a folder of Relay's own, like W:\\Relay\\models".into(),
        ));
    }
    let path = prepare_folder(wanted).await?;
    let p = PathBuf::from(&path);
    let (builtin, default, extra) = library_paths(&s);
    for lib in std::iter::once(&builtin)
        .chain(Some(&default))
        .chain(extra.iter())
    {
        if crate::libraries::same_path(lib, &p) {
            return Err(ApiError::Conflict(
                "that folder is already a library".into(),
            ));
        }
        if crate::libraries::contains(lib, &p) || crate::libraries::contains(&p, lib) {
            return Err(ApiError::BadRequest(format!(
                "libraries can't be inside each other ({} is already a library)",
                lib.display()
            )));
        }
    }
    let make_default = body.make_default;
    let stored = path.clone();
    s.cfg.update(move |c| {
        c.llama.models_dirs.push(stored.clone());
        if make_default {
            c.models_folder = stored;
        }
    })?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "libraries": libraries_now(&s).await? })),
    ))
}

#[derive(Deserialize)]
struct OnlyPath {
    path: String,
}

fn find_library(s: &AppState, path: &str) -> ApiResult<(PathBuf, bool)> {
    let (builtin, default, extra) = library_paths(s);
    let p = Path::new(path.trim().trim_matches('"'));
    std::iter::once(builtin.clone())
        .chain(Some(default))
        .chain(extra)
        .find(|lib| crate::libraries::same_path(lib, p))
        .map(|lib| {
            let is_builtin = crate::libraries::same_path(&lib, &builtin);
            (lib, is_builtin)
        })
        .ok_or(ApiError::NotFound("library"))
}

/// New downloads and copies go to this library.
async fn set_default_library(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<OnlyPath>,
) -> ApiResult<Json<Value>> {
    let (lib, builtin) = find_library(&s, &body.path)?;
    let value = if builtin {
        String::new()
    } else {
        lib.to_string_lossy().into_owned()
    };
    s.cfg.update(move |c| {
        // Keep the old default listed as a library.
        let old = c.models_folder.trim().to_string();
        if !old.is_empty() && !c.llama.models_dirs.iter().any(|d| d.trim() == old) {
            c.llama.models_dirs.push(old);
        }
        c.models_folder = value;
    })?;
    Ok(Json(json!({ "libraries": libraries_now(&s).await? })))
}

/// Forgets a library. Its folder and the models in it stay on the disk.
async fn remove_library(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<OnlyPath>,
) -> ApiResult<Json<Value>> {
    let (lib, builtin) = find_library(&s, &body.path)?;
    if builtin {
        return Err(ApiError::BadRequest(
            "Relay's built-in library can't be removed".into(),
        ));
    }
    let current = s.cfg.read(|l| l.config.llama.model_path.clone());
    if !current.is_empty() && crate::libraries::contains(&lib, Path::new(&current)) {
        return Err(ApiError::Conflict(
            "the model in use is in this library. Choose another model or move it first".into(),
        ));
    }
    let busy = s.downloads.list().into_iter().any(|d| {
        d.state == crate::hub::DownloadState::Downloading
            && crate::libraries::contains(&lib, Path::new(&d.dest))
    });
    if busy {
        return Err(ApiError::Conflict(
            "wait for the download or move into this library to finish".into(),
        ));
    }
    s.cfg.update(move |c| {
        c.llama
            .models_dirs
            .retain(|d| !crate::libraries::same_path(Path::new(d.trim()), &lib));
        if !c.models_folder.trim().is_empty()
            && crate::libraries::same_path(Path::new(c.models_folder.trim()), &lib)
        {
            c.models_folder = String::new();
        }
    })?;
    Ok(Json(json!({ "libraries": libraries_now(&s).await? })))
}

#[derive(Deserialize)]
struct MoveModel {
    /// The model file (the first part of a split model).
    path: String,
    /// The library to move it to.
    library: String,
}

/// Moves a model to another library: copies every file, checks the copies,
/// points chats (and the loaded model) at the new place, then removes the
/// originals. Only models inside Relay's libraries can be moved.
async fn move_model(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<MoveModel>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let src = PathBuf::from(body.path.trim().trim_matches('"'));
    let is_gguf = src
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"));
    if !is_gguf || !src.is_file() {
        return Err(ApiError::BadRequest(format!(
            "not a model file: {}",
            src.display()
        )));
    }
    let (target, _) = find_library(&s, &body.library)?;
    let libs = libraries_now(&s).await?;
    let Some(from_lib) = libs
        .iter()
        .map(|l| PathBuf::from(&l.path))
        .find(|l| crate::libraries::contains(l, &src))
    else {
        return Err(ApiError::BadRequest(
            "only models in Relay's libraries can be moved".into(),
        ));
    };
    if crate::libraries::same_path(&from_lib, &target) {
        return Err(ApiError::BadRequest(
            "the model is already in that library".into(),
        ));
    }
    let parts = crate::libraries::model_parts(&src);
    let mut files = Vec::new();
    for part in &parts {
        let canon_lib = dunce::canonicalize(&from_lib).unwrap_or(from_lib.clone());
        let canon_part = dunce::canonicalize(part).unwrap_or(part.clone());
        let rel = canon_part
            .strip_prefix(&canon_lib)
            .map_err(|_| ApiError::BadRequest("the model is outside its library".into()))?
            .to_path_buf();
        files.push((part.clone(), target.join(rel)));
    }
    let size = crate::libraries::model_size(&src);
    let room = {
        let t = target.clone();
        tokio::task::spawn_blocking(move || crate::libraries::disk(&t))
            .await
            .ok()
            .flatten()
    };
    if let Some(d) = room
        && d.free_bytes < size + 512 * 1024 * 1024
    {
        return Err(ApiError::BadRequest(format!(
            "not enough space on {}: the model needs {:.1} GB and {:.1} GB is free",
            d.root,
            size as f64 / 1e9,
            d.free_bytes as f64 / 1e9
        )));
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let d = s
        .downloads
        .move_files(&name, files.clone())
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;

    let id = d.id.clone();
    let st = s.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match st.downloads.state(&id) {
                Some(crate::hub::DownloadState::Done) => break,
                Some(crate::hub::DownloadState::Downloading) => continue,
                _ => return,
            }
        }
        let old = src.to_string_lossy().into_owned();
        let new = files[0].1.to_string_lossy().into_owned();
        let (o, n) = (old.clone(), new.clone());
        if let Err(e) = st.db.call(move |c| db::replace_model_path(c, &o, &n)).await {
            tracing::warn!("could not point chats at the moved model: {e:#}");
        }
        let in_use = |st: &AppState| {
            let cur = st.cfg.get().llama.model_path;
            !cur.is_empty() && crate::libraries::same_path(Path::new(&cur), Path::new(&old))
        };
        if in_use(&st) {
            while st.agent.any_running() {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            let (o, n) = (old.clone(), new.clone());
            match st.cfg.update(move |c| c.llama.model_moved(&o, &n)) {
                Ok(_) => st.model.switch_to(&new).await,
                Err(e) => {
                    tracing::warn!("could not switch to the moved model: {e:#}");
                    return;
                }
            }
        }
        // The old files may stay locked for a moment after llama-server exits.
        for (from, _) in &files {
            for attempt in 0..20 {
                match tokio::fs::remove_file(from).await {
                    Ok(()) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) if attempt == 19 => {
                        tracing::warn!("could not remove {} after moving it: {e}", from.display())
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_secs(3)).await,
                }
            }
        }
        // Tidy up the now-empty <publisher>\<model> folders.
        if let Some(mut dir) = src.parent().map(Path::to_path_buf) {
            while crate::libraries::contains(&from_lib, &dir) && std::fs::remove_dir(&dir).is_ok() {
                match dir.parent() {
                    Some(p) => dir = p.to_path_buf(),
                    None => break,
                }
            }
        }
        tracing::info!("moved {old} to {new}");
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "download": d }))))
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
        let (o, n) = (old.clone(), new.clone());
        match st.cfg.update(move |c| c.llama.model_moved(&o, &n)) {
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

// ---------- updates ----------

#[derive(Deserialize)]
struct UpdateQuery {
    #[serde(default)]
    refresh: bool,
}

/// The latest release and whether it's newer than this Relay. A failed
/// check isn't an error: the answer says why instead.
async fn check_updates(State(s): State<AppState>, Query(q): Query<UpdateQuery>) -> Json<Value> {
    let current = crate::updates::CURRENT;
    let (latest, error) = match s.updates.latest(q.refresh).await {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    Json(json!({
        "current": current,
        "available": latest.as_ref().is_some_and(|r| crate::updates::newer(&r.version, current)),
        "latest": latest,
        "error": error,
        "can_install": cfg!(windows),
        "install": s.updates.install_state(),
    }))
}

/// Downloads the new Windows installer, checks it and runs it.
async fn install_update(State(s): State<AppState>) -> ApiResult<(StatusCode, Json<Value>)> {
    let dir = s.data_dir.join("updates");
    let release = s
        .updates
        .install(dir)
        .await
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "version": release.version, "install": s.updates.install_state() })),
    ))
}

/// Room for a prompt with a few large pictures as base64.
const PROMPT_BODY_LIMIT: usize = 120 * 1024 * 1024;

async fn get_attachment(
    State(s): State<AppState>,
    UrlPath(name): UrlPath<String>,
) -> ApiResult<Response> {
    let picture = crate::images::valid_name(&name);
    if !picture && !crate::files::valid_name(&name) {
        return Err(ApiError::NotFound("attachment"));
    }
    let path = crate::images::dir(&s.data_dir).join(&name);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound("attachment"))?;
    if !picture {
        // Other files are only offered as downloads, never shown as pages.
        return Ok((
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::CONTENT_DISPOSITION, "attachment"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            bytes,
        )
            .into_response());
    }
    Ok((
        [
            (header::CONTENT_TYPE, crate::images::mime(&name)),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response())
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
    let mut v = serde_json::to_value(s.model.status()).unwrap_or(Value::Null);
    let (path, default) = s.cfg.read(|l| {
        (
            l.config.llama.model_path.clone(),
            l.config.llama.default_model().to_string(),
        )
    });
    v["vision"] = json!(crate::images::mmproj_for(Path::new(&path)).is_some());
    v["default_model"] = json!(default);
    Json(v)
}

/// Performance: how fast each model writes, and memory use on the brain and
/// on every linked computer that is online.
async fn stats(State(s): State<AppState>) -> Json<Value> {
    let status = s.model.status();
    // Relay only knows the process it launched.
    let pid = if status.external { None } else { status.pid };
    let (memory, gpus) = tokio::join!(
        tokio::task::spawn_blocking(move || crate::stats::memory(pid)),
        crate::stats::gpu_memory()
    );
    let devices = s.devices.list().await.unwrap_or_default();
    let checks = devices.iter().map(|d| {
        let online = device_online(&s, d);
        let client = crate::device::DeviceClient::for_device(d, &s.links);
        async move {
            let memory = if online {
                tokio::time::timeout(std::time::Duration::from_secs(4), client.info())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .and_then(|i| i.get("memory").cloned())
            } else {
                None
            };
            json!({
                "id": d.id,
                "name": d.name,
                "os": d.os,
                "connected": online,
                "memory": memory,
            })
        }
    });
    let devices = futures_util::future::join_all(checks).await;
    Json(json!({
        "brain": {
            "name": crate::link::hostname(),
            "os": std::env::consts::OS,
            "model": {
                "state": status.state,
                "file": Path::new(&status.model_path).file_name().map(|n| n.to_string_lossy().into_owned()),
            },
            "memory": memory.ok().flatten(),
            "gpus": gpus,
        },
        "speeds": s.model.speeds().list(),
        "devices": devices,
    }))
}

async fn model_restart(State(s): State<AppState>) -> (StatusCode, Json<Value>) {
    s.model.restart().await;
    (StatusCode::ACCEPTED, Json(json!({ "ok": true })))
}

async fn model_load(State(s): State<AppState>) -> (StatusCode, Json<Value>) {
    s.model.wake();
    (StatusCode::ACCEPTED, Json(json!({ "ok": true })))
}

async fn model_unload(State(s): State<AppState>) -> ApiResult<(StatusCode, Json<Value>)> {
    if s.model.in_use() {
        return Err(ApiError::Conflict(
            "a chat is working; the model unloads once it's done".into(),
        ));
    }
    s.model.unload().await;
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

// ---------- projects ----------

#[derive(Deserialize)]
struct CreateProject {
    name: Option<String>,
    /// An existing folder. When empty, a folder named after the project is
    /// created inside the projects folder from the settings.
    #[serde(default)]
    path: Option<String>,
    /// The paired device the folder is on. Empty: this computer (the brain).
    #[serde(default)]
    device_id: Option<String>,
}

/// Checks that a folder setting is an absolute path and creates it if it
/// doesn't exist yet. Returns the canonical path.
async fn prepare_folder(path: &str) -> ApiResult<String> {
    let trimmed = path.trim().trim_matches('"').to_string();
    if !Path::new(&trimmed).is_absolute() {
        return Err(ApiError::BadRequest(format!(
            "the folder must be a full path (for example W:\\Relay\\models): {trimmed}"
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
    /// A paired device this project's tools run on; "" runs them locally again.
    device_id: Option<String>,
    /// Hide the project (and its chats) from the lists, or restore it.
    archived: Option<bool>,
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

/// `Some("")` clears the binding; otherwise the id must be a paired device.
async fn validate_device(
    s: &AppInner,
    device: Option<String>,
) -> ApiResult<Option<Option<String>>> {
    let Some(d) = device else { return Ok(None) };
    let d = d.trim().to_string();
    if d.is_empty() {
        return Ok(Some(None));
    }
    if s.devices.get(&d).await?.is_none() {
        return Err(ApiError::BadRequest(format!("unknown device: {d}")));
    }
    Ok(Some(Some(d)))
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
    let devices = s.devices.list().await?;
    let list: Vec<Value> = projects.iter().map(|p| with_location(&s, p, &devices)).collect();
    Ok(Json(json!({ "projects": list })))
}

async fn create_project(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<CreateProject>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if let Some(device_id) = body.device_id.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        return create_device_project(&s, device_id, body.name.as_deref(), body.path.as_deref())
            .await;
    }
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
    let devices = s.devices.list().await?;
    Ok((StatusCode::CREATED, Json(json!({ "project": with_location(&s, &project, &devices) }))))
}

async fn get_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let project =
        s.db.call(move |c| db::get_project(c, &id))
            .await?
            .ok_or(ApiError::NotFound("project"))?;
    let devices = s.devices.list().await?;
    Ok(Json(json!({ "project": with_location(&s, &project, &devices) })))
}

/// A project's JSON plus `location`: which machine its folder is on (this
/// brain, or a paired device and whether that device is connected).
fn with_location(s: &AppInner, p: &db::Project, devices: &[crate::device::Device]) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    let location = match p.device_id.as_deref().filter(|d| !d.is_empty()) {
        None if p.is_chats() => json!({ "kind": "none" }),
        None => json!({
            "kind": "brain",
            "name": crate::link::hostname(),
            "online": true,
        }),
        Some(id) => match devices.iter().find(|d| d.id == id) {
            Some(d) => json!({
                "kind": "device",
                "device_id": d.id,
                "name": d.name,
                "online": device_online(s, d),
            }),
            None => json!({ "kind": "device", "device_id": id, "name": "removed device", "online": false }),
        },
    };
    if let Some(obj) = v.as_object_mut() {
        obj.insert("location".into(), location);
    }
    v
}

/// Makes a project whose folder is on a paired device.
async fn create_device_project(
    s: &AppInner,
    device_id: &str,
    name: Option<&str>,
    path: Option<&str>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let d = s
        .devices
        .get(device_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest(format!("unknown device: {device_id}")))?;
    let dc = crate::device::DeviceClient::for_device(&d, &s.links);
    let folder = path.map(str::trim).map(|p| p.trim_matches('"')).unwrap_or_default();
    let folder = if folder.is_empty() { d.root.clone() } else { folder.to_string() };
    if folder.is_empty() {
        return Err(ApiError::BadRequest("choose a folder on the device".into()));
    }
    // The device checks the folder exists and sits inside what it shares.
    let dc = dc.in_folder(Some(&folder));
    let info = dc.info().await.map_err(|e| {
        ApiError::BadRequest(format!("{} is not reachable: {e:#}", d.name))
    })?;
    dc.list_entries(".").await.map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let _ = info;
    let name = match name.map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => folder
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(&d.name)
            .to_string(),
    };
    let dev = d.id.clone();
    let f = folder.clone();
    let project = s
        .db
        .call(move |c| {
            let p = db::create_project(c, &name, &f)?;
            db::update_project(c, &p.id, None, None, None, None, Some(Some(dev.as_str())))
        })
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    let devices = s.devices.list().await?;
    Ok((StatusCode::CREATED, Json(json!({ "project": with_location(s, &project, &devices) }))))
}

/// Serves a file from a project's folder so the app can download it (for
/// example an APK or zip the agent built). The path is confined to the
/// project folder by the same sandbox the agent's tools use, so it can't
/// reach anything outside.
async fn download_project_file(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<DownloadQuery>,
) -> ApiResult<Response> {
    let project = s
        .db
        .call(move |c| db::get_project(c, &id))
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    let path = q.path.trim().trim_matches('"').to_string();
    if path.is_empty() || path == "." {
        return Err(ApiError::BadRequest("a file path is required".into()));
    }
    // Device projects: fetch the file from the device (confined there).
    if let Some(dev) = project.device_id.as_deref().filter(|d| !d.is_empty()) {
        let d = s.devices.get(dev).await?.ok_or(ApiError::NotFound("device"))?;
        let folder = Some(project.path.as_str()).filter(|p| !p.trim().is_empty());
        let dc = crate::device::DeviceClient::for_device(&d, &s.links).in_folder(folder);
        let bytes = dc
            .read_bytes(&path)
            .await
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("file")
            .to_string();
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            file_mime(&name)
                .parse()
                .unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
        );
        headers.insert(header::CONTENT_LENGTH, bytes.len().to_string().parse().unwrap());
        let dispo = format!("attachment; filename=\"{}\"", name.replace('"', ""));
        headers.insert(header::CONTENT_DISPOSITION, dispo.parse().unwrap());
        headers.insert(header::CACHE_CONTROL, "private, max-age=0".parse().unwrap());
        return Ok((StatusCode::OK, headers, Body::from(bytes)).into_response());
    }
    let sandbox = crate::tools::Sandbox::new(Path::new(&project.path), &[])
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let file = sandbox
        .resolve_existing(&path, crate::tools::Access::Read)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if !file.is_file() {
        return Err(ApiError::NotFound("file"));
    }
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let mime = file_mime(&name);
    let len = tokio::fs::metadata(&file).await.map(|m| m.len()).unwrap_or(0);
    let bytes = tokio::fs::read(&file).await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let body = Body::from(bytes);
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        mime.parse().unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
    );
    if len > 0 {
        headers.insert(header::CONTENT_LENGTH, len.to_string().parse().unwrap());
    }
    // Tell the phone/browser to save it rather than try to display it.
    let dispo = format!("attachment; filename=\"{}\"", name.replace('"', ""));
    headers.insert(header::CONTENT_DISPOSITION, dispo.parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "private, max-age=0".parse().unwrap());
    Ok((StatusCode::OK, headers, body).into_response())
}

#[derive(Deserialize)]
struct DownloadQuery {
    path: String,
}

/// A content type for the common build-artifact extensions we serve.
fn file_mime(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "apk" => "application/vnd.android.package-archive",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "exe" => "application/vnd.microsoft.portable-executable",
        "msi" => "application/x-msi",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "txt" | "md" => "text/plain",
        _ => "application/octet-stream",
    }
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
    if id == db::CHATS_PROJECT_ID
        && (name.is_some() || body.path.is_some() || body.archived.is_some())
    {
        return Err(ApiError::BadRequest(
            "plain chats have no name or folder to change".into(),
        ));
    }
    let archived = body.archived;
    // A project bound to a device keeps its folder on that device, so the
    // PC can only check that it looks like an absolute path.
    let path = match body.path.as_deref() {
        Some(p) => {
            let trimmed = p.trim().trim_matches('"').to_string();
            if trimmed.is_empty() {
                return Err(ApiError::BadRequest("path is required".into()));
            }
            if !Path::new(&trimmed).is_absolute() {
                return Err(ApiError::BadRequest(format!(
                    "path must be absolute: {trimmed}"
                )));
            }
            if Path::new(&trimmed).is_dir() {
                validate_folder(p)?;
            }
            Some(trimmed)
        }
        None => None,
    };
    let model = validate_model(body.model)?;
    let device_id = validate_device(&s, body.device_id).await?;
    let instructions = body.instructions;
    if instructions.as_ref().is_some_and(|i| i.len() > 50_000) {
        return Err(ApiError::BadRequest(
            "instructions are too long (max 50,000 bytes)".into(),
        ));
    }
    let project =
        s.db.call(move |c| {
            if let Some(a) = archived {
                db::set_project_archived(c, &id, a)?;
            }
            db::update_project(
                c,
                &id,
                name.as_deref(),
                path.as_deref(),
                instructions.as_deref(),
                model.as_ref().map(|m| m.as_deref()),
                device_id.as_ref().map(|d| d.as_deref()),
            )
        })
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    let devices = s.devices.list().await?;
    Ok(Json(json!({ "project": with_location(&s, &project, &devices) })))
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
    /// Hide the chat from the lists, or restore it.
    archived: Option<bool>,
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

/// Archived projects and chats, to restore or delete.
async fn archived(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let (projects, sessions) =
        s.db.call(|c| Ok((db::archived_projects(c)?, db::archived_sessions(c)?)))
            .await?;
    Ok(Json(json!({ "projects": projects, "sessions": sessions })))
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
            if let Some(a) = body.archived {
                db::set_session_archived(c, &id, a)?;
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
    #[serde(default)]
    content: String,
    /// Pictures as data URLs.
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    files: Vec<crate::files::NewFile>,
}

async fn send_prompt(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<Prompt>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let message = s
        .agent
        .start(&id, body.content, body.images, body.files)
        .await?;
    // Carrying on with an archived chat brings it back to the lists.
    let sid = id.clone();
    s.db.call(move |c| db::set_session_archived(c, &sid, false))
        .await?;
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

// ---------- devices ----------
//
// A device host (`relay host` on a phone or other machine) is paired into
// the PC's device list. The PC stores its URL and token; the agent can then
// work in the device's folder through the same tool interface it uses
// locally.

#[derive(Deserialize)]
struct AddDevice {
    id: Option<String>,
    name: String,
    url: String,
    token: String,
    os: Option<String>,
    root: Option<String>,
    shell: Option<String>,
}

/// Whether the brain can reach a device right now: a linked device while its
/// link is up, a direct one if it answered within the last two minutes.
fn device_online(s: &AppInner, d: &crate::device::Device) -> bool {
    if d.is_link() {
        s.links.is_connected(&d.id)
    } else {
        s.devices.is_online(d, 120)
    }
}

fn device_json(s: &AppInner, d: &crate::device::Device) -> Value {
    let link = s.links.get(&d.id);
    json!({
        "id": d.id,
        "name": d.name,
        "hostname": d.hostname,
        "url": if d.is_link() { Value::Null } else { Value::String(d.url.clone()) },
        // "link": the device connects to the brain; "direct": the brain calls it.
        "connection": if d.is_link() { "link" } else { "direct" },
        "address": link.as_ref().map(|l| l.remote.clone()),
        "os": d.os,
        "arch": d.arch,
        "root": d.root,
        "shell": d.shell,
        "last_seen": d.last_seen,
        "connected": device_online(s, d),
    })
}

async fn list_devices(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let devices = s.devices.list().await?;
    let list = devices.iter().map(|d| device_json(&s, d)).collect::<Vec<_>>();
    Ok(Json(json!({ "devices": list })))
}

/// Starts (or restarts) pairing: a 6-digit code, good for ten minutes.
async fn new_pair_code(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let n: u32 = rand::random_range(0..1_000_000);
    let code = format!("{n:06}");
    let expires = chrono::Utc::now() + chrono::Duration::minutes(PAIR_CODE_MINUTES);
    *s.pair_code.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(PairCode { code: code.clone(), expires, misses: 0 });
    let (bind,) = s.cfg.read(|l| (l.config.bind.clone(),));
    let url = setup::pairing_url(s.tailscale.as_ref(), crate::port_of(&bind));
    Ok(Json(json!({
        "code": code,
        "expires_at": expires.to_rfc3339(),
        "brain_url": url,
        "brain_name": crate::link::hostname(),
        "tailscale": s.tailscale.is_some(),
    })))
}

#[derive(Deserialize)]
struct ClaimBody {
    code: String,
    name: Option<String>,
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    root: String,
    /// Re-pairing the same machine keeps its id, so its projects stay bound.
    previous_id: Option<String>,
}

/// A device trades the code for its own key (and the brain's API token, so
/// its window can open the chats). No other auth: the code is the proof.
async fn claim_device(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<ClaimBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    {
        let mut slot = s.pair_code.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pc) = slot.as_mut() else {
            return Err(ApiError::BadRequest(
                "no pairing code is active. On the brain, open Devices and choose Add a computer to get one."
                    .into(),
            ));
        };
        if chrono::Utc::now() > pc.expires {
            *slot = None;
            return Err(ApiError::BadRequest("that code expired; get a new one on the brain".into()));
        }
        if !constant_time_eq(body.code.trim().as_bytes(), pc.code.as_bytes()) {
            pc.misses += 1;
            if pc.misses >= PAIR_CODE_MISSES {
                *slot = None;
                return Err(ApiError::BadRequest(
                    "too many wrong codes; get a new one on the brain".into(),
                ));
            }
            return Err(ApiError::BadRequest("that code is not right; check it and try again".into()));
        }
        // One code pairs one device.
        *slot = None;
    }
    let existing = match body.previous_id.as_deref().filter(|i| !i.trim().is_empty()) {
        Some(id) => s.devices.get(id).await?.filter(|d| d.is_link()),
        None => None,
    };
    let hostname = body.hostname.trim().to_string();
    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| existing.as_ref().map(|d| d.name.clone()))
        .unwrap_or_else(|| if hostname.is_empty() { "computer".into() } else { hostname.clone() });
    let id = existing
        .as_ref()
        .map(|d| d.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let key = crate::config::generate_token();
    let d = crate::device::Device {
        id: id.clone(),
        name,
        url: crate::device::LINK_URL.into(),
        token: key.clone(),
        os: body.os.trim().to_string(),
        root: body.root.trim().to_string(),
        shell: String::new(),
        last_seen: crate::db::now(),
        arch: body.arch.trim().to_string(),
        hostname,
    };
    // A new key replaces the old one; drop any link still using it.
    s.links.disconnect(&id);
    s.devices.upsert(&d).await?;
    tracing::info!("paired device {} ({})", d.name, d.id);
    let token = s.cfg.read(|l| l.config.token.clone());
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "device_id": id,
            "device_key": key,
            "api_token": token,
            "brain_name": crate::link::hostname(),
        })),
    ))
}

/// A paired device opens its link here (WebSocket, with its own key).
async fn device_link(
    State(s): State<AppState>,
    extensions: axum::http::Extensions,
    headers: header::HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let remote = extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_default();
    let key = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if key.is_empty() {
        return Err(ApiError::Unauthorized);
    }
    let Some(d) = s.devices.by_key(&key).await? else {
        return Err(ApiError::Unauthorized);
    };
    let links = Arc::clone(&s.links);
    let registry = Arc::clone(&s.devices);
    let shutdown = s.shutdown.clone();
    let id = d.id.clone();
    let st = s.clone();
    Ok(ws.on_upgrade(move |socket| async move {
        let touch_id = id.clone();
        let touch = move || registry.touch(&touch_id);
        // Learn the device's shell and folder once it is up.
        let refresh_id = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            refresh_device_info(&st, &refresh_id).await;
        });
        links.serve(socket, id, remote, shutdown, touch).await;
    }))
}

/// Asks a connected device for its details and saves them.
async fn refresh_device_info(s: &AppInner, id: &str) {
    let Ok(Some(mut d)) = s.devices.get(id).await else { return };
    let dc = crate::device::DeviceClient::for_device(&d, &s.links);
    let Ok(info) = dc.info().await else { return };
    let get = |k: &str| info.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(v) = get("os") { d.os = v; }
    if let Some(v) = get("arch") { d.arch = v; }
    if let Some(v) = get("root") { d.root = v; }
    if let Some(v) = get("shell") { d.shell = v; }
    if let Some(v) = get("hostname") { d.hostname = v; }
    d.last_seen = crate::db::now();
    let _ = s.devices.upsert(&d).await;
}

/// Unauthenticated: lets a device on the tailnet find this brain.
async fn hello() -> Json<Value> {
    Json(json!({
        "relay": "brain",
        "name": crate::link::hostname(),
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    }))
}

#[derive(Deserialize)]
struct RenameDevice {
    name: String,
}

async fn rename_device(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<RenameDevice>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".into()));
    }
    let mut d = s.devices.get(&id).await?.ok_or(ApiError::NotFound("device"))?;
    d.name = name.to_string();
    s.devices.upsert(&d).await?;
    Ok(Json(json!({ "device": device_json(&s, &d) })))
}

#[derive(Deserialize)]
struct BrowseQuery {
    #[serde(default)]
    path: String,
}

/// Lists the sub-folders of a folder on a device (for choosing a project
/// folder). An empty path lists what the device shares.
async fn browse_device(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<BrowseQuery>,
) -> ApiResult<Json<Value>> {
    let d = s.devices.get(&id).await?.ok_or(ApiError::NotFound("device"))?;
    let dc = crate::device::DeviceClient::for_device(&d, &s.links);
    let path = if q.path.trim().is_empty() { d.root.clone() } else { q.path.trim().to_string() };
    let entries = dc
        .list_entries(if path.is_empty() { "." } else { &path })
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let folders: Vec<&str> = entries
        .iter()
        .filter(|e| e.get("dir").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|e| e.get("name").and_then(Value::as_str))
        .filter(|n| !n.starts_with('.'))
        .collect();
    Ok(Json(json!({ "path": path, "root": d.root, "folders": folders })))
}

async fn add_device(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<AddDevice>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let url = body.url.trim().trim_end_matches('/');
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ApiError::BadRequest(
            "url must start with http:// or https://".into(),
        ));
    }
    if body.token.trim().is_empty() {
        return Err(ApiError::BadRequest("token is required".into()));
    }
    let id = body
        .id
        .as_deref()
        .filter(|i| !i.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut d = crate::device::Device {
        id: id.clone(),
        name: body.name.trim().to_string(),
        url: url.to_string(),
        token: body.token.trim().to_string(),
        os: body.os.unwrap_or_default(),
        root: body.root.unwrap_or_default(),
        shell: body.shell.unwrap_or_default(),
        last_seen: crate::db::now(),
        arch: String::new(),
        hostname: String::new(),
    };
    // Prove the pairing works before saving it.
    let client = crate::device::DeviceClient::new(&d.url, &d.token);
    let info = client
        .info()
        .await
        .map_err(|e| ApiError::BadRequest(format!("could not reach {url}: {e}")))?;
    d.os = info
        .get("os")
        .and_then(Value::as_str)
        .unwrap_or(&d.os)
        .to_string();
    d.root = info
        .get("root")
        .and_then(Value::as_str)
        .unwrap_or(&d.root)
        .to_string();
    d.shell = info
        .get("shell")
        .and_then(Value::as_str)
        .unwrap_or(&d.shell)
        .to_string();
    d.hostname = info.get("hostname").and_then(Value::as_str).unwrap_or("").to_string();
    d.arch = info.get("arch").and_then(Value::as_str).unwrap_or("").to_string();
    s.devices.upsert(&d).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "device": device_json(&s, &d) })),
    ))
}

async fn get_device(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let d = s.devices.get(&id).await?.ok_or(ApiError::NotFound("device"))?;
    Ok(Json(json!({ "device": device_json(&s, &d) })))
}

/// Re-checks the link and refreshes `last_seen`.
async fn ping_device(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let d = s.devices.get(&id).await?.ok_or(ApiError::NotFound("device"))?;
    let client = crate::device::DeviceClient::for_device(&d, &s.links);
    match client.info().await {
        Ok(info) => {
            s.devices.touch(&id);
            Ok(Json(json!({ "connected": true, "info": info })))
        }
        Err(e) => Ok(Json(json!({ "connected": false, "error": e.to_string() }))),
    }
}

async fn remove_device(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let removed = s.devices.remove(&id).await?;
    if !removed {
        return Err(ApiError::NotFound("device"));
    }
    // Its key no longer works; close the link it may still have open.
    s.links.disconnect(&id);
    Ok(Json(json!({ "ok": true })))
}
