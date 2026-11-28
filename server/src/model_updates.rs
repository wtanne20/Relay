//! Model updates: notices when a model downloaded from Hugging Face has a
//! newer version there (its file changed upstream, e.g. a fixed
//! quantization), and swaps the new version in.
//!
//! Each file's version is the hash Hugging Face lists for it in the
//! repository tree (`lfs.oid`, or the git blob id for small files). Relay
//! notes it when a file is downloaded. A file downloaded before Relay kept
//! notes is taken to be current when its size matches the one upstream, and
//! from then on its hash is compared.
//!
//! Where a file came from: chat models are saved as
//! `<library>/<owner>/<repo>/<path in repo>`, so the path says it. Image
//! model files are saved as `<image-models>/<category>/<name>`, so their
//! origin is noted at download time (starter models' files are known).
//!
//! Checks run once a day in the background, or on demand from the app.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{ApiError, AppState, JsonBody};
use crate::config::SharedConfig;
use crate::hub::{self, DownloadState};

type ApiResult<T> = Result<T, ApiError>;

/// Download group of update downloads.
pub const GROUP: &str = "update";
/// How often the background check runs.
const CHECK_EVERY: chrono::TimeDelta = chrono::TimeDelta::hours(24);

/// A file's version on Hugging Face.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Version {
    pub oid: String,
    pub size: u64,
}

/// Where an image model file came from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Origin {
    pub repo: String,
    pub path: String,
}

/// One file of an update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateFile {
    /// Path in the repository.
    pub remote: String,
    /// The file on the PC it replaces.
    pub local: String,
    pub size_bytes: u64,
    pub oid: String,
}

/// A model with a newer version on Hugging Face.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Update {
    /// "chat" or "image".
    pub kind: String,
    /// The model: a chat model's (first) file, or an image model file's id
    /// ("diffusion_models/x.gguf").
    pub path: String,
    pub name: String,
    pub repo: String,
    pub files: Vec<UpdateFile>,
    /// What the update downloads.
    pub size_bytes: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    /// The version of each file on the PC, keyed `repo:path`.
    known: BTreeMap<String, Version>,
    /// Image model files' origin, keyed by id (`category/name`).
    image_origins: BTreeMap<String, Origin>,
    checked_at: Option<chrono::DateTime<chrono::Utc>>,
    updates: Vec<Update>,
    /// Repositories that couldn't be checked last time, and why.
    errors: Vec<String>,
}

fn key(repo: &str, path: &str) -> String {
    format!("{repo}:{path}")
}

/// The version of a tree entry.
pub fn remote_version(e: &Value) -> Option<Version> {
    let oid = e
        .get("lfs")
        .and_then(|l| l.get("oid"))
        .or_else(|| e.get("oid"))
        .and_then(Value::as_str)?;
    Some(Version {
        oid: oid.to_string(),
        size: hub::entry_size(e),
    })
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// The file on the PC is the version upstream.
    Current,
    /// Taken as current (same size, no note yet); note the version.
    Adopt,
    /// Upstream has a different version.
    Changed,
}

/// Compares a file on the PC (its size, and its noted version if any) with
/// the version upstream.
pub fn compare(local_size: u64, known: Option<&Version>, remote: &Version) -> Verdict {
    match known {
        Some(k) if k.oid == remote.oid => Verdict::Current,
        Some(_) => Verdict::Changed,
        None if local_size == remote.size => Verdict::Adopt,
        None => Verdict::Changed,
    }
}

/// A downloaded model on the PC, with where its files came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub kind: &'static str,
    pub path: String,
    pub name: String,
    pub repo: String,
    /// (path in the repository, file on the PC)
    pub files: Vec<(String, PathBuf)>,
}

