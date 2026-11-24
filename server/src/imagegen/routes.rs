//! REST routes for images. Pictures are served by `/attachments/{name}`.

use std::collections::BTreeMap;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use super::store::{self, ProjectPatch};
use super::{GenError, GenerateRequest, catalog};
use crate::api::{ApiError, AppState, JsonBody};

type ApiResult<T> = Result<T, ApiError>;

const MAX_NAME_CHARS: usize = 120;
const MAX_STYLE_CHARS: usize = 2000;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/images/status", get(status))
        .route("/images/prepare", post(prepare))
        .route("/images/unload", post(unload))
        .route("/images/catalog", get(get_catalog))
        .route("/images/settings", get(get_settings).patch(patch_settings))
        .route("/image-models", get(list_models).post(create_model))
        .route(
            "/image-models/{id}",
            axum::routing::patch(update_model).delete(delete_model),
        )
        .route("/image-projects", get(list_projects).post(create_project))
        .route(
            "/image-projects/{id}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route(
            "/image-projects/{id}/chats",
            get(list_chats).post(create_chat),
        )
        .route(
            "/image-chats/{id}",
            get(get_chat).patch(update_chat).delete(delete_chat),
        )
        .route("/image-chats/{id}/turns", get(list_turns))
        .route("/image-chats/{id}/generate", post(generate))
        .route("/image-turns/{id}", get(get_turn).delete(delete_turn))
        .route("/image-turns/{id}/stop", post(stop_turn))
}

impl From<GenError> for ApiError {
    fn from(e: GenError) -> Self {
        match e {
            GenError::Invalid(m) => ApiError::BadRequest(m),
            GenError::NotFound => ApiError::NotFound("image chat"),
            GenError::Busy(m) => ApiError::Conflict(m),
            GenError::Internal(e) => e.into(),
        }
    }
}

