//! Image model downloads from Hugging Face, saved into Relay's own image
//! model folder (`<data>/image-models/<category>/`):
//!
//! - Starter models: every file a model type needs, in one tap, with the
//!   quantization picked for the GPU. Set up once the files are in.
//! - Any file from any repository, for everything else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::catalog::{self, ModelFile};
use super::engine::MODEL_CATEGORIES;
use super::store;
use crate::api::{ApiError, AppState, JsonBody};
use crate::hub::{self, DownloadState};

type ApiResult<T> = Result<T, ApiError>;

/// One file of a starter model.
#[derive(Debug, Clone, Copy)]
pub struct StarterFile {
    /// The recipe slot it fills.
    pub slot: &'static str,
    /// The folder it goes in.
    pub category: &'static str,
    pub repo: &'static str,
    /// Path inside the repository.
    pub path: &'static str,
    /// Size on Hugging Face (checked again when downloading).
    pub size: u64,
}

/// A model type with every file it needs, in a smaller and a bigger
/// version.
#[derive(Debug, Clone, Copy)]
pub struct Starter {
    pub id: &'static str,
    /// sd-server recipe.
    pub recipe: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub edit: bool,
    /// GPUs with at least this much VRAM get the bigger version.
    pub big_vram_mb: u64,
    pub small: &'static [StarterFile],
    pub big: &'static [StarterFile],
}

const Z_IMAGE_VAE: StarterFile = StarterFile {
    slot: "vae",
    category: "vae",
    repo: "Comfy-Org/z_image_turbo",
    path: "split_files/vae/ae.safetensors",
    size: 335_304_388,
};

const FLUX2_VAE: StarterFile = StarterFile {
    slot: "vae",
    category: "vae",
    repo: "Comfy-Org/flux2-klein-4B",
    path: "split_files/vae/flux2-vae.safetensors",
    size: 336_211_292,
};

pub const STARTERS: &[Starter] = &[
    Starter {
        id: "z-image-turbo",
        recipe: "z-image-turbo",
        name: "Z-Image Turbo",
        blurb: "Fast, sharp photos and art. The best first model.",
        edit: false,
        big_vram_mb: 16_000,
        small: &[
            StarterFile {
                slot: "diffusion_model",
                category: "diffusion_models",
                repo: "leejet/Z-Image-Turbo-GGUF",
                path: "z_image_turbo-Q4_K.gguf",
                size: 3_864_250_304,
            },
            StarterFile {
                slot: "llm",
                category: "text_encoders",
                repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
                path: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
                size: 2_497_281_120,
            },
            Z_IMAGE_VAE,
        ],
        big: &[
            StarterFile {
                slot: "diffusion_model",
                category: "diffusion_models",
                repo: "leejet/Z-Image-Turbo-GGUF",
                path: "z_image_turbo-Q8_0.gguf",
                size: 6_577_440_704,
            },
            StarterFile {
                slot: "llm",
                category: "text_encoders",
                repo: "unsloth/Qwen3-4B-Instruct-2507-GGUF",
                path: "Qwen3-4B-Instruct-2507-Q8_0.gguf",
                size: 4_280_405_600,
            },
            Z_IMAGE_VAE,
        ],
    },
    Starter {
        id: "flux2-klein-4b",
        recipe: "flux2-klein-4b",
        name: "FLUX.2 Klein 4B",
        blurb: "Makes pictures and edits the one you're working on.",
        edit: true,
        big_vram_mb: 12_000,
        small: &[
            StarterFile {
                slot: "diffusion_model",
                category: "diffusion_models",
                repo: "leejet/FLUX.2-klein-4B-GGUF",
                path: "flux-2-klein-4b-Q4_0.gguf",
                size: 2_460_378_560,
            },
            StarterFile {
                slot: "llm",
                category: "text_encoders",
                repo: "unsloth/Qwen3-4B-GGUF",
                path: "Qwen3-4B-Q4_K_M.gguf",
                size: 2_497_281_312,
            },
            FLUX2_VAE,
        ],
        big: &[
            StarterFile {
                slot: "diffusion_model",
                category: "diffusion_models",
                repo: "leejet/FLUX.2-klein-4B-GGUF",
                path: "flux-2-klein-4b-Q8_0.gguf",
                size: 4_300_629_440,
            },
            StarterFile {
                slot: "llm",
                category: "text_encoders",
                repo: "unsloth/Qwen3-4B-GGUF",
                path: "Qwen3-4B-Q8_0.gguf",
                size: 4_280_405_792,
            },
            FLUX2_VAE,
        ],
    },
];

