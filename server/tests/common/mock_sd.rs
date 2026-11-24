//! A scripted stand-in for stable-diffusion.cpp's sd-server, used by the
//! integration tests and by `cargo run --example mock_sd` to try image
//! chats without a GPU.
//!
//! It knows two recipes (Z-Image Turbo, and FLUX.1 Kontext which can edit),
//! lists a few model files, and answers image jobs after a short "load" and
//! a few "sampling" steps with a gradient picture whose colour depends on
//! the prompt. A prompt containing "fail" fails; "slow" takes longer.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct MockSdOptions {
    /// How long each job "loads" before sampling.
    pub load_ms: u64,
    pub step_ms: u64,
}

impl Default for MockSdOptions {
    fn default() -> Self {
        Self {
            load_ms: 300,
            step_ms: 60,
        }
    }
}

struct Job {
    started: Instant,
    req: Value,
    cancelled: bool,
    steps: u64,
}

#[derive(Clone)]
struct St {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    opts: MockSdOptions,
    /// Every img_gen body received, for the tests to inspect.
    pub requests: Arc<Mutex<Vec<Value>>>,
}

/// Bodies of the img_gen requests the mock received.
#[derive(Clone, Default)]
pub struct Requests(pub Arc<Mutex<Vec<Value>>>);

pub async fn serve(listener: tokio::net::TcpListener, opts: MockSdOptions) {
    serve_recording(listener, opts, Requests::default()).await
}

pub async fn serve_recording(
    listener: tokio::net::TcpListener,
    opts: MockSdOptions,
    seen: Requests,
) {
    let st = St {
        jobs: Arc::default(),
        opts,
        requests: seen.0,
    };
    let app = Router::new()
        .route("/sdcpp/v1/recipes", get(recipes))
        .route("/sdcpp/v1/model-files", get(files))
        .route(
            "/sdcpp/v1/models/refresh",
            post(|| async { Json(json!({"errors": []})) }),
        )
        .route("/sdcpp/v1/img_gen", post(img_gen))
        .route("/sdcpp/v1/jobs/{id}", get(job))
        .route("/sdcpp/v1/jobs/{id}/cancel", post(cancel))
        .with_state(st);
    let _ = axum::serve(listener, app).await;
}

async fn recipes() -> Json<Value> {
    Json(json!({"recipes": [
        {"id": "z-image-turbo", "name": "Z-Image Turbo", "source": "builtin",
         "model_url": "https://huggingface.co/Tongyi-MAI/Z-Image-Turbo",
         "slots": [
            {"id": "diffusion_model", "required": true, "label": "Diffusion model", "category": "diffusion_models", "categories": ["diffusion_models"]},
            {"id": "llm", "required": true, "label": "Qwen 3 Text Encoder", "category": "text_encoders", "categories": ["text_encoders"]},
            {"id": "vae", "required": false, "label": "VAE", "category": "vae", "categories": ["vae"]},
            {"id": "tae", "required": false, "label": "TAE", "category": "vae", "categories": ["vae", "vae_approx"]}
         ],
         "required_any": [["vae", "tae"]],
         "defaults": {"sampling_method": "euler", "cfg_scale": 1, "steps": 9, "width": 1024, "height": 1024},
         "effective_features": {"img2img": true, "ref_images": false}},
        {"id": "flux1-kontext", "name": "FLUX.1 Kontext dev", "source": "builtin",
         "description": "Supports text-to-image and native reference-image editing.",
         "slots": [
            {"id": "diffusion_model", "required": true, "label": "Diffusion model", "category": "diffusion_models", "categories": ["diffusion_models"]},
            {"id": "clip_l", "required": true, "label": "CLIP-L", "category": "text_encoders", "categories": ["text_encoders"]},
            {"id": "t5xxl", "required": true, "label": "T5-XXL", "category": "text_encoders", "categories": ["text_encoders"]},
            {"id": "vae", "required": false, "label": "VAE", "category": "vae", "categories": ["vae"]}
         ],
         "required_any": [["vae"]],
         "defaults": {"steps": 20, "cfg_scale": 1},
         "effective_features": {"img2img": true, "ref_images": true}},
        {"id": "sdxl-checkpoint", "name": "SDXL", "slots": [
            {"id": "model", "required": true, "label": "Checkpoint", "category": "checkpoints", "categories": ["checkpoints"]}
         ], "defaults": {"width": 1024, "height": 1024}},
        {"id": "wan22-t2v", "name": "Wan 2.2", "defaults": {"video_frames": 33}, "slots": []}
    ], "errors": []}))
}

async fn files() -> Json<Value> {
    let f = |id: &str, cat: &str| {
        let name = id.rsplit('/').next().unwrap_or(id);
        json!({"id": id, "name": name, "category": cat, "root": "r0", "relative_path": id})
    };
    Json(json!({"files": [
        f("diffusion_models/z_image_turbo-Q4_K.gguf", "diffusion_models"),
        f("diffusion_models/flux1-kontext-dev-Q4_0.gguf", "diffusion_models"),
        f("text_encoders/Qwen3-4B-Instruct-2507-Q4_K_M.gguf", "text_encoders"),
        f("text_encoders/clip_l.safetensors", "text_encoders"),
        f("text_encoders/t5xxl_fp16.safetensors", "text_encoders"),
        f("vae/ae.safetensors", "vae"),
    ], "errors": []}))
}

