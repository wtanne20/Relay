//! Image projects and chats end to end, against the scripted mock sd-server.

mod common;

use std::time::Duration;

use serde_json::{Value, json};

use relay::config::LoadedConfig;

struct H {
    base: String,
    token: String,
    http: reqwest::Client,
    seen: common::mock_sd::Requests,
    dir: std::path::PathBuf,
}

impl H {
    async fn start() -> Self {
        let llama = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let llama_port = llama.local_addr().unwrap().port();
        tokio::spawn(common::mock_llama::serve(llama, Default::default()));
        let sd = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let sd_port = sd.local_addr().unwrap().port();
        let seen = common::mock_sd::Requests::default();
        tokio::spawn(common::mock_sd::serve_recording(
            sd,
            common::mock_sd::MockSdOptions {
                load_ms: 100,
                step_ms: 20,
            },
            seen.clone(),
        ));

        let dir = std::env::temp_dir().join(format!("relay-img-{}", uuid::Uuid::new_v4()));
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
            seen,
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

    async fn get(&self, path: &str) -> Value {
        let (s, v) = self.req(reqwest::Method::GET, path, None).await;
        assert_eq!(s, 200, "GET {path}: {v}");
        v
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.req(reqwest::Method::POST, path, Some(body)).await
    }

    async fn wait_turn(&self, id: &str) -> Value {
        for _ in 0..200 {
            let v = self.get(&format!("/image-turns/{id}")).await;
            let st = v["turn"]["status"].as_str().unwrap_or("");
            if !["queued", "running"].contains(&st) {
                return v["turn"].clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("picture never finished");
    }
}

#[tokio::test]
async fn image_chat_makes_and_refines_pictures() {
    let h = H::start().await;

    // The catalog comes from sd-server; video recipes are left out and the
    // files found in the folders are offered as ready setups.
    let cat = h.get("/images/catalog").await;
    let ids: Vec<&str> = cat["recipes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"z-image-turbo") && !ids.contains(&"wan22-t2v"),
        "{ids:?}"
    );
    let sugg = cat["suggestions"].as_array().unwrap();
    let z = sugg
        .iter()
        .find(|s| s["recipe"] == "z-image-turbo")
        .expect("z suggestion");
    assert_eq!(z["files"]["vae"], "vae/ae.safetensors");

    // Generating before any model is set up explains what to do.
    let (s, p) = h.post("/image-projects", json!({"name": "Mascot"})).await;
    assert_eq!(s, 201, "{p}");
    let pid = p["id"].as_str().unwrap().to_string();
    let (s, chat) = h
        .post(&format!("/image-projects/{pid}/chats"), json!({}))
        .await;
    assert_eq!(s, 201, "{chat}");
    let cid = chat["id"].as_str().unwrap().to_string();
    let (s, v) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "a fox"}),
        )
        .await;
    assert_eq!(s, 400);
    assert!(v["error"].as_str().unwrap().contains("image model"), "{v}");

    // A model missing a required part is refused.
    let (s, v) = h
        .post(
            "/image-models",
            json!({"name": "Bad", "recipe": "z-image-turbo", "files": {"diffusion_model": "diffusion_models/z_image_turbo-Q4_K.gguf"}}),
        )
        .await;
    assert_eq!(s, 400, "{v}");
    let (s, zm) = h
        .post(
            "/image-models",
            json!({"name": "Z-Image Turbo", "recipe": "z-image-turbo", "files": z["files"]}),
        )
        .await;
    assert_eq!(s, 201, "{zm}");
    let (s, km) = h
        .post(
            "/image-models",
            json!({"name": "Kontext", "recipe": "flux1-kontext", "files": {
                "diffusion_model": "diffusion_models/flux1-kontext-dev-Q4_0.gguf",
                "clip_l": "text_encoders/clip_l.safetensors",
                "t5xxl": "text_encoders/t5xxl_fp16.safetensors",
                "vae": "vae/ae.safetensors"}}),
        )
        .await;
    assert_eq!(s, 201, "{km}");

