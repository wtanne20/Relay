//! Image generation: image projects hold image chats, and each message in an
//! image chat makes pictures with stable-diffusion.cpp's sd-server. A message
//! can start from an earlier picture to refine it.
//!
//! The image model and the chat model share the GPU (see [`crate::gpu`]):
//! making a picture unloads the chat model first, and a chat that needs its
//! model stops the image model once the current picture is done.

pub mod catalog;
pub mod engine;
pub mod hub;
pub mod routes;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::SharedConfig;
use crate::db::Db;
use crate::gpu::Gpu;
use crate::setup::Hardware;
use catalog::Catalog;
use engine::Engine;
use store::{ImageModel, ImageTurn, NewTurn, TurnParams};

/// Longest a picture may take, including loading the model.
const JOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const POLL: Duration = Duration::from_millis(500);
pub const MAX_PROMPT_CHARS: usize = 4000;
pub const MAX_COUNT: u32 = 4;
pub const MIN_SIDE: u32 = 256;
pub const MAX_SIDE: u32 = 2048;
/// Strength used when refining with a model that starts from the picture.
pub const DEFAULT_STRENGTH: f32 = 0.6;

/// What a picture being made is doing right now (kept in memory only).
#[derive(Debug, Clone, Serialize, Default)]
pub struct Live {
    /// queued, waiting_for_chat, starting, loading, sampling, decoding, saving
    pub stage: String,
    pub step: Option<u64>,
    pub total: Option<u64>,
    /// The latest preview frame as a data URL.
    pub preview: Option<String>,
    pub elapsed_ms: u64,
    #[serde(skip)]
    cancel: CancellationToken,
}

pub struct ImageGen {
    pub engine: Arc<Engine>,
    db: Db,
    gpu: Arc<Gpu>,
    shared: Arc<SharedConfig>,
    /// Where pictures are stored (the same folder as chat attachments, so
    /// they're served by `/attachments/{name}`).
    pics_dir: PathBuf,
    live: Mutex<HashMap<String, Live>>,
    /// One picture at a time.
    serial: tokio::sync::Mutex<()>,
    catalog: Mutex<Option<(Instant, Catalog)>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GenerateRequest {
    pub prompt: String,
    pub negative: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub steps: Option<u32>,
    pub cfg_scale: Option<f32>,
    pub strength: Option<f32>,
    pub count: Option<u32>,
    pub seed: Option<i64>,
    pub model_id: Option<String>,
    /// A stored picture to start from (one made in this chat).
    pub base_image: Option<String>,
    /// Or a new picture to start from, as a data URL.
    pub base_upload: Option<String>,
}

impl ImageGen {
    pub fn new(
        db: Db,
        shared: Arc<SharedConfig>,
        hardware: Hardware,
        gpu: Arc<Gpu>,
        pics_dir: PathBuf,
    ) -> Arc<Self> {
        let engine = Engine::new(shared.clone(), hardware, gpu.clone());
        engine.spawn_watcher();
        Arc::new(Self {
            engine,
            db,
            gpu,
            shared,
            pics_dir,
            live: Mutex::new(HashMap::new()),
            serial: tokio::sync::Mutex::new(()),
            catalog: Mutex::new(None),
        })
    }

    pub fn live(&self, turn_id: &str) -> Option<Live> {
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(turn_id)
            .cloned()
    }

    fn set_live(&self, turn_id: &str, f: impl FnOnce(&mut Live)) {
        if let Some(l) = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(turn_id)
        {
            f(l);
        }
    }

