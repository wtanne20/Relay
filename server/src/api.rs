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

use crate::agent::{AgentManager, StartError};
use crate::config::{LoadedConfig, constant_time_eq};
use crate::db::{self, Db};
use crate::model_manager::ModelManager;
use crate::push::Pusher;

pub struct AppInner {
    pub cfg: Arc<LoadedConfig>,
    pub db: Db,
    pub model: ModelManager,
    pub agent: Arc<AgentManager>,
    pub pusher: Pusher,
    /// Cancelled when Relay is shutting down (closes WebSockets).
    pub shutdown: CancellationToken,
    pub started_at: chrono::DateTime<chrono::Utc>,
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
            StartError::Internal(e) => e.into(),
        }
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
        .route("/model", get(model_status))
        .route("/model/restart", post(model_restart))
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
            get(get_session)
                .patch(rename_session)
                .delete(delete_session),
        )
        .route("/sessions/{id}/messages", get(list_messages))
        .route("/sessions/{id}/prompt", post(send_prompt))
        .route("/sessions/{id}/stop", post(stop_task))
        .route("/sessions/{id}/ws", get(ws_upgrade))
        .route(
            "/push-tokens",
            post(add_push_token).delete(remove_push_token),
        )
        .fallback(|| async { ApiError::NotFound("route") })
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Requires `Authorization: Bearer <token>` on every request. WebSocket
/// upgrades may instead pass `?token=` for clients that cannot set headers.
async fn auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let expected = state.cfg.config.token.as_bytes();
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
        "max_steps": s.cfg.config.max_steps,
        "model": s.model.status(),
    })))
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
    path: String,
}

#[derive(Deserialize)]
struct UpdateProject {
    name: Option<String>,
    path: Option<String>,
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
    let path = validate_folder(&body.path)?;
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
    let path = body.path.as_deref().map(validate_folder).transpose()?;
    let project =
        s.db.call(move |c| db::update_project(c, &id, name.as_deref(), path.as_deref()))
            .await?
            .ok_or(ApiError::NotFound("project"))?;
    Ok(Json(json!({ "project": project })))
}

/// Removes the project from Relay only. The folder on disk is never touched.
async fn delete_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
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
struct RenameSession {
    title: String,
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
    let title = parsed
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("Session {}", chrono::Local::now().format("%b %-d, %H:%M")));
    let session =
        s.db.call(move |c| {
            if db::get_project(c, &id)?.is_none() {
                return Ok(None);
            }
            Ok(Some(db::create_session(c, &id, &title)?))
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

async fn rename_session(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<RenameSession>,
) -> ApiResult<Json<Value>> {
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::BadRequest("title must not be empty".into()));
    }
    let session =
        s.db.call(move |c| db::rename_session(c, &id, &title))
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