async fn img_gen(State(st): State<St>, Json(req): Json<Value>) -> Response {
    st.requests.lock().unwrap().push(req.clone());
    if req.get("model").and_then(|m| m.get("recipe")).is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "No current model is configured", "code": "model_not_loaded"})),
        )
            .into_response();
    }
    let id = format!("job_{}", uuid::Uuid::new_v4().simple());
    let steps = req
        .pointer("/sample_params/sample_steps")
        .and_then(Value::as_u64)
        .unwrap_or(9);
    let slow = req["prompt"].as_str().unwrap_or("").contains("slow");
    st.jobs.lock().unwrap().insert(
        id.clone(),
        Job {
            started: Instant::now(),
            req,
            cancelled: false,
            steps: if slow { steps * 20 } else { steps },
        },
    );
    (
        StatusCode::ACCEPTED,
        Json(json!({"id": id, "kind": "img_gen", "status": "queued", "poll_url": format!("/sdcpp/v1/jobs/{id}")})),
    )
        .into_response()
}

async fn cancel(State(st): State<St>, Path(id): Path<String>) -> Response {
    match st.jobs.lock().unwrap().get_mut(&id) {
        Some(j) => {
            j.cancelled = true;
            Json(json!({"id": id, "cancel_requested": true})).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn job(State(st): State<St>, Path(id): Path<String>) -> Response {
    let jobs = st.jobs.lock().unwrap();
    let Some(j) = jobs.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let elapsed = j.started.elapsed().as_millis() as u64;
    let prompt = j.req["prompt"].as_str().unwrap_or("").to_string();
    if j.cancelled {
        return Json(json!({"id": id, "status": "cancelled", "result": null,
            "error": {"code": "cancelled", "message": "job cancelled by client"}}))
        .into_response();
    }
    let load = st.opts.load_ms;
    if elapsed < load {
        return Json(json!({"id": id, "status": "generating", "progress": {
            "stage": "preparing", "detail": "", "pass": 0, "completed": null, "total": null, "unit": null,
            "stage_elapsed_ms": elapsed, "elapsed_ms": elapsed}, "preview": null}))
        .into_response();
    }
    if prompt.contains("fail") {
        return Json(json!({"id": id, "status": "failed", "result": null,
            "error": {"code": "generation_failed", "message": "CUDA error: out of memory"}}))
        .into_response();
    }
    let done = (elapsed - load) / st.opts.step_ms.max(1);
    let (w, h) = (
        j.req["width"].as_u64().unwrap_or(1024).min(2048) as u32,
        j.req["height"].as_u64().unwrap_or(1024).min(2048) as u32,
    );
    if done < j.steps {
        let preview = png(
            &prompt,
            (w / 8).max(16),
            (h / 8).max(16),
            done as f32 / j.steps as f32,
        );
        return Json(json!({"id": id, "status": "generating", "progress": {
            "stage": "sampling", "detail": "", "pass": 1, "completed": done, "total": j.steps, "unit": "steps",
            "stage_elapsed_ms": elapsed - load, "elapsed_ms": elapsed},
            "preview": {"pass": 1, "step": done.max(1), "total_steps": j.steps, "b64_json": preview}}))
        .into_response();
    }
    let count = j.req["batch_count"].as_u64().unwrap_or(1).clamp(1, 4);
    let images: Vec<Value> = (0..count)
        .map(|i| json!({"index": i, "b64_json": png(&format!("{prompt}{i}"), w.min(768), h.min(768), 1.0)}))
        .collect();
    Json(json!({"id": id, "status": "completed", "progress": {"stage": "encoding", "elapsed_ms": elapsed},
        "result": {"output_format": "png", "images": images}, "error": null}))
    .into_response()
}

/// A soft two-colour gradient picked from the prompt; `sharp` 0..1 fades it
/// in from grey like a sampling preview.
fn png(prompt: &str, w: u32, h: u32, sharp: f32) -> String {
    let hash = prompt.bytes().fold(2166136261u32, |a, b| {
        (a ^ u32::from(b)).wrapping_mul(16777619)
    });
    let c1 = [
        (hash & 0xff) as f32,
        ((hash >> 8) & 0xff) as f32,
        ((hash >> 16) & 0xff) as f32,
    ];
    let c2 = [255.0 - c1[1], c1[2] * 0.6 + 60.0, 255.0 - c1[0] * 0.5];
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let t = (x as f32 / w as f32 + y as f32 / h as f32) / 2.0;
        let r = (((x as f32 / w as f32 - 0.6).powi(2) + (y as f32 / h as f32 - 0.35).powi(2))
            .sqrt()
            * 2.2)
            .min(1.0);
        let px: Vec<u8> = (0..3)
            .map(|i| {
                let v = c1[i] * (1.0 - t) + c2[i] * t;
                let v = v * (0.75 + 0.25 * (1.0 - r));
                (128.0 * (1.0 - sharp) + v * sharp).clamp(0.0, 255.0) as u8
            })
            .collect();
        image::Rgb([px[0], px[1], px[2]])
    });
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    STANDARD.encode(out.into_inner())
}