    // The project's style is added to every prompt.
    let (s, _) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/image-projects/{pid}"),
            Some(json!({"style": "flat vector logo", "negative": "text"})),
        )
        .await;
    assert_eq!(s, 200);

    let (s, t1) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "a fox mascot", "model_id": zm["id"], "width": 768, "height": 512, "steps": 4, "count": 2}),
        )
        .await;
    assert_eq!(s, 202, "{t1}");
    // One picture at a time per chat.
    let (s, _) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "again"}),
        )
        .await;
    assert_eq!(s, 409);
    let t1 = h.wait_turn(t1["id"].as_str().unwrap()).await;
    assert_eq!(t1["status"], "done", "{t1}");
    assert_eq!(t1["full_prompt"], "a fox mascot, flat vector logo");
    let pics = t1["images"].as_array().unwrap();
    assert_eq!(pics.len(), 2);
    let first = pics[0].as_str().unwrap().to_string();
    {
        let body = &h.seen.0.lock().unwrap()[0];
        assert_eq!(body["model"]["recipe"], "z-image-turbo");
        assert_eq!(body["negative_prompt"], "text");
        assert_eq!(body["batch_count"], 2);
        assert_eq!(body["width"], 768);
        assert_eq!(body["sample_params"]["sample_steps"], 4);
    }
    let resp = h
        .http
        .get(format!("{}/attachments/{first}", h.base))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.bytes().await.unwrap().starts_with(b"\x89PNG"));

    // The chat got a title from its first prompt and remembers the model.
    let chat = h.get(&format!("/image-chats/{cid}")).await;
    assert_eq!(chat["title"], "A fox mascot");
    assert_eq!(chat["model_id"], zm["id"]);
    assert_eq!(chat["cover"], first.as_str());

    // Refining with a model that can't follow edits starts from the picture
    // and keeps the earlier description.
    let (s, t2) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "wearing a scarf", "base_image": first}),
        )
        .await;
    assert_eq!(s, 202, "{t2}");
    let t2 = h.wait_turn(t2["id"].as_str().unwrap()).await;
    assert_eq!(t2["status"], "done", "{t2}");
    assert_eq!(
        t2["full_prompt"],
        "a fox mascot, wearing a scarf, flat vector logo"
    );
    assert_eq!(t2["params"]["edit"], false);
    {
        let body = &h.seen.0.lock().unwrap()[1];
        assert!(
            body["init_image"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(body["strength"].as_f64().unwrap() as f32, 0.6);
        // The base picture's size is used.
        assert_eq!(body["width"], 768);
    }

    // An editing model gets the picture as a reference and just the instruction.
    let (s, t3) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "make it night", "base_image": first, "model_id": km["id"]}),
        )
        .await;
    assert_eq!(s, 202, "{t3}");
    let t3 = h.wait_turn(t3["id"].as_str().unwrap()).await;
    assert_eq!(t3["full_prompt"], "make it night, flat vector logo");
    assert_eq!(t3["params"]["edit"], true);
    {
        let body = &h.seen.0.lock().unwrap()[2];
        assert_eq!(body["ref_images"].as_array().unwrap().len(), 1);
        assert!(body.get("init_image").is_none());
    }

    // Engine errors are explained.
    let (_, t4) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "fail please"}),
        )
        .await;
    let t4 = h.wait_turn(t4["id"].as_str().unwrap()).await;
    assert_eq!(t4["status"], "error");
    assert!(
        t4["error"].as_str().unwrap().contains("ran out of memory"),
        "{t4}"
    );

    // Stopping a picture being made.
    let (_, t5) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "slow sunset"}),
        )
        .await;
    let id5 = t5["id"].as_str().unwrap().to_string();
    for _ in 0..50 {
        let v = h.get(&format!("/image-chats/{cid}/turns")).await;
        if v["live"][&id5]["stage"] == "sampling" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let (s, v) = h.post(&format!("/image-turns/{id5}/stop"), json!({})).await;
    assert_eq!((s, v["stopping"].clone()), (200, json!(true)));
    assert_eq!(h.wait_turn(&id5).await["status"], "stopped");

    let turns = h.get(&format!("/image-chats/{cid}/turns")).await;
    assert_eq!(turns["turns"].as_array().unwrap().len(), 5);
    let projects = h.get("/image-projects").await;
    assert_eq!(projects["projects"][0]["chat_count"], 1);

    // Deleting the project removes its pictures.
    let (s, _) = h
        .req(
            reqwest::Method::DELETE,
            &format!("/image-projects/{pid}"),
            None,
        )
        .await;
    assert_eq!(s, 204);
    let resp = h
        .http
        .get(format!("{}/attachments/{first}", h.base))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    assert_eq!(h.get("/images/status").await["engine"]["external"], true);
}