    pub fn cancel(&self, turn_id: &str) -> bool {
        match self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(turn_id)
        {
            Some(l) => {
                l.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Stops the image model now (a picture being made is finished first).
    pub async fn unload(&self) {
        self.gpu.release_images().await;
    }

    /// Whether a picture is being made (or waiting its turn) now.
    pub fn busy(&self) -> bool {
        !self.live.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Deletes model files (ids as the catalog lists them, relative to an
    /// image model folder) from the disk. The image engine is stopped first
    /// so the files aren't locked, and the folders are rescanned when it
    /// next starts. Returns the bytes freed.
    pub async fn delete_files(&self, ids: &[String]) -> Result<u64> {
        let roots = self.shared.read(|l| l.image_model_dirs());
        let mut paths = Vec::new();
        for id in ids {
            let rel = std::path::Path::new(id);
            let safe = !id.is_empty()
                && rel
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)));
            if !safe {
                bail!("{id} isn't a model file");
            }
            if let Some(p) = roots.iter().map(|r| r.join(rel)).find(|p| p.is_file()) {
                paths.push(p);
            }
        }
        if paths.is_empty() {
            return Ok(0);
        }
        self.unload().await;
        let mut freed = 0;
        for p in &paths {
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            let mut attempt = 0;
            loop {
                match tokio::fs::remove_file(p).await {
                    Ok(()) => {
                        freed += size;
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) if attempt == 9 => {
                        bail!("could not delete {}: {e}", p.display())
                    }
                    Err(_) => {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
        // The cached list still names the deleted files.
        *self.catalog.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(freed)
    }

    /// Frees the GPU for images ahead of time and starts the engine, so the
    /// first picture is quicker. Returns right away.
    pub fn prepare(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let job = this.gpu.claim_for_images(|_| {}).await;
            if let Err(e) = this.engine.ensure_running().await {
                tracing::warn!("image engine: {e:#}");
            }
            drop(job);
        });
    }

    /// Recipes and model files, from sd-server (started if needed). Cached
    /// for a little while; `refresh` rescans the folders.
    pub async fn catalog(&self, refresh: bool) -> Result<Catalog> {
        if !refresh
            && let Some((t, c)) = self
                .catalog
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            && t.elapsed() < Duration::from_secs(20)
        {
            return Ok(c.clone());
        }
        self.engine.ensure_running().await?;
        if refresh {
            let _ = self
                .engine
                .post_json("/sdcpp/v1/models/refresh", &json!({}))
                .await;
        }
        let (recipes, files) = tokio::join!(
            self.engine.get_json("/sdcpp/v1/recipes"),
            self.engine.get_json("/sdcpp/v1/model-files"),
        );
        let (recipes, files) = (recipes?, files?);
        let mut errors = catalog::parse_errors(&recipes);
        errors.extend(catalog::parse_errors(&files));
        let recipes = catalog::parse_recipes(&recipes);
        let files = catalog::parse_files(&files);
        let saved = self.db.call(|c| store::list_models(c)).await?;
        let suggestions = catalog::suggest(&recipes, &files)
            .into_iter()
            .filter(|s| {
                !saved
                    .iter()
                    .any(|m| m.recipe == s.recipe && m.files == s.files)
            })
            .collect();
        let c = Catalog {
            recipes,
            files,
            suggestions,
            errors,
        };
        *self.catalog.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), c.clone()));
        Ok(c)
    }

    /// Starts making pictures for one message. Returns the new turn.
    pub async fn generate(
        self: &Arc<Self>,
        chat_id: &str,
        req: GenerateRequest,
    ) -> Result<ImageTurn, GenError> {
        let prompt = req.prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(GenError::Invalid("describe the picture you want".into()));
        }
        if prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(GenError::Invalid(format!(
                "keep the prompt under {MAX_PROMPT_CHARS} characters"
            )));
        }
        let count = req.count.unwrap_or(1);
        if !(1..=MAX_COUNT).contains(&count) {
            return Err(GenError::Invalid(format!(
                "make 1 to {MAX_COUNT} pictures at a time"
            )));
        }
        for side in [req.width, req.height].into_iter().flatten() {
            if !(MIN_SIDE..=MAX_SIDE).contains(&side) {
                return Err(GenError::Invalid(format!(
                    "picture sides must be {MIN_SIDE} to {MAX_SIDE} pixels"
                )));
            }
        }
        if req.steps.is_some_and(|s| !(1..=150).contains(&s)) {
            return Err(GenError::Invalid("steps must be 1 to 150".into()));
        }
        if req.cfg_scale.is_some_and(|c| !(0.0..=30.0).contains(&c)) {
            return Err(GenError::Invalid("guidance must be 0 to 30".into()));
        }
        if req.strength.is_some_and(|s| !(0.0..=1.0).contains(&s)) {
            return Err(GenError::Invalid("strength must be 0 to 1".into()));
        }