fn rel_parts(root: &Path, file: &Path) -> Option<Vec<String>> {
    let r = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let f = dunce::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let rel = f.strip_prefix(&r).ok()?;
    rel.components()
        .map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// `<owner>/<repo>/<path>` under a library: (repo, path).
fn origin_from_path(root: &Path, file: &Path) -> Option<(String, String)> {
    let parts = rel_parts(root, file)?;
    if parts.len() < 3 || parts[0].eq_ignore_ascii_case("imported") {
        return None;
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    let path = parts[2..].join("/");
    (hub::valid_repo(&repo) && hub::valid_path(&path)).then_some((repo, path))
}

/// Chat models in the libraries that came from Hugging Face.
pub fn chat_models(dirs: &[PathBuf]) -> Vec<Local> {
    let mut out = Vec::new();
    for m in crate::setup::discover_models(dirs) {
        let path = PathBuf::from(&m.path);
        let Some(root) = dirs.iter().find(|d| crate::libraries::contains(d, &path)) else {
            continue;
        };
        let Some((repo, _)) = origin_from_path(root, &path) else {
            continue;
        };
        let mut files: Vec<(String, PathBuf)> = crate::libraries::model_parts(&path)
            .into_iter()
            .filter_map(|p| Some((origin_from_path(root, &p)?.1, p)))
            .collect();
        // The vision file next to it.
        if let Some(dir) = path.parent()
            && let Ok(rd) = std::fs::read_dir(dir)
        {
            let mut projectors: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    p.is_file() && n.starts_with("mmproj") && n.ends_with(".gguf")
                })
                .collect();
            projectors.sort();
            for p in projectors {
                if let Some((r, rp)) = origin_from_path(root, &p)
                    && r == repo
                {
                    files.push((rp, p));
                }
            }
        }
        out.push(Local {
            kind: "chat",
            path: m.path,
            name: m.name,
            repo,
            files,
        });
    }
    out
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Image model files in Relay's image model folder whose origin is known.
fn image_files(own: &Path, origins: &BTreeMap<String, Origin>) -> Vec<Local> {
    use crate::imagegen::hub::STARTERS;
    let mut out = Vec::new();
    for cat in crate::imagegen::engine::MODEL_CATEGORIES {
        let Ok(rd) = std::fs::read_dir(own.join(cat)) else {
            continue;
        };
        let mut names: Vec<(String, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        names.sort();
        for (name, path) in names {
            let id = format!("{cat}/{name}");
            let origin = origins.get(&id).cloned().or_else(|| {
                STARTERS
                    .iter()
                    .flat_map(|s| s.small.iter().chain(s.big))
                    .find(|f| f.category == *cat && file_name(f.path) == name)
                    .map(|f| Origin {
                        repo: f.repo.to_string(),
                        path: f.path.to_string(),
                    })
            });
            let Some(o) = origin else { continue };
            out.push(Local {
                kind: "image",
                path: id,
                name,
                repo: o.repo,
                files: vec![(o.path, path)],
            });
        }
    }
    out
}

/// `<file>.update`: where the new version downloads to.
fn staged(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".update");
    PathBuf::from(s)
}

pub struct ModelUpdates {
    cfg: Arc<SharedConfig>,
    file: PathBuf,
    saved: Mutex<Saved>,
    checking: AtomicBool,
    /// One check at a time.
    check_lock: tokio::sync::Mutex<()>,
}

impl ModelUpdates {
    pub fn new(cfg: Arc<SharedConfig>) -> Arc<Self> {
        let file = cfg.read(|l| l.data_dir.join("model-updates.json"));
        let saved = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Arc::new(Self {
            cfg,
            file,
            saved: Mutex::new(saved),
            checking: AtomicBool::new(false),
            check_lock: tokio::sync::Mutex::new(()),
        })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Saved) -> R) -> R {
        let mut s = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut s);
        if let Ok(b) = serde_json::to_vec_pretty(&*s)
            && let Err(e) = std::fs::write(&self.file, b)
        {
            tracing::warn!("could not save {}: {e}", self.file.display());
        }
        r
    }

    fn read<R>(&self, f: impl FnOnce(&Saved) -> R) -> R {
        f(&self.saved.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Notes the version of files being downloaded now, from the
    /// repository tree they were picked from.
    pub fn remember(&self, repo: &str, paths: &[String], tree: &[Value]) {
        let found: Vec<(String, Version)> = paths
            .iter()
            .filter_map(|p| {
                let e = tree
                    .iter()
                    .find(|e| e.get("path").and_then(Value::as_str) == Some(p.as_str()))?;
                Some((key(repo, p), remote_version(e)?))
            })
            .collect();
        if found.is_empty() {
            return;
        }
        self.with(|s| s.known.extend(found));
    }

    /// Notes where an image model file (`category/name`) came from.
    pub fn remember_image_origin(&self, id: &str, repo: &str, path: &str) {
        let o = Origin {
            repo: repo.to_string(),
            path: path.to_string(),
        };
        self.with(|s| {
            s.image_origins.insert(id.to_string(), o);
        });
    }

    pub fn updates(&self) -> Vec<Update> {
        self.read(|s| s.updates.clone())
    }

    pub fn checking(&self) -> bool {
        self.checking.load(Ordering::SeqCst)
    }

    /// Looks for newer versions of every downloaded model.
    pub async fn check(&self) -> Result<Vec<Update>> {
        let _one = self.check_lock.lock().await;
        self.checking.store(true, Ordering::SeqCst);
        let r = self.check_now().await;
        self.checking.store(false, Ordering::SeqCst);
        r
    }

    async fn check_now(&self) -> Result<Vec<Update>> {
        let (dirs, own) = self.cfg.read(|l| (l.model_dirs(), l.image_models_dir()));
        let origins = self.read(|s| s.image_origins.clone());
        let locals = tokio::task::spawn_blocking(move || {
            let mut v = chat_models(&dirs);
            v.extend(image_files(&own, &origins));
            v
        })
        .await?;

        let mut trees: HashMap<String, Option<Vec<Value>>> = HashMap::new();
        let mut errors = Vec::new();
        for l in &locals {
            if trees.contains_key(&l.repo) {
                continue;
            }
            let t = match hub::tree(&l.repo).await {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::info!("model update check: {}: {e:#}", l.repo);
                    errors.push(format!("{}: {e:#}", l.repo));
                    None
                }
            };
            trees.insert(l.repo.clone(), t);
        }

        let known = self.read(|s| s.known.clone());
        let mut adopt = Vec::new();
        let mut updates = Vec::new();
        for l in &locals {
            let Some(Some(tree)) = trees.get(&l.repo) else {
                continue;
            };
            let mut files = Vec::new();
            for (remote, local) in &l.files {
                let Some(v) = tree
                    .iter()
                    .find(|e| e.get("path").and_then(Value::as_str) == Some(remote.as_str()))
                    .and_then(remote_version)
                else {
                    // Gone upstream: nothing to update it to.
                    continue;
                };
                let size = local.metadata().map(|m| m.len()).unwrap_or(0);
                let k = key(&l.repo, remote);
                match compare(size, known.get(&k), &v) {
                    Verdict::Current => {}
                    Verdict::Adopt => adopt.push((k, v)),
                    Verdict::Changed => files.push(UpdateFile {
                        remote: remote.clone(),
                        local: local.to_string_lossy().into_owned(),
                        size_bytes: v.size,
                        oid: v.oid,
                    }),
                }
            }
            if !files.is_empty() {
                updates.push(Update {
                    kind: l.kind.to_string(),
                    path: l.path.clone(),
                    name: l.name.clone(),
                    repo: l.repo.clone(),
                    size_bytes: files.iter().map(|f| f.size_bytes).sum(),
                    files,
                });
            }
        }
        let found = updates.clone();
        self.with(move |s| {
            s.known.extend(adopt);
            s.updates = updates;
            s.errors = errors;
            s.checked_at = Some(chrono::Utc::now());
        });
        if !found.is_empty() {
            tracing::info!("{} model update(s) available", found.len());
        }
        Ok(found)
    }

    /// Notes an update's files as current and drops the update.
    fn settle(&self, u: &Update) {
        let u = u.clone();
        self.with(move |s| {
            for f in &u.files {
                s.known.insert(
                    key(&u.repo, &f.remote),
                    Version {
                        oid: f.oid.clone(),
                        size: f.size_bytes,
                    },
                );
            }
            s.updates.retain(|x| x.path != u.path);
        });
    }

    /// Checks once a day while Relay runs.
    pub fn spawn(self: &Arc<Self>, shutdown: tokio_util::sync::CancellationToken) {
        let this = self.clone();
        tokio::spawn(async move {
            // Let Relay start up first.
            let mut wait = Duration::from_secs(180);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(wait) => {}
                }
                wait = Duration::from_secs(3600);
                let due = this
                    .read(|s| s.checked_at)
                    .is_none_or(|t| chrono::Utc::now() - t >= CHECK_EVERY);
                if due && let Err(e) = this.check().await {
                    tracing::info!("model update check failed: {e:#}");
                }
            }
        });
    }
}