pub fn starter(id: &str) -> Option<&'static Starter> {
    STARTERS.iter().find(|s| s.id == id)
}

impl Starter {
    /// The version for a GPU with `vram_mb` (0 when unknown: the smaller).
    pub fn files_for(&self, vram_mb: u64, big: Option<bool>) -> &'static [StarterFile] {
        if big.unwrap_or(vram_mb >= self.big_vram_mb) {
            self.big
        } else {
            self.small
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Where a file from Hugging Face is saved: `<own>/<category>/<file name>`.
pub fn dest(own: &Path, category: &str, path: &str) -> PathBuf {
    own.join(category).join(file_name(path))
}

/// Download group of a starter's files.
fn group(id: &str) -> String {
    format!("image:{id}")
}

/// Download group of single files.
const FILE_GROUP: &str = "image-file";

/// Picks the folder a file most likely goes in, from its path.
pub fn guess_category(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let name = file_name(&lower);
    let has = |w: &str| lower.contains(w);
    if has("lora") {
        "loras"
    } else if has("vae") || name.starts_with("ae.") || has("taesd") || name.starts_with("tae") {
        "vae"
    } else if has("text_encoder")
        || ["qwen", "t5", "clip", "llm", "mistral", "gemma", "llama", "umt5", "byt5"]
            .iter()
            .any(|w| name.contains(w))
    {
        "text_encoders"
    } else if has("checkpoint")
        || ["sd_xl", "sdxl", "sd15", "sd_1", "v1-5", "sd-v1", "sd2", "pony", "illustrious"]
            .iter()
            .any(|w| name.contains(w))
    {
        "checkpoints"
    } else {
        "diffusion_models"
    }
}

const FILE_EXTS: &[&str] = &[".gguf", ".safetensors", ".sft", ".ckpt"];

/// Model files in a repository tree: (path, size), biggest folders' shards
/// of diffusers-style models left out (sd-server can't use them).
pub fn model_files(tree: &[Value]) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = tree
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("file"))
        .filter_map(|e| {
            let path = e.get("path").and_then(Value::as_str)?;
            let lower = path.to_ascii_lowercase();
            if !hub::valid_path(path)
                || !FILE_EXTS.iter().any(|x| lower.ends_with(x))
                || lower.contains("-of-0")
                || lower.contains("mmproj")
            {
                return None;
            }
            Some((path.to_string(), hub::entry_size(e)))
        })
        .collect();
    out.sort();
    out
}

/// The sd-server file that is `<category>/<name>`, if it's been scanned.
pub fn find_file<'a>(files: &'a [ModelFile], category: &str, name: &str) -> Option<&'a ModelFile> {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    let want = norm(name);
    files.iter().find(|f| {
        f.category == category && {
            let n = norm(&f.name);
            let id = norm(&f.id);
            [n, id]
                .iter()
                .any(|s| s == &want || s.ends_with(&format!("/{want}")))
        }
    })
}

// ---------- routes ----------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/images/downloads", get(list_starters))
        .route("/images/downloads/{id}", post(download_starter))
        .route("/images/hub/search", get(search))
        .route("/images/hub/files", get(repo_files))
        .route("/images/hub/download", post(download_file))
}

#[derive(Serialize)]
struct FileView {
    slot: &'static str,
    category: &'static str,
    repo: &'static str,
    name: &'static str,
    size_bytes: u64,
    have: bool,
}

#[derive(Serialize)]
struct StarterView {
    id: &'static str,
    recipe: &'static str,
    name: &'static str,
    blurb: &'static str,
    edit: bool,
    /// The bigger, sharper version (this GPU has room for it).
    big: bool,
    files: Vec<FileView>,
    total_bytes: u64,
    missing_bytes: u64,
    /// Set up already (an image model of this type uses these files).
    set_up: bool,
    downloading: bool,
    /// Why the last try failed.
    error: Option<String>,
}