        let cid = chat_id.to_string();
        let wanted = req.model_id.clone();
        let base_name = req.base_image.clone();
        let ctx = self
            .db
            .call(move |c| {
                let Some(chat) = store::get_chat(c, &cid)? else {
                    return Ok(None);
                };
                let project = store::get_project(c, &chat.project_id)?;
                let models = store::list_models(c)?;
                let base_turn = match &base_name {
                    Some(n) => store::turn_with_picture(c, &cid, n)?,
                    None => None,
                };
                Ok(Some((chat, project, models, base_turn)))
            })
            .await?;
        let Some((chat, project, models, base_turn)) = ctx else {
            return Err(GenError::NotFound);
        };
        if chat.status == "running" {
            return Err(GenError::Busy(
                "this chat is still making a picture; wait for it or stop it".into(),
            ));
        }
        let model = pick_model(
            &models,
            [
                wanted.as_deref(),
                chat.model_id.as_deref(),
                project.as_ref().and_then(|p| p.model_id.as_deref()),
            ],
        )
        .ok_or_else(|| {
            GenError::Invalid(
                "set up an image model first (Image models in Settings, or the model button above the message box)"
                    .into(),
            )
        })?;
        let recipe = self
            .catalog(false)
            .await
            .ok()
            .and_then(|c| c.recipes.into_iter().find(|r| r.id == model.recipe));
        let edit = recipe.as_ref().is_some_and(|r| r.edit);

        // The picture to start from.
        let base = match (&req.base_image, &req.base_upload) {
            (Some(n), _) => {
                if base_turn.is_none() {
                    return Err(GenError::Invalid(
                        "start from a picture made in this chat".into(),
                    ));
                }
                Some(n.clone())
            }
            (None, Some(data)) => {
                let dir = self.pics_dir.clone();
                let data = data.clone();
                Some(
                    tokio::task::spawn_blocking(move || crate::images::save(&dir, &data))
                        .await
                        .map_err(|e| anyhow!("{e}"))?
                        .map_err(|e| GenError::Invalid(format!("{e:#}")))?,
                )
            }
            (None, None) => None,
        };

        let style = project
            .as_ref()
            .map(|p| p.style.trim().to_string())
            .unwrap_or_default();
        let core = match (&base, &base_turn, edit) {
            // A model that can't follow edits needs the whole description.
            (Some(_), Some(prev), false) => {
                let prev_core = strip_style(&prev.full_prompt, &style);
                format!("{prev_core}, {prompt}")
            }
            _ => prompt.clone(),
        };
        let full_prompt = join_prompt(&core, &style);
        let negative = join_prompt(
            project.as_ref().map(|p| p.negative.as_str()).unwrap_or(""),
            req.negative.as_deref().unwrap_or(""),
        );
        let (width, height) = match (req.width, req.height, &base) {
            (None, None, Some(b)) => base_size(&self.pics_dir.join(b))
                .map(|(w, h)| (Some(w), Some(h)))
                .unwrap_or((None, None)),
            (w, h, _) => (w, h),
        };
        let params = TurnParams {
            width,
            height,
            steps: req.steps,
            cfg_scale: req.cfg_scale,
            strength: base
                .as_ref()
                .filter(|_| !edit)
                .map(|_| req.strength.unwrap_or(DEFAULT_STRENGTH)),
            count,
            edit: base.is_some() && edit,
        };
        let seed = req
            .seed
            .filter(|s| *s >= 0)
            .unwrap_or_else(|| i64::from(rand::random::<u32>() >> 1));
        let new = NewTurn {
            chat_id: chat.id.clone(),
            prompt: prompt.clone(),
            full_prompt,
            negative,
            params,
            base_image: base,
            model_id: Some(model.id.clone()),
            model_name: model.name.clone(),
            seed,
        };
        let chat_id2 = chat.id.clone();
        let model_id = model.id.clone();
        let first = chat.title.trim().is_empty() || chat.title == "New image";
        let title = title_from(&prompt);
        let turn = self
            .db
            .call(move |c| {
                let t = store::insert_turn(c, &new)?;
                store::update_chat(
                    c,
                    &chat_id2,
                    first.then_some(title.as_str()),
                    Some(Some(&model_id)),
                )?;
                Ok(t)
            })
            .await?;
        let cancel = CancellationToken::new();
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(
            turn.id.clone(),
            Live {
                stage: "queued".into(),
                cancel: cancel.clone(),
                ..Default::default()
            },
        );
        let this = self.clone();
        let t = turn.clone();
        tokio::spawn(async move { this.run(t, model, cancel).await });
        Ok(turn)
    }