// ---------- routes ----------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/models/updates", get(list))
        .route("/models/updates/apply", post(apply))
        .route("/models/updates/skip", post(skip))
}

fn updating(s: &AppState, u: &Update) -> bool {
    s.downloads.list().iter().any(|d| {
        d.state == DownloadState::Downloading
            && d.group.as_deref() == Some(GROUP)
            && d.repo == u.repo
            && d.file == u.name
    })
}

fn view(s: &AppState) -> Value {
    let (checked_at, errors) = s
        .model_updates
        .read(|v| (v.checked_at, v.errors.clone()));
    let updates: Vec<Value> = s
        .model_updates
        .updates()
        .iter()
        .map(|u| {
            let mut v = serde_json::to_value(u).unwrap_or_default();
            v["updating"] = json!(updating(s, u));
            v
        })
        .collect();
    json!({
        "updates": updates,
        "checked_at": checked_at,
        "checking": s.model_updates.checking(),
        "errors": errors,
    })
}

#[derive(Deserialize)]
struct ListQuery {
    /// Check Hugging Face now instead of showing the last check.
    #[serde(default)]
    refresh: Option<String>,
}

async fn list(State(s): State<AppState>, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    if q.refresh.as_deref().is_some_and(|r| r == "1" || r == "true") {
        s.model_updates.check().await?;
    }
    Ok(Json(view(&s)))
}