fn own_dir(s: &AppState) -> PathBuf {
    s.cfg.read(|l| l.image_models_dir())
}

async fn list_starters(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let own = own_dir(&s);
    let vram = s.hardware.total_vram_mb();
    let saved = s.images.db().call(|c| store::list_models(c)).await?;
    let downloads = s.downloads.list();
    let starters: Vec<StarterView> = STARTERS
        .iter()
        .map(|st| {
            let files: Vec<FileView> = st
                .files_for(vram, None)
                .iter()
                .map(|f| FileView {
                    slot: f.slot,
                    category: f.category,
                    repo: f.repo,
                    name: file_name(f.path),
                    size_bytes: f.size,
                    have: dest(&own, f.category, f.path).is_file(),
                })
                .collect();
            let g = group(st.id);
            let mine: Vec<_> = downloads
                .iter()
                .filter(|d| d.group.as_deref() == Some(g.as_str()))
                .collect();
            let primary = files.iter().find(|f| f.slot == "diffusion_model" || f.slot == "model");
            StarterView {
                id: st.id,
                recipe: st.recipe,
                name: st.name,
                blurb: st.blurb,
                edit: st.edit,
                big: files.iter().zip(st.big).all(|(a, b)| a.name == file_name(b.path)),
                total_bytes: files.iter().map(|f| f.size_bytes).sum(),
                missing_bytes: files.iter().filter(|f| !f.have).map(|f| f.size_bytes).sum(),
                set_up: saved.iter().any(|m| {
                    m.recipe == st.recipe
                        && primary.is_some_and(|p| {
                            m.files.get(p.slot).is_some_and(|id| {
                                id.replace('\\', "/").to_lowercase().ends_with(&p.name.to_lowercase())
                            })
                        })
                }),
                downloading: mine.iter().any(|d| d.state == DownloadState::Downloading),
                error: mine
                    .iter()
                    .find(|d| d.state == DownloadState::Failed)
                    .and_then(|d| d.error.clone()),
                files,
            }
        })
        .collect();
    Ok(Json(json!({
        "starters": starters,
        "vram_mb": vram,
        "folder": own.to_string_lossy(),
    })))
}

#[derive(Deserialize, Default)]
struct StarterBody {
    /// Force the bigger (true) or smaller (false) version.
    big: Option<bool>,
}

async fn download_starter(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    body: Option<Json<StarterBody>>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let st = starter(&id).ok_or(ApiError::NotFound("starter model"))?;
    let big = body.and_then(|b| b.0.big);
    let own = own_dir(&s);
    let files = st.files_for(s.hardware.total_vram_mb(), big);
    let g = group(st.id);
    let mut started = Vec::new();
    for f in files {
        let to = dest(&own, f.category, f.path);
        if to.is_file() {
            continue;
        }
        // Check the file is (still) there, and take its current size.
        let tree = hub::tree(f.repo)
            .await
            .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
        let size = tree
            .iter()
            .find(|e| e.get("path").and_then(Value::as_str) == Some(f.path))
            .map(hub::entry_size)
            .ok_or_else(|| {
                ApiError::BadRequest(format!("{} is no longer in {} on Hugging Face", file_name(f.path), f.repo))
            })?;
        match s.downloads.start_files(
            f.repo,
            file_name(f.path),
            vec![(f.path.to_string(), to)],
            size,
            Some(g.clone()),
        ) {
            Ok(d) => {
                s.model_updates.remember(f.repo, &[f.path.to_string()], &tree);
                started.push(d)
            }
            // Already on its way (a second tap).
            Err(e) => tracing::info!("{}: {e:#}", f.path),
        }
    }
    let ids: Vec<String> = started.iter().map(|d| d.id.clone()).collect();
    let state = s.clone();
    tokio::spawn(async move {
        if let Err(e) = set_up_when_done(&state, st, files, &ids).await {
            tracing::info!("{} not set up automatically: {e:#}", st.name);
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({ "downloads": started }))))
}