    async fn run(self: Arc<Self>, turn: ImageTurn, model: ImageModel, cancel: CancellationToken) {
        let started = Instant::now();
        let result = tokio::select! {
            r = self.make(&turn, &model, &cancel) => r,
            _ = tokio::time::sleep(JOB_TIMEOUT) => Err(anyhow!("the picture took too long")),
        };
        let duration = started.elapsed().as_millis() as i64;
        let (status, error, images) = match result {
            Ok(images) => ("done", None, images),
            Err(_) if cancel.is_cancelled() => ("stopped", None, Vec::new()),
            Err(e) => ("error", Some(format!("{e:#}")), Vec::new()),
        };
        if let Some(e) = &error {
            tracing::warn!(turn = %turn.id, "picture failed: {e}");
        }
        let id = turn.id.clone();
        let imgs = images.clone();
        if let Err(e) = self
            .db
            .call(move |c| {
                store::finish_turn(c, &id, status, error.as_deref(), &imgs, Some(duration))
            })
            .await
        {
            tracing::error!("saving the picture's result: {e:#}");
        }
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&turn.id);
        self.engine.touch();
    }

    /// Waits its turn, takes the GPU, asks sd-server for the pictures and
    /// stores them. Returns the stored names.
    async fn make(
        &self,
        turn: &ImageTurn,
        model: &ImageModel,
        cancel: &CancellationToken,
    ) -> Result<Vec<String>> {
        let _serial = tokio::select! {
            g = self.serial.lock() => g,
            _ = cancel.cancelled() => bail!("stopped"),
        };
        let id = turn.id.clone();
        {
            let tid = turn.id.clone();
            self.db
                .call(move |c| store::set_turn_running(c, &tid))
                .await?;
        }
        let job = {
            let claim = self.gpu.claim_for_images(|waiting| {
                self.set_live(&id, |l| {
                    l.stage = if waiting {
                        "waiting_for_chat"
                    } else {
                        "starting"
                    }
                    .into()
                })
            });
            tokio::select! {
                j = claim => j,
                _ = cancel.cancelled() => bail!("stopped"),
            }
        };
        self.set_live(&id, |l| l.stage = "starting".into());
        tokio::select! {
            r = self.engine.ensure_running() => r?,
            _ = cancel.cancelled() => bail!("stopped"),
        }
        self.set_live(&id, |l| l.stage = "loading".into());
        let body = self.request_body(turn, model).await?;
        let submitted = match self.engine.post_json("/sdcpp/v1/img_gen", &body).await {
            Ok(v) => v,
            // Older builds may not know the preview option; try without it.
            Err(e) if body.get("preview").is_some() => {
                tracing::debug!("img_gen with preview failed ({e:#}); retrying without");
                let mut b = body.clone();
                if let Some(o) = b.as_object_mut() {
                    o.remove("preview");
                    o.remove("preview_interval");
                }
                self.engine.post_json("/sdcpp/v1/img_gen", &b).await?
            }
            Err(e) => return Err(e),
        };
        let job_id = submitted
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("the image engine didn't return a job"))?
            .to_string();
        let result = self.follow(&id, &job_id, cancel).await;
        drop(job);
        let images = result?;
        self.set_live(&id, |l| l.stage = "saving".into());
        let dir = self.pics_dir.clone();
        let names = tokio::task::spawn_blocking(move || save_pngs(&dir, &images)).await??;
        if names.is_empty() {
            bail!("the image engine returned no pictures");
        }
        Ok(names)
    }

    async fn request_body(&self, turn: &ImageTurn, model: &ImageModel) -> Result<Value> {
        let p = &turn.params;
        let mut body = json!({
            "prompt": turn.full_prompt,
            "negative_prompt": turn.negative,
            "seed": turn.seed.unwrap_or(-1),
            "batch_count": p.count.max(1),
            "model": { "recipe": model.recipe, "models": model.files },
            "output_format": "png",
            "preview": "proj",
            "preview_interval": 2,
        });
        let o = body.as_object_mut().expect("object");
        if let Some(w) = p.width {
            o.insert("width".into(), json!(w));
        }
        if let Some(h) = p.height {
            o.insert("height".into(), json!(h));
        }
        let mut sample = serde_json::Map::new();
        if let Some(s) = p.steps {
            sample.insert("sample_steps".into(), json!(s));
        }
        if let Some(c) = p.cfg_scale {
            sample.insert("guidance".into(), json!({ "txt_cfg": c }));
        }
        if !sample.is_empty() {
            o.insert("sample_params".into(), Value::Object(sample));
        }
        if let Some(base) = &turn.base_image {
            let path = self.pics_dir.join(base);
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|_| anyhow!("the picture to start from is gone"))?;
            let url = format!(
                "data:{};base64,{}",
                crate::images::mime(base),
                STANDARD.encode(bytes)
            );
            if p.edit {
                o.insert("ref_images".into(), json!([url]));
            } else {
                o.insert("init_image".into(), json!(url));
                o.insert(
                    "strength".into(),
                    json!(p.strength.unwrap_or(DEFAULT_STRENGTH)),
                );
            }
        }
        Ok(body)
    }

    /// Polls the sd-server job until it ends; returns the PNG bytes.
    async fn follow(
        &self,
        turn_id: &str,
        job_id: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<Vec<u8>>> {
        let path = format!("/sdcpp/v1/jobs/{job_id}");
        let mut cancel_sent: Option<Instant> = None;
        let mut misses = 0;
        loop {
            if cancel.is_cancelled() && cancel_sent.is_none() {
                cancel_sent = Some(Instant::now());
                let _ = self
                    .engine
                    .post_json(&format!("{path}/cancel"), &json!({}))
                    .await;
            }
            if cancel_sent.is_some_and(|t| t.elapsed() > Duration::from_secs(20)) {
                bail!("stopped");
            }
            let v = match self.engine.get_json(&path).await {
                Ok(v) => {
                    misses = 0;
                    v
                }
                Err(e) => {
                    misses += 1;
                    if misses >= 6 {
                        return Err(e.context("lost the image engine"));
                    }
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            };
            let status = v.get("status").and_then(Value::as_str).unwrap_or("");
            self.set_live(turn_id, |l| apply_progress(l, &v));
            match status {
                "completed" => return Ok(decode_images(&v)),
                "failed" => {
                    let msg = v
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("the picture failed");
                    bail!("{}", friendly_error(msg));
                }
                "cancelled" => bail!("stopped"),
                _ => {}
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = cancel.cancelled(), if cancel_sent.is_none() => {}
            }
        }
    }

    /// Deletes stored pictures nothing refers to any more.
    pub async fn forget_pictures(&self, names: Vec<String>) {
        let dir = self.pics_dir.clone();
        let _ = self
            .db
            .call(move |c| {
                for n in names {
                    if crate::images::valid_name(&n) && !store::picture_in_use(c, &n)? {
                        let _ = std::fs::remove_file(dir.join(&n));
                    }
                }
                Ok(())
            })
            .await;
    }

    pub fn shared(&self) -> &Arc<SharedConfig> {
        &self.shared
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("{0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Busy(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

fn pick_model(models: &[ImageModel], prefs: [Option<&str>; 3]) -> Option<ImageModel> {
    prefs
        .into_iter()
        .flatten()
        .find_map(|id| models.iter().find(|m| m.id == id))
        .or_else(|| models.last())
        .cloned()
}

fn join_prompt(a: &str, b: &str) -> String {
    let (a, b) = (a.trim(), b.trim());
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}, {b}"),
    }
}

fn strip_style(full: &str, style: &str) -> String {
    let style = style.trim();
    if style.is_empty() {
        return full.to_string();
    }
    full.strip_suffix(style)
        .map(|s| s.trim_end().trim_end_matches(',').trim_end().to_string())
        .unwrap_or_else(|| full.to_string())
}

/// A chat title from its first prompt.
pub fn title_from(prompt: &str) -> String {
    let one_line: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut t: String = one_line.chars().take(48).collect();
    if one_line.chars().count() > 48 {
        if let Some(i) = t.rfind(' ').filter(|i| *i > 20) {
            t.truncate(i);
        }
        t.push('…');
    }
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "New image".into(),
    }
}

/// The base picture's size, rounded to a multiple of 16 within limits.
fn base_size(path: &Path) -> Option<(u32, u32)> {
    let (w, h) = image::image_dimensions(path).ok()?;
    let scale = (MAX_SIDE as f32 / w.max(h) as f32).min(1.0);
    let fit = |v: u32| (((v as f32 * scale) as u32) / 16 * 16).clamp(MIN_SIDE, MAX_SIDE);
    Some((fit(w), fit(h)))
}

fn apply_progress(l: &mut Live, v: &Value) {
    if let Some(p) = v.get("progress") {
        let stage = p.get("stage").and_then(Value::as_str).unwrap_or("");
        l.stage = match stage {
            "queued" | "waiting" | "preparing" | "conditioning" | "" => "loading",
            "sampling" => "sampling",
            "vae_decode" | "vae_encode" | "encoding" | "upscaling" => "decoding",
            other => other,
        }
        .into();
        let steps = p.get("unit").and_then(Value::as_str) == Some("steps");
        l.step = steps
            .then(|| p.get("completed").and_then(Value::as_u64))
            .flatten();
        l.total = steps
            .then(|| p.get("total").and_then(Value::as_u64))
            .flatten();
        l.elapsed_ms = p.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(0);
    }
    if let Some(b64) = v.pointer("/preview/b64_json").and_then(Value::as_str)
        && !b64.is_empty()
    {
        l.preview = Some(format!("data:image/png;base64,{b64}"));
    }
}

fn decode_images(v: &Value) -> Vec<Vec<u8>> {
    v.pointer("/result/images")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|i| i.get("b64_json").and_then(Value::as_str))
                .filter_map(|b| STANDARD.decode(b.trim()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn save_pngs(dir: &Path, images: &[Vec<u8>]) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut names = Vec::new();
    for bytes in images {
        if !bytes.starts_with(b"\x89PNG") {
            bail!("the image engine returned something that isn't a PNG");
        }
        let name = format!("{}.png", uuid::Uuid::new_v4().simple());
        std::fs::write(dir.join(&name), bytes)?;
        names.push(name);
    }
    Ok(names)
}

/// sd-server's messages, a little friendlier for the common cases.
fn friendly_error(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("out of memory") || lower.contains("cudamalloc") || lower.contains("oom") {
        return format!(
            "the GPU ran out of memory. Try a smaller picture, a smaller (more quantized) model, or turn on \"Use less VRAM\" in Image models. ({msg})"
        );
    }
    msg.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_keep_the_project_style() {
        assert_eq!(join_prompt("a fox", "watercolor"), "a fox, watercolor");
        assert_eq!(join_prompt("a fox", " "), "a fox");
        assert_eq!(strip_style("a fox, watercolor", "watercolor"), "a fox");
        assert_eq!(strip_style("a fox", "watercolor"), "a fox");
    }

    #[test]
    fn titles() {
        assert_eq!(title_from("a red fox in the snow"), "A red fox in the snow");
        let long =
            title_from("a very detailed painting of a lighthouse on a cliff at sunset with waves");
        assert!(long.ends_with('…') && long.chars().count() <= 49, "{long}");
    }

    #[test]
    fn reads_job_progress() {
        let mut l = Live::default();
        apply_progress(
            &mut l,
            &json!({"status": "generating", "progress": {"stage": "sampling", "unit": "steps", "completed": 3, "total": 9, "elapsed_ms": 1200},
                    "preview": {"b64_json": "AAAA"}}),
        );
        assert_eq!(
            (l.stage.as_str(), l.step, l.total),
            ("sampling", Some(3), Some(9))
        );
        assert_eq!(l.preview.as_deref(), Some("data:image/png;base64,AAAA"));
    }

    #[test]
    fn explains_out_of_memory() {
        assert!(friendly_error("CUDA error: out of memory").contains("smaller"));
    }
}