#[derive(Deserialize)]
struct UpdateBody {
    /// The update's `path`.
    path: String,
}

fn find(s: &AppState, path: &str) -> ApiResult<Update> {
    s.model_updates
        .updates()
        .into_iter()
        .find(|u| u.path == path)
        .ok_or(ApiError::NotFound("model update"))
}

/// Doesn't offer this version again (a newer one is offered).
async fn skip(State(s): State<AppState>, JsonBody(b): JsonBody<UpdateBody>) -> ApiResult<Json<Value>> {
    let u = find(&s, &b.path)?;
    s.model_updates.settle(&u);
    Ok(Json(view(&s)))
}

/// Downloads the new version next to the old one, then swaps it in once the
/// model isn't being used.
async fn apply(
    State(s): State<AppState>,
    JsonBody(b): JsonBody<UpdateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mut u = find(&s, &b.path)?;
    if updating(&s, &u) {
        return Err(ApiError::Conflict("that model is already updating".into()));
    }
    // Take the version upstream now (it may have changed again).
    let tree = hub::tree(&u.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    for f in &mut u.files {
        let v = tree
            .iter()
            .find(|e| e.get("path").and_then(Value::as_str) == Some(f.remote.as_str()))
            .and_then(remote_version)
            .ok_or_else(|| {
                ApiError::BadRequest(format!("{} is no longer in {} on Hugging Face", f.remote, u.repo))
            })?;
        f.oid = v.oid;
        f.size_bytes = v.size;
    }
    u.size_bytes = u.files.iter().map(|f| f.size_bytes).sum();
    let need = u.size_bytes;
    let room = {
        let dir = PathBuf::from(&u.files[0].local)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        tokio::task::spawn_blocking(move || crate::libraries::disk(&dir))
            .await
            .ok()
            .flatten()
    };
    if let Some(d) = room
        && d.free_bytes < need + 512 * 1024 * 1024
    {
        return Err(ApiError::BadRequest(format!(
            "not enough space on {}: the new version needs {:.1} GB and {:.1} GB is free",
            d.root,
            need as f64 / 1e9,
            d.free_bytes as f64 / 1e9
        )));
    }
    let mut files = Vec::new();
    for f in &u.files {
        let to = staged(Path::new(&f.local));
        // Left over from an earlier try: it may be an older version.
        let _ = tokio::fs::remove_file(&to).await;
        files.push((f.remote.clone(), to));
    }
    let d = s
        .downloads
        .start_files(&u.repo, &u.name, files, need, Some(GROUP.into()))
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    let (st, id, upd) = (s.clone(), d.id.clone(), u.clone());
    tokio::spawn(async move {
        if let Err(e) = swap_in(&st, &id, &upd).await {
            tracing::warn!("updating {} failed: {e:#}", upd.name);
        }
        for f in &upd.files {
            let _ = tokio::fs::remove_file(staged(Path::new(&f.local))).await;
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "download": d }))))
}