/// Waits for the downloads, then adds the model to "Your image models".
async fn set_up_when_done(
    s: &AppState,
    st: &Starter,
    files: &[StarterFile],
    ids: &[String],
) -> anyhow::Result<()> {
    loop {
        let l = s.downloads.list();
        let mine: Vec<_> = l.iter().filter(|d| ids.contains(&d.id)).collect();
        if mine.iter().any(|d| d.state == DownloadState::Downloading) {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        if mine.iter().any(|d| d.state != DownloadState::Done) {
            anyhow::bail!("a download didn't finish");
        }
        break;
    }
    // Every file there? (The others may still be coming from an earlier tap.)
    let own = own_dir(s);
    for _ in 0..3600 {
        if files.iter().all(|f| dest(&own, f.category, f.path).is_file()) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let c = s.images.catalog(true).await?;
    let Some(recipe) = c.recipes.iter().find(|r| r.id == st.recipe) else {
        anyhow::bail!("this image engine has no {} model type", st.name);
    };
    let mut map = BTreeMap::new();
    for f in files {
        let m = find_file(&c.files, f.category, file_name(f.path))
            .ok_or_else(|| anyhow::anyhow!("{} wasn't found after downloading", f.path))?;
        map.insert(f.slot.to_string(), m.id.clone());
    }
    if !catalog::complete(recipe, &map) {
        anyhow::bail!("not every part is there");
    }
    let name = st.name.to_string();
    let recipe = st.recipe.to_string();
    s.images
        .db()
        .call(move |c| {
            if store::list_models(c)?
                .iter()
                .any(|m| m.recipe == recipe && m.files == map)
            {
                return Ok(());
            }
            store::insert_model(c, &name, &recipe, &map).map(|_| ())
        })
        .await?;
    // So "Found in your folders" stops offering it.
    let _ = s.images.catalog(true).await;
    Ok(())
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

async fn search(Query(q): Query<SearchQuery>) -> ApiResult<Json<Value>> {
    let query: String = q.q.trim().chars().take(100).collect();
    if query.is_empty() {
        return Ok(Json(json!({ "results": [] })));
    }
    let hits = hub::search_with(&query, None)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    Ok(Json(json!({ "results": hits })))
}

#[derive(Deserialize)]
struct FilesQuery {
    repo: String,
}

async fn repo_files(State(s): State<AppState>, Query(q): Query<FilesQuery>) -> ApiResult<Json<Value>> {
    if !hub::valid_repo(&q.repo) {
        return Err(ApiError::BadRequest("not a Hugging Face repository name".into()));
    }
    let tree = hub::tree(&q.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let own = own_dir(&s);
    let files: Vec<Value> = model_files(&tree)
        .into_iter()
        .map(|(path, size)| {
            let category = guess_category(&path);
            json!({
                "file": path,
                "name": file_name(&path),
                "size_bytes": size,
                "quant": hub::quant_of(file_name(&path)),
                "category": category,
                "have": MODEL_CATEGORIES.iter().any(|c| dest(&own, c, &path).is_file()),
            })
        })
        .collect();
    Ok(Json(json!({ "repo": q.repo, "files": files, "categories": MODEL_CATEGORIES })))
}

#[derive(Deserialize)]
struct FileBody {
    repo: String,
    file: String,
    category: String,
}

async fn download_file(
    State(s): State<AppState>,
    JsonBody(b): JsonBody<FileBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !hub::valid_repo(&b.repo) || !hub::valid_path(&b.file) {
        return Err(ApiError::BadRequest("invalid repository or file name".into()));
    }
    if !MODEL_CATEGORIES.contains(&b.category.as_str()) {
        return Err(ApiError::BadRequest(format!("{} isn't a model folder", b.category)));
    }
    let tree = hub::tree(&b.repo)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let (path, size) = model_files(&tree)
        .into_iter()
        .find(|(p, _)| *p == b.file)
        .ok_or(ApiError::NotFound("model file"))?;
    let to = dest(&own_dir(&s), &b.category, &path);
    if to.is_file() {
        return Err(ApiError::Conflict(format!("{} is already downloaded", file_name(&path))));
    }
    let d = s
        .downloads
        .start_files(&b.repo, file_name(&path), vec![(path.clone(), to)], size, Some(FILE_GROUP.into()))
        .map_err(|e| ApiError::Conflict(format!("{e:#}")))?;
    let id = format!("{}/{}", b.category, file_name(&path));
    s.model_updates.remember_image_origin(&id, &b.repo, &path);
    s.model_updates.remember(&b.repo, std::slice::from_ref(&path), &tree);
    Ok((StatusCode::ACCEPTED, Json(json!({ "download": d }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_fill_their_recipes() {
        for st in STARTERS {
            for files in [st.small, st.big] {
                let slots: Vec<_> = files.iter().map(|f| f.slot).collect();
                assert!(slots.contains(&"diffusion_model"), "{}", st.id);
                assert!(slots.contains(&"llm"), "{}", st.id);
                assert!(slots.contains(&"vae"), "{}", st.id);
                for f in files {
                    assert!(hub::valid_repo(f.repo) && hub::valid_path(f.path));
                    assert!(MODEL_CATEGORIES.contains(&f.category));
                    assert_eq!(guess_category(f.path), f.category, "{}", f.path);
                }
            }
        }
    }

    #[test]
    fn picks_the_version_for_the_gpu() {
        let z = starter("z-image-turbo").unwrap();
        assert_eq!(z.files_for(0, None)[0].path, "z_image_turbo-Q4_K.gguf");
        assert_eq!(z.files_for(8_192, None)[0].path, "z_image_turbo-Q4_K.gguf");
        assert_eq!(z.files_for(24_576, None)[0].path, "z_image_turbo-Q8_0.gguf");
        assert_eq!(z.files_for(24_576, Some(false))[0].path, "z_image_turbo-Q4_K.gguf");
    }

    #[test]
    fn guesses_folders() {
        assert_eq!(guess_category("split_files/vae/ae.safetensors"), "vae");
        assert_eq!(guess_category("sdxl_vae.safetensors"), "vae");
        assert_eq!(guess_category("split_files/loras/x_lora_bf16.safetensors"), "loras");
        assert_eq!(guess_category("split_files/text_encoders/qwen_3_4b.safetensors"), "text_encoders");
        assert_eq!(guess_category("t5xxl_fp16.safetensors"), "text_encoders");
        assert_eq!(guess_category("sd_xl_base_1.0.safetensors"), "checkpoints");
        assert_eq!(guess_category("flux1-schnell-Q4_0.gguf"), "diffusion_models");
    }

    #[test]
    fn lists_model_files_only() {
        let tree = serde_json::json!([
            {"type": "directory", "path": "vae"},
            {"type": "file", "path": "README.md", "size": 10},
            {"type": "file", "path": "a-Q4_K.gguf", "size": 5, "lfs": {"size": 500}},
            {"type": "file", "path": "text_encoder/model-00001-of-00002.safetensors", "size": 9},
            {"type": "file", "path": "vae/ae.safetensors", "size": 7},
        ]);
        let files = model_files(tree.as_array().unwrap());
        assert_eq!(
            files,
            vec![("a-Q4_K.gguf".to_string(), 500), ("vae/ae.safetensors".to_string(), 7)]
        );
    }

    #[test]
    fn finds_downloaded_files_in_the_catalog() {
        let f = |id: &str, name: &str, cat: &str| ModelFile {
            id: id.into(),
            name: name.into(),
            category: cat.into(),
        };
        let files = vec![
            f("r0/vae/ae.safetensors", "C:\\Relay\\image-models\\vae\\ae.safetensors", "vae"),
            f("diffusion_models/z.gguf", "z.gguf", "diffusion_models"),
        ];
        assert_eq!(find_file(&files, "vae", "ae.safetensors").unwrap().id, "r0/vae/ae.safetensors");
        assert_eq!(find_file(&files, "diffusion_models", "z.gguf").unwrap().id, "diffusion_models/z.gguf");
        assert!(find_file(&files, "text_encoders", "z.gguf").is_none());
        assert!(find_file(&files, "vae", "e.safetensors").is_none());
    }
}
