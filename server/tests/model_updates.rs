//! Model updates end to end, against a tiny Hugging Face whose files change.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use relay::config::LoadedConfig;

/// (repo, path, contents); a file's version is its contents.
type Files = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

async fn mock_hf(files: Files) -> String {
    use axum::extract::Path;
    use axum::routing::get;
    let tree_files = files.clone();
    let app = axum::Router::new()
        .route(
            "/api/models/{owner}/{name}/tree/main",
            get(move |Path((o, n)): Path<(String, String)>| {
                let files = tree_files.clone();
                async move {
                    let repo = format!("{o}/{n}");
                    let entries: Vec<Value> = files
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(r, _, _)| *r == repo)
                        .map(|(_, p, b)| {
                            json!({"type": "file", "path": p, "size": 130, "oid": "blob",
                                "lfs": {"oid": String::from_utf8_lossy(b), "size": b.len()}})
                        })
                        .collect();
                    axum::Json(Value::Array(entries))
                }
            }),
        )
        .route(
            "/{owner}/{name}/resolve/main/{*path}",
            get(move |Path((o, n, p)): Path<(String, String, String)>| {
                let files = files.clone();
                async move {
                    let repo = format!("{o}/{n}");
                    files
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(r, f, _)| *r == repo && *f == p)
                        .map(|(_, _, b)| b.clone())
                        .ok_or(axum::http::StatusCode::NOT_FOUND)
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(axum::serve(l, app).into_future());
    url
}

fn set(files: &Files, repo: &str, path: &str, body: &[u8]) {
    let mut f = files.lock().unwrap();
    f.retain(|(r, p, _)| !(r == repo && p == path));
    f.push((repo.into(), path.into(), body.to_vec()));
}

struct H {
    base: String,
    token: String,
    http: reqwest::Client,
    dir: std::path::PathBuf,
}

impl H {
    async fn start() -> Self {
        let llama = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let llama_port = llama.local_addr().unwrap().port();
        tokio::spawn(common::mock_llama::serve(llama, Default::default()));
        let sd = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let sd_port = sd.local_addr().unwrap().port();
        tokio::spawn(common::mock_sd::serve(sd, Default::default()));
        let dir = std::env::temp_dir().join(format!("relay-upd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let cfg_path = dir.join("data/config.toml");
        std::fs::write(
            &cfg_path,
            format!(
                "bind = \"127.0.0.1:0\"\n[llama]\nport = {llama_port}\n[images]\nport = {sd_port}\n[push]\nenabled = false\n"
            ),
        )
        .unwrap();
        let cfg = LoadedConfig::load(&cfg_path).unwrap();
        let token = cfg.config.token.clone();
        let (app, _state) = relay::build_app(cfg).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(axum::serve(listener, app).into_future());
        H {
            base: format!("http://127.0.0.1:{port}"),
            token,
            http: reqwest::Client::new(),
            dir,
        }
    }

    async fn req(&self, m: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut r = self
            .http
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let s = resp.status().as_u16();
        (s, resp.json().await.unwrap_or(Value::Null))
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.req(reqwest::Method::POST, path, Some(body)).await
    }

    async fn check(&self) -> Vec<Value> {
        let (s, v) = self
            .req(reqwest::Method::GET, "/models/updates?refresh=1", None)
            .await;
        assert_eq!(s, 200, "{v}");
        assert!(v["checked_at"].is_string(), "{v}");
        v["updates"].as_array().unwrap().clone()
    }
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what} never happened");
}

#[tokio::test]
async fn offers_and_installs_newer_model_versions() {
    let files: Files = Default::default();
    set(&files, "acme/Tiny-GGUF", "tiny-Q4_K_M.gguf", b"chat-v1");
    set(&files, "Comfy-Org/z_image_turbo", "split_files/vae/ae.safetensors", b"vae-v2");
    set(&files, "someone/styles", "style.safetensors", b"style-v1");
    let hf = mock_hf(files.clone()).await;
    // SAFETY: only this test (its own binary) talks to Hugging Face.
    unsafe { std::env::set_var("RELAY_HF_URL", &hf) };
    let h = H::start().await;
    let chat = h.dir.join("data/models/acme/Tiny-GGUF/tiny-Q4_K_M.gguf");
    let own = h.dir.join("data/image-models");

    // A model downloaded through Relay is current.
    let (s, v) = h
        .post("/hub/downloads", json!({"repo": "acme/Tiny-GGUF", "file": "tiny-Q4_K_M.gguf"}))
        .await;
    assert_eq!(s, 202, "{v}");
    wait_for("the download", || chat.is_file()).await;
    let (s, v) = h
        .post(
            "/images/hub/download",
            json!({"repo": "someone/styles", "file": "style.safetensors", "category": "loras"}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    wait_for("the image file download", || own.join("loras/style.safetensors").is_file()).await;
    // A starter file from before Relay kept notes, a different size from
    // the one upstream: it's out of date.
    std::fs::create_dir_all(own.join("vae")).unwrap();
    std::fs::write(own.join("vae/ae.safetensors"), b"vae-v1-old").unwrap();

    let ups = h.check().await;
    assert_eq!(ups.len(), 1, "{ups:?}");
    assert_eq!(ups[0]["kind"], "image");
    assert_eq!(ups[0]["path"], "vae/ae.safetensors");
    assert_eq!(ups[0]["repo"], "Comfy-Org/z_image_turbo");

    // Skipping it: not offered again until it changes again.
    let (s, v) = h.post("/models/updates/skip", json!({"path": "vae/ae.safetensors"})).await;
    assert_eq!(s, 200, "{v}");
    assert!(h.check().await.is_empty());

    // New versions upstream, same size or not.
    set(&files, "acme/Tiny-GGUF", "tiny-Q4_K_M.gguf", b"chat-v2");
    set(&files, "someone/styles", "style.safetensors", b"style-v2-bigger");
    let ups = h.check().await;
    assert_eq!(ups.len(), 2, "{ups:?}");
    let chat_up = ups.iter().find(|u| u["kind"] == "chat").unwrap();
    assert_eq!(chat_up["name"], "tiny-Q4_K_M.gguf");
    assert_eq!(chat_up["repo"], "acme/Tiny-GGUF");
    assert_eq!(chat_up["size_bytes"], 7);
    let path = chat_up["path"].as_str().unwrap().to_string();

    // Updating swaps the new version in and settles it.
    let (s, v) = h.post("/models/updates/apply", json!({"path": path})).await;
    assert_eq!(s, 202, "{v}");
    assert_eq!(v["download"]["repo"], "acme/Tiny-GGUF");
    wait_for("the swap", || std::fs::read(&chat).unwrap() == b"chat-v2").await;
    let (s, v) = h.post("/models/updates/apply", json!({"path": "loras/style.safetensors"})).await;
    assert_eq!(s, 202, "{v}");
    wait_for("the image swap", || {
        std::fs::read(own.join("loras/style.safetensors")).unwrap() == b"style-v2-bigger"
    })
    .await;
    for _ in 0..100 {
        let (_, v) = h.req(reqwest::Method::GET, "/models/updates", None).await;
        if v["updates"].as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(h.check().await.is_empty());
    assert!(!h.dir.join("data/models/acme/Tiny-GGUF/tiny-Q4_K_M.gguf.update").exists());
    // Only the model shows in the list, not the staging file.
    let (_, v) = h.req(reqwest::Method::GET, "/models", None).await;
    assert_eq!(v["models"].as_array().unwrap().len(), 1, "{v}");

    let (s, _) = h.post("/models/updates/apply", json!({"path": "nope"})).await;
    assert_eq!(s, 404);
}