async fn swap_in(s: &AppState, id: &str, u: &Update) -> Result<()> {
    loop {
        match s.downloads.state(id) {
            Some(DownloadState::Downloading) => tokio::time::sleep(Duration::from_secs(2)).await,
            Some(DownloadState::Done) => break,
            _ => anyhow::bail!("the download didn't finish"),
        }
    }
    // Free the files: the model can't be using them while they're replaced.
    if u.kind == "image" {
        while s.images.busy() {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        s.images.unload().await;
    } else {
        let loaded = |s: &AppState| {
            let st = s.model.status();
            st.state != crate::model_manager::ModelState::Idle
                && !st.model_path.is_empty()
                && crate::libraries::same_path(Path::new(&st.model_path), Path::new(&u.path))
        };
        if loaded(s) {
            while s.model.in_use() {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            s.model.unload_and_wait().await;
        }
    }
    for f in &u.files {
        let (from, to) = (staged(Path::new(&f.local)), PathBuf::from(&f.local));
        let mut attempt = 0;
        loop {
            match tokio::fs::rename(&from, &to).await {
                Ok(()) => break,
                // The model may hold the file for a moment after it stops.
                Err(_) if attempt < 20 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
                Err(e) => anyhow::bail!("could not replace {}: {e}", to.display()),
            }
        }
    }
    if u.kind == "image" {
        s.images.files_changed();
    }
    s.model_updates.settle(u);
    tracing::info!("updated {} from {}", u.name, u.repo);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(oid: &str, size: u64) -> Version {
        Version {
            oid: oid.into(),
            size,
        }
    }

    #[test]
    fn compares_versions() {
        let up = v("new", 10);
        assert_eq!(compare(10, Some(&v("new", 10)), &up), Verdict::Current);
        assert_eq!(compare(10, Some(&v("old", 10)), &up), Verdict::Changed);
        assert_eq!(compare(10, None, &up), Verdict::Adopt);
        assert_eq!(compare(9, None, &up), Verdict::Changed);
    }

    #[test]
    fn reads_versions_from_the_tree() {
        let lfs = json!({"type": "file", "path": "a.gguf", "oid": "blob", "size": 130,
            "lfs": {"oid": "sha", "size": 5000, "pointerSize": 130}});
        assert_eq!(remote_version(&lfs), Some(v("sha", 5000)));
        let small = json!({"type": "file", "path": "a.json", "oid": "blob", "size": 12});
        assert_eq!(remote_version(&small), Some(v("blob", 12)));
        assert_eq!(remote_version(&json!({"path": "x"})), None);
    }

    #[test]
    fn finds_where_chat_models_came_from() {
        let root = std::env::temp_dir().join(format!("relay-upd-{}", uuid::Uuid::new_v4()));
        let dir = root.join("unsloth/Qwen3-GGUF");
        std::fs::create_dir_all(dir.join("Q8_0")).unwrap();
        std::fs::create_dir_all(root.join("imported")).unwrap();
        std::fs::write(dir.join("Qwen3-Q4_K_M.gguf"), b"m").unwrap();
        std::fs::write(dir.join("mmproj-F16.gguf"), b"p").unwrap();
        std::fs::write(dir.join("Q8_0/Qwen3-Q8_0-00001-of-00002.gguf"), b"a").unwrap();
        std::fs::write(dir.join("Q8_0/Qwen3-Q8_0-00002-of-00002.gguf"), b"b").unwrap();
        std::fs::write(root.join("imported/mine.gguf"), b"i").unwrap();
        let mut found = chat_models(std::slice::from_ref(&root));
        found.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(found.len(), 2, "{found:?}");
        let remote = |l: &Local| l.files.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
        assert_eq!(found[0].repo, "unsloth/Qwen3-GGUF");
        assert_eq!(remote(&found[0]), ["Qwen3-Q4_K_M.gguf", "mmproj-F16.gguf"]);
        assert_eq!(
            remote(&found[1]),
            ["Q8_0/Qwen3-Q8_0-00001-of-00002.gguf", "Q8_0/Qwen3-Q8_0-00002-of-00002.gguf"]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn knows_starter_image_files() {
        let own = std::env::temp_dir().join(format!("relay-upd-img-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(own.join("vae")).unwrap();
        std::fs::create_dir_all(own.join("loras")).unwrap();
        std::fs::write(own.join("vae/ae.safetensors"), b"v").unwrap();
        std::fs::write(own.join("loras/style.safetensors"), b"l").unwrap();
        std::fs::write(own.join("loras/unknown.safetensors"), b"u").unwrap();
        let mut origins = BTreeMap::new();
        origins.insert(
            "loras/style.safetensors".to_string(),
            Origin {
                repo: "someone/styles".into(),
                path: "style.safetensors".into(),
            },
        );
        let found = image_files(&own, &origins);
        let ids: Vec<_> = found.iter().map(|l| (l.path.as_str(), l.repo.as_str())).collect();
        assert_eq!(
            ids,
            [
                ("vae/ae.safetensors", "Comfy-Org/z_image_turbo"),
                ("loras/style.safetensors", "someone/styles")
            ]
        );
        let _ = std::fs::remove_dir_all(&own);
    }
}