fn clean_name(name: Option<&str>, what: &str) -> ApiResult<String> {
    let n = name.unwrap_or_default().trim().to_string();
    if n.is_empty() {
        return Err(ApiError::BadRequest(format!("give the {what} a name")));
    }
    if n.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest(format!(
            "keep the name under {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(n)
}

// ---------- engine ----------

async fn status(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let models = s.images.db().call(|c| store::list_models(c)).await?;
    Ok(Json(json!({
        "engine": s.images.engine.status(),
        "chat_model": s.model.status().state,
        "models": models.len(),
    })))
}

async fn prepare(State(s): State<AppState>) -> ApiResult<(StatusCode, Json<Value>)> {
    s.images.prepare();
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

async fn unload(State(s): State<AppState>) -> ApiResult<(StatusCode, Json<Value>)> {
    let images = s.images.clone();
    tokio::spawn(async move { images.unload().await });
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
struct CatalogQuery {
    refresh: Option<String>,
}

async fn get_catalog(
    State(s): State<AppState>,
    Query(q): Query<CatalogQuery>,
) -> ApiResult<Json<Value>> {
    let refresh = q.refresh.is_some_and(|r| r != "0" && r != "false");
    let c = s
        .images
        .catalog(refresh)
        .await
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    let folders: Vec<String> = s.cfg.read(|l| {
        l.image_model_dirs()
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect()
    });
    let mut v = serde_json::to_value(c).map_err(anyhow::Error::from)?;
    v["folders"] = json!(folders);
    v["categories"] = json!(super::engine::MODEL_CATEGORIES);
    Ok(Json(v))
}

async fn get_settings(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let (images, own) = s.cfg.read(|l| {
        (
            l.config.images.clone(),
            l.image_models_dir().to_string_lossy().into_owned(),
        )
    });
    Ok(Json(json!({
        "sd_server_path": images.sd_server_path,
        "model_dirs": images.model_dirs,
        "own_folder": own,
        "offload_to_cpu": images.offload_to_cpu,
        "idle_unload_minutes": images.idle_unload_minutes,
    })))
}

#[derive(Deserialize)]
struct SettingsPatch {
    sd_server_path: Option<String>,
    model_dirs: Option<Vec<String>>,
    offload_to_cpu: Option<bool>,
    idle_unload_minutes: Option<u32>,
}

async fn patch_settings(
    State(s): State<AppState>,
    JsonBody(p): JsonBody<SettingsPatch>,
) -> ApiResult<Json<Value>> {
    if let Some(dirs) = &p.model_dirs {
        for d in dirs {
            if !std::path::Path::new(d.trim()).is_dir() {
                return Err(ApiError::BadRequest(format!(
                    "{d} isn't a folder on this PC"
                )));
            }
        }
    }
    let before = s.cfg.get().images;
    let updated = s.cfg.update(|c| {
        if let Some(v) = &p.sd_server_path {
            c.images.sd_server_path = v.trim().to_string();
        }
        if let Some(v) = &p.model_dirs {
            c.images.model_dirs = v
                .iter()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect();
        }
        if let Some(v) = p.offload_to_cpu {
            c.images.offload_to_cpu = v;
        }
        if let Some(v) = p.idle_unload_minutes {
            c.images.idle_unload_minutes = v;
        }
    })?;
    let after = updated.images;
    // Folders and launch options apply when sd-server starts again.
    let restart = before.sd_server_path != after.sd_server_path
        || before.model_dirs != after.model_dirs
        || before.offload_to_cpu != after.offload_to_cpu;
    if restart {
        let images = s.images.clone();
        tokio::spawn(async move { images.unload().await });
    }
    get_settings(State(s)).await
}

// ---------- models ----------

async fn list_models(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let models = s.images.db().call(|c| store::list_models(c)).await?;
    Ok(Json(json!({ "models": models })))
}

#[derive(Deserialize)]
struct ModelBody {
    name: Option<String>,
    recipe: Option<String>,
    files: Option<BTreeMap<String, String>>,
}

/// Checks the recipe and files against sd-server's catalog when it's
/// reachable (sd-server checks again when it loads them).
async fn check_model(
    s: &AppState,
    recipe: &str,
    files: &BTreeMap<String, String>,
) -> ApiResult<()> {
    let Ok(c) = s.images.catalog(false).await else {
        return Ok(());
    };
    let Some(r) = c.recipes.iter().find(|r| r.id == recipe) else {
        return Err(ApiError::BadRequest(format!("unknown model type {recipe}")));
    };
    if let Some((slot, _)) = files
        .iter()
        .find(|(slot, _)| !r.slots.iter().any(|sl| &sl.id == *slot))
    {
        return Err(ApiError::BadRequest(format!(
            "{} has no {slot} part",
            r.name
        )));
    }
    if let Some(f) = files
        .values()
        .find(|f| !f.is_empty() && !c.files.iter().any(|m| &m.id == *f))
    {
        return Err(ApiError::BadRequest(format!(
            "{f} isn't in the image model folders"
        )));
    }
    if !catalog::complete(r, files) {
        let missing: Vec<&str> = r
            .slots
            .iter()
            .filter(|sl| sl.required && files.get(&sl.id).is_none_or(|v| v.is_empty()))
            .map(|sl| sl.label.as_str())
            .collect();
        return Err(ApiError::BadRequest(if missing.is_empty() {
            "choose a VAE (or TAE) file".into()
        } else {
            format!("choose a file for {}", missing.join(", "))
        }));
    }
    Ok(())
}

async fn create_model(
    State(s): State<AppState>,
    JsonBody(b): JsonBody<ModelBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = clean_name(b.name.as_deref(), "model")?;
    let recipe = b.recipe.unwrap_or_default().trim().to_string();
    if recipe.is_empty() {
        return Err(ApiError::BadRequest("choose the model type".into()));
    }
    let files: BTreeMap<String, String> = b
        .files
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .collect();
    check_model(&s, &recipe, &files).await?;
    let m = s
        .images
        .db()
        .call(move |c| store::insert_model(c, &name, &recipe, &files))
        .await?;
    Ok((StatusCode::CREATED, Json(json!(m))))
}

async fn update_model(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(b): JsonBody<ModelBody>,
) -> ApiResult<Json<Value>> {
    let mid = id.clone();
    let cur = s
        .images
        .db()
        .call(move |c| store::get_model(c, &mid))
        .await?
        .ok_or(ApiError::NotFound("image model"))?;
    let name = match b.name.as_deref() {
        Some(n) => clean_name(Some(n), "model")?,
        None => cur.name.clone(),
    };
    let files = match b.files {
        Some(f) => {
            let f: BTreeMap<String, String> = f
                .into_iter()
                .filter(|(_, v)| !v.trim().is_empty())
                .collect();
            check_model(&s, &cur.recipe, &f).await?;
            f
        }
        None => cur.files.clone(),
    };
    let (mid, n2, f2) = (id.clone(), name, files);
    s.images
        .db()
        .call(move |c| store::update_model(c, &mid, &n2, &f2))
        .await?;
    let m = s
        .images
        .db()
        .call(move |c| store::get_model(c, &id))
        .await?
        .ok_or(ApiError::NotFound("image model"))?;
    Ok(Json(json!(m)))
}

async fn delete_model(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<StatusCode> {
    let found = s
        .images
        .db()
        .call(move |c| store::delete_model(c, &id))
        .await?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("image model"))
    }
}

// ---------- projects ----------

async fn list_projects(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let projects = s.images.db().call(|c| store::list_projects(c)).await?;
    Ok(Json(json!({ "projects": projects })))
}

#[derive(Deserialize)]
struct CreateProject {
    name: Option<String>,
    model_id: Option<String>,
}

async fn create_project(
    State(s): State<AppState>,
    JsonBody(b): JsonBody<CreateProject>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = clean_name(b.name.as_deref(), "project")?;
    let model = b.model_id.filter(|m| !m.is_empty());
    let p = s
        .images
        .db()
        .call(move |c| store::insert_project(c, &name, model.as_deref()))
        .await?;
    Ok((StatusCode::CREATED, Json(json!(p))))
}

async fn get_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let p = s
        .images
        .db()
        .call(move |c| store::get_project(c, &id))
        .await?
        .ok_or(ApiError::NotFound("image project"))?;
    Ok(Json(json!(p)))
}

async fn update_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(mut p): JsonBody<ProjectPatch>,
) -> ApiResult<Json<Value>> {
    if let Some(n) = p.name.as_deref() {
        p.name = Some(clean_name(Some(n), "project")?);
    }
    for v in [&p.style, &p.negative].into_iter().flatten() {
        if v.chars().count() > MAX_STYLE_CHARS {
            return Err(ApiError::BadRequest(format!(
                "keep it under {MAX_STYLE_CHARS} characters"
            )));
        }
    }
    let pid = id.clone();
    let found = s
        .images
        .db()
        .call(move |c| store::update_project(c, &pid, &p))
        .await?;
    if !found {
        return Err(ApiError::NotFound("image project"));
    }
    get_project(State(s), UrlPath(id)).await
}

async fn delete_project(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<StatusCode> {
    let pid = id.clone();
    let chats = s
        .images
        .db()
        .call(move |c| store::list_chats(c, &pid))
        .await?;
    if chats.iter().any(|c| c.status == "running") {
        return Err(ApiError::Conflict(
            "a picture is being made in this project; stop it first".into(),
        ));
    }
    let pics = s
        .images
        .db()
        .call(move |c| store::delete_project(c, &id))
        .await?
        .ok_or(ApiError::NotFound("image project"))?;
    s.images.forget_pictures(pics).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- chats ----------

async fn list_chats(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let chats = s
        .images
        .db()
        .call(move |c| store::list_chats(c, &id))
        .await?;
    Ok(Json(json!({ "chats": chats })))
}

#[derive(Deserialize)]
struct CreateChat {
    title: Option<String>,
    model_id: Option<String>,
}

async fn create_chat(
    State(s): State<AppState>,
    UrlPath(project_id): UrlPath<String>,
    JsonBody(b): JsonBody<CreateChat>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let title = match b.title.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => clean_name(Some(t), "chat")?,
        _ => "New image".to_string(),
    };
    let chat = s
        .images
        .db()
        .call(move |c| {
            let Some(p) = store::get_project(c, &project_id)? else {
                return Ok(None);
            };
            let model = b.model_id.or(p.model_id);
            Ok(Some(store::insert_chat(
                c,
                &p.id,
                &title,
                model.as_deref(),
            )?))
        })
        .await?
        .ok_or(ApiError::NotFound("image project"))?;
    Ok((StatusCode::CREATED, Json(json!(chat))))
}

async fn get_chat(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let chat = s
        .images
        .db()
        .call(move |c| store::get_chat(c, &id))
        .await?
        .ok_or(ApiError::NotFound("image chat"))?;
    Ok(Json(json!(chat)))
}

#[derive(Deserialize)]
struct ChatPatch {
    title: Option<String>,
    /// "" clears it.
    model_id: Option<String>,
}

async fn update_chat(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(b): JsonBody<ChatPatch>,
) -> ApiResult<Json<Value>> {
    let title = match b.title.as_deref() {
        Some(t) => Some(clean_name(Some(t), "chat")?),
        None => None,
    };
    let cid = id.clone();
    let found = s
        .images
        .db()
        .call(move |c| {
            let model = b.model_id.as_deref().map(|m| (!m.is_empty()).then_some(m));
            store::update_chat(c, &cid, title.as_deref(), model)
        })
        .await?;
    if !found {
        return Err(ApiError::NotFound("image chat"));
    }
    get_chat(State(s), UrlPath(id)).await
}

async fn delete_chat(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<StatusCode> {
    let cid = id.clone();
    let chat = s
        .images
        .db()
        .call(move |c| store::get_chat(c, &cid))
        .await?
        .ok_or(ApiError::NotFound("image chat"))?;
    if chat.status == "running" {
        return Err(ApiError::Conflict(
            "a picture is being made in this chat; stop it first".into(),
        ));
    }
    let pics = s
        .images
        .db()
        .call(move |c| store::delete_chat(c, &id))
        .await?
        .ok_or(ApiError::NotFound("image chat"))?;
    s.images.forget_pictures(pics).await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- turns ----------

async fn list_turns(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let turns = s
        .images
        .db()
        .call(move |c| store::list_turns(c, &id))
        .await?;
    let live: serde_json::Map<String, Value> = turns
        .iter()
        .filter(|t| t.status == "queued" || t.status == "running")
        .filter_map(|t| s.images.live(&t.id).map(|l| (t.id.clone(), json!(l))))
        .collect();
    Ok(Json(json!({ "turns": turns, "live": live })))
}

async fn generate(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(req): JsonBody<GenerateRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let turn = s.images.generate(&id, req).await?;
    Ok((StatusCode::ACCEPTED, Json(json!(turn))))
}

async fn get_turn(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let tid = id.clone();
    let turn = s
        .images
        .db()
        .call(move |c| store::get_turn(c, &tid))
        .await?
        .ok_or(ApiError::NotFound("picture"))?;
    Ok(Json(json!({ "turn": turn, "live": s.images.live(&id) })))
}

async fn stop_turn(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "stopping": s.images.cancel(&id) })))
}

async fn delete_turn(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<StatusCode> {
    if s.images.live(&id).is_some() {
        return Err(ApiError::Conflict("stop the picture first".into()));
    }
    let pics = s
        .images
        .db()
        .call(move |c| store::delete_turn(c, &id))
        .await?
        .ok_or(ApiError::NotFound("picture"))?;
    s.images.forget_pictures(pics).await;
    Ok(StatusCode::NO_CONTENT)
}
