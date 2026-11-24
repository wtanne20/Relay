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