/// A tiny Hugging Face: repository trees and file downloads.
async fn mock_hf(files: &'static [(&'static str, &'static str, &'static [u8])]) -> String {
    use axum::extract::Path;
    use axum::routing::get;
    let app = axum::Router::new()
        .route(
            "/api/models/{owner}/{name}/tree/main",
            get(move |Path((o, n)): Path<(String, String)>| async move {
                let repo = format!("{o}/{n}");
                let entries: Vec<Value> = files
                    .iter()
                    .filter(|(r, _, _)| *r == repo)
                    .map(|(_, p, b)| json!({"type": "file", "path": p, "size": b.len()}))
                    .collect();
                axum::Json(Value::Array(entries))
            }),
        )
        .route(
            "/{owner}/{name}/resolve/main/{*path}",
            get(move |Path((o, n, p)): Path<(String, String, String)>| async move {
                let repo = format!("{o}/{n}");
                files
                    .iter()
                    .find(|(r, f, _)| *r == repo && *f == p)
                    .map(|(_, _, b)| b.to_vec())
                    .ok_or(axum::http::StatusCode::NOT_FOUND)
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(axum::serve(l, app).into_future());
    url
}

#[tokio::test]
async fn downloads_a_starter_model_and_sets_it_up() {
    let hf = mock_hf(&[
        ("leejet/Z-Image-Turbo-GGUF", "z_image_turbo-Q4_K.gguf", b"diffusion"),
        ("leejet/Z-Image-Turbo-GGUF", "z_image_turbo-Q8_0.gguf", b"bigger diffusion"),
        ("unsloth/Qwen3-4B-Instruct-2507-GGUF", "Qwen3-4B-Instruct-2507-Q4_K_M.gguf", b"encoder"),
        ("Comfy-Org/z_image_turbo", "split_files/vae/ae.safetensors", b"vae"),
        ("someone/sdxl-thing", "sd_xl_base_1.0.safetensors", b"checkpoint"),
        ("someone/sdxl-thing", "text_encoder/model-00001-of-00002.safetensors", b"shard"),
    ])
    .await;
    // SAFETY: only this test talks to Hugging Face.
    unsafe { std::env::set_var("RELAY_HF_URL", &hf) };
    let h = H::start().await;
    let own = h.dir.join("data/image-models");

    let v = h.get("/images/downloads").await;
    let z = v["starters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "z-image-turbo")
        .unwrap()
        .clone();
    assert_eq!(z["set_up"], false, "{z}");
    assert!(z["missing_bytes"].as_u64().unwrap() > 1_000_000_000, "{z}");

    let (s, v) = h.post("/images/downloads/z-image-turbo", json!({"big": false})).await;
    assert_eq!(s, 202, "{v}");
    assert_eq!(v["downloads"].as_array().unwrap().len(), 3, "{v}");

    // The files land in their folders and the model is set up on its own.
    let mut models = Value::Null;
    for _ in 0..200 {
        models = h.get("/image-models").await;
        if !models["models"].as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        std::fs::read(own.join("diffusion_models/z_image_turbo-Q4_K.gguf")).unwrap(),
        b"diffusion"
    );
    assert_eq!(std::fs::read(own.join("vae/ae.safetensors")).unwrap(), b"vae");
    let m = &models["models"][0];
    assert_eq!(m["name"], "Z-Image Turbo", "{models}");
    assert_eq!(m["recipe"], "z-image-turbo");
    assert_eq!(m["files"]["diffusion_model"], "diffusion_models/z_image_turbo-Q4_K.gguf");
    assert_eq!(m["files"]["llm"], "text_encoders/Qwen3-4B-Instruct-2507-Q4_K_M.gguf");
    assert_eq!(m["files"]["vae"], "vae/ae.safetensors");

    let v = h.get("/images/downloads").await;
    let z = &v["starters"][0];
    assert_eq!(z["id"], "z-image-turbo");
    assert_eq!(z["set_up"], true, "{z}");
    assert_eq!(z["missing_bytes"], 0, "{z}");

    // Any file of any repository, into the folder it's for.
    let v = h.get("/images/hub/files?repo=someone/sdxl-thing").await;
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{v}");
    assert_eq!(files[0]["category"], "checkpoints");
    let (s, v) = h
        .post(
            "/images/hub/download",
            json!({"repo": "someone/sdxl-thing", "file": "sd_xl_base_1.0.safetensors", "category": "checkpoints"}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let path = own.join("checkpoints/sd_xl_base_1.0.safetensors");
    for _ in 0..100 {
        if path.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"checkpoint");
    let (s, _) = h
        .post(
            "/images/hub/download",
            json!({"repo": "someone/sdxl-thing", "file": "x.safetensors", "category": "nowhere"}),
        )
        .await;
    assert_eq!(s, 400);
}
