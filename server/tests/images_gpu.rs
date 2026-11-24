//! The chat model and the image model take turns on the GPU, with real
//! processes: the mock_llama and mock_sd example binaries stand in for
//! llama-server and sd-server.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use relay::config::LoadedConfig;

/// Builds an example binary and returns its path (as tests/supervisor.rs does).
fn example(name: &str) -> PathBuf {
    let out = std::process::Command::new(env!("CARGO"))
        .args(["build", "--example", name, "--message-format=json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stderr(std::process::Stdio::inherit())
        .output()
        .expect("cargo build");
    assert!(out.status.success(), "building {name} failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["target"]["name"] == name)
        .find_map(|v| v["executable"].as_str().map(PathBuf::from))
        .expect("example path")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct H {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl H {
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
    async fn get(&self, p: &str) -> Value {
        self.req(reqwest::Method::GET, p, None).await.1
    }
    async fn post(&self, p: &str, b: Value) -> (u16, Value) {
        self.req(reqwest::Method::POST, p, Some(b)).await
    }
    async fn until(&self, what: &str, f: impl Fn(&Value, &Value) -> bool) {
        for _ in 0..300 {
            let (m, i) = (self.get("/model").await, self.get("/images/status").await);
            if f(&m, &i) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!(
            "timed out waiting for {what}: model {} images {}",
            self.get("/model").await,
            self.get("/images/status").await
        );
    }
}

#[tokio::test]
async fn chat_model_and_image_model_take_turns() {
    let llama = example("mock_llama");
    let sd = example("mock_sd");
    let dir = std::env::temp_dir().join(format!("relay-gpu-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let model = dir.join("model.gguf");
    std::fs::write(&model, b"GGUF").unwrap();
    let slash = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
    let cfg_path = dir.join("data/config.toml");
    std::fs::write(
        &cfg_path,
        format!(
            "bind = \"127.0.0.1:0\"\n[llama]\nllama_server_path = \"{}\"\nmodel_path = \"{}\"\nport = {}\nauto_setup = false\n\
             [images]\nsd_server_path = \"{}\"\nport = {}\n[push]\nenabled = false\n",
            slash(&llama),
            slash(&model),
            free_port(),
            slash(&sd),
            free_port()
        ),
    )
    .unwrap();
    let cfg = LoadedConfig::load(&cfg_path).unwrap();
    let token = cfg.config.token.clone();
    let (app, state) = relay::build_app(cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(axum::serve(listener, app).into_future());
    let h = H {
        base: format!("http://127.0.0.1:{port}"),
        token,
        http: reqwest::Client::new(),
    };

    // The chat model loads first.
    h.post("/model/load", json!({})).await;
    h.until("the chat model", |m, _| {
        m["state"] == "ready" && m["external"] == false
    })
    .await;

    // Making a picture unloads it and starts sd-server.
    let cat = h.get("/images/catalog").await;
    let z = cat["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["recipe"] == "z-image-turbo")
        .unwrap()
        .clone();
    let (_, m) = h
        .post(
            "/image-models",
            json!({"name": "Z", "recipe": "z-image-turbo", "files": z["files"]}),
        )
        .await;
    let (_, p) = h
        .post("/image-projects", json!({"name": "P", "model_id": m["id"]}))
        .await;
    let (_, c) = h
        .post(
            &format!("/image-projects/{}/chats", p["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
    let cid = c["id"].as_str().unwrap();
    let (s, t) = h
        .post(
            &format!("/image-chats/{cid}/generate"),
            json!({"prompt": "a lighthouse", "steps": 2}),
        )
        .await;
    assert_eq!(s, 202, "{t}");
    h.until("the picture", |m, i| {
        m["state"] == "idle" && i["engine"]["gpu"] == "images" && i["engine"]["jobs"] == 0
    })
    .await;
    let turn = h
        .get(&format!("/image-turns/{}", t["id"].as_str().unwrap()))
        .await;
    assert_eq!(turn["turn"]["status"], "done", "{turn}");
    assert_eq!(h.get("/images/status").await["engine"]["state"], "ready");

    // A chat needing its model stops the image model and loads the chat model.
    h.post("/model/load", json!({})).await;
    h.until("the chat model again", |m, i| {
        m["state"] == "ready" && i["engine"]["state"] == "idle" && i["engine"]["gpu"] == "chat"
    })
    .await;

    state.model.shutdown().await;
    state.images.engine.stop().await;
}
