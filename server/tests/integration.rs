//! Phases 4-5 end to end: REST + WebSocket + agent loop + tools + stop,
//! against the scripted mock llama-server.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

use relay::config::LoadedConfig;

struct Harness {
    base: String,
    ws_base: String,
    token: String,
    http: reqwest::Client,
    dir: PathBuf,
}

impl Harness {
    async fn start() -> Self {
        let mock = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let mock_port = mock.local_addr().expect("addr").port();
        tokio::spawn(common::mock_llama::serve(mock, Default::default()));

        let dir = std::env::temp_dir().join(format!("relay-it-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data/skills/greeting")).expect("mkdir");
        std::fs::write(
            dir.join("data/skills/greeting/SKILL.md"),
            "---\nname: greeting\ndescription: How to greet people\n---\nSay hi.\n",
        )
        .expect("skill");
        let cfg_path = dir.join("data/config.toml");
        std::fs::write(
            &cfg_path,
            format!(
                "bind = \"127.0.0.1:0\"\n[llama]\nport = {mock_port}\n[push]\nenabled = false\n"
            ),
        )
        .expect("config");
        let cfg = LoadedConfig::load(&cfg_path).expect("load config");
        assert!(cfg.token_generated);
        let token = cfg.config.token.clone();

        let (app, _state) = relay::build_app(cfg).await.expect("build app");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind relay");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(axum::serve(listener, app).into_future());

        let h = Harness {
            base: format!("http://127.0.0.1:{port}"),
            ws_base: format!("ws://127.0.0.1:{port}"),
            token,
            http: reqwest::Client::new(),
            dir,
        };
        // Wait for the model manager to detect the mock.
        for _ in 0..100 {
            let m = h.get("/model").await;
            if m["state"] == "ready" {
                return h;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("model never became ready");
    }

    async fn req(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut r = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.expect("request");
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> Value {
        let (s, v) = self.req(reqwest::Method::GET, path, None).await;
        assert_eq!(s, 200, "GET {path}: {v}");
        v
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.req(reqwest::Method::POST, path, Some(body)).await
    }

    async fn ws(&self, session_id: &str) -> Ws {
        let mut req = format!("{}/sessions/{session_id}/ws", self.ws_base)
            .into_client_request()
            .expect("ws request");
        req.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.token).parse().expect("header"),
        );
        let (stream, _) = tokio_tungstenite::connect_async(req)
            .await
            .expect("ws connect");
        Ws { stream }
    }
}

struct Ws {
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl Ws {
    async fn next(&mut self) -> Value {
        use futures_core::Stream;
        use std::pin::Pin;
        loop {
            let msg = tokio::time::timeout(
                Duration::from_secs(20),
                std::future::poll_fn(|cx| Pin::new(&mut self.stream).poll_next(cx)),
            )
            .await
            .expect("timed out waiting for a websocket event")
            .expect("websocket closed")
            .expect("websocket error");
            if let tungstenite::Message::Text(t) = msg {
                return serde_json::from_str(t.as_str()).expect("event json");
            }
        }
    }

    /// Collects events until a status event with one of `statuses`.
    async fn until_status(&mut self, statuses: &[&str]) -> Vec<Value> {
        let mut events = Vec::new();
        loop {
            let ev = self.next().await;
            let done = ev["type"] == "status" && statuses.iter().any(|s| ev["status"] == *s);
            events.push(ev);
            if done {
                return events;
            }
        }
    }

    async fn until_type(&mut self, ty: &str) -> Value {
        loop {
            let ev = self.next().await;
            if ev["type"] == ty {
                return ev;
            }
        }
    }
}

fn types(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|e| e["type"].as_str().unwrap_or("").to_string())
        .collect()
}

#[tokio::test]
async fn end_to_end() {
    let h = Harness::start().await;

    // --- auth ---
    let r = h
        .http
        .get(format!("{}/info", h.base))
        .send()
        .await
        .expect("req");
    assert_eq!(r.status(), 401);
    let r = h
        .http
        .get(format!("{}/info", h.base))
        .bearer_auth("wrong")
        .send()
        .await
        .expect("req");
    assert_eq!(r.status(), 401);
    let info = h.get("/info").await;
    assert_eq!(info["name"], "relay");
    assert_eq!(info["model"]["state"], "ready");

    // --- projects ---
    let proj_dir = h.dir.join("myproj");
    std::fs::create_dir_all(&proj_dir).expect("mkdir");
    std::fs::write(proj_dir.join("AGENTS.md"), "Always use tabs.").expect("agents");
    let (s, v) = h.post("/projects", json!({"path": "relative/path"})).await;
    assert_eq!(s, 400, "{v}");
    let (s, v) = h
        .post(
            "/projects",
            json!({"name": "My Project", "path": proj_dir.to_string_lossy()}),
        )
        .await;
    assert_eq!(s, 201, "{v}");
    let project_id = v["project"]["id"].as_str().expect("id").to_string();
    let (s, _) = h
        .post("/projects", json!({"path": proj_dir.to_string_lossy()}))
        .await;
    assert_eq!(s, 409, "duplicate folder");
    assert_eq!(
        h.get("/projects").await["projects"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );

    // --- sessions ---
    let (s, v) = h
        .post(&format!("/projects/{project_id}/sessions"), json!({}))
        .await;
    assert_eq!(s, 201, "{v}");
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    assert_eq!(v["session"]["status"], "idle");
    let (s, v) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/sessions/{sid}"),
            Some(json!({"title": "Hello file"})),
        )
        .await;
    assert_eq!(
        (s, v["session"]["title"].as_str()),
        (200, Some("Hello file"))
    );

    // --- prompt -> tool call -> file -> final answer, streamed ---
    let mut ws = h.ws(&sid).await;
    let hello = ws.next().await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["status"], "idle");

    let (s, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "create hello.txt containing hi"}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let events = ws.until_status(&["done", "error", "stopped"]).await;
    let t = types(&events);
    assert_eq!(
        events.last().map(|e| e["status"].clone()),
        Some(json!("done")),
        "{events:#?}"
    );
    for needed in [
        "status",
        "message",
        "reasoning_delta",
        "text_delta",
        "tool_call",
        "tool_result",
        "final",
    ] {
        assert!(t.iter().any(|x| x == needed), "missing {needed} in {t:?}");
    }
    let call = events
        .iter()
        .find(|e| e["type"] == "tool_call")
        .expect("tool_call");
    assert_eq!(call["name"], "write_file");
    assert_eq!(
        serde_json::from_str::<Value>(call["arguments"].as_str().unwrap_or("")).ok(),
        Some(json!({"path": "hello.txt", "content": "hi"}))
    );
    let result = events
        .iter()
        .find(|e| e["type"] == "tool_result")
        .expect("tool_result");
    assert_eq!(result["is_error"], false, "{result}");
    assert_eq!(
        std::fs::read_to_string(proj_dir.join("hello.txt")).expect("hello.txt created"),
        "hi"
    );

    let msgs = h.get(&format!("/sessions/{sid}/messages")).await;
    let roles: Vec<&str> = msgs["messages"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|m| m["role"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(roles, ["user", "assistant", "tool", "assistant"]);
    assert_eq!(msgs["messages"][1]["tool_calls"][0]["name"], "write_file");
    assert_eq!(msgs["has_more"], false);
    let page = h.get(&format!("/sessions/{sid}/messages?limit=2")).await;
    assert_eq!(page["has_more"], true);
    let first_id = page["messages"][0]["id"].as_i64().expect("id");
    let older = h
        .get(&format!(
            "/sessions/{sid}/messages?before={first_id}&limit=10"
        ))
        .await;
    assert_eq!(older["messages"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        h.get(&format!("/sessions/{sid}")).await["session"]["status"],
        "done"
    );

    // --- stop while a command runs: the command is killed ---
    let sleep = if cfg!(windows) {
        "run: Start-Sleep -Seconds 60"
    } else {
        "run: sleep 60"
    };
    let (s, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": sleep}),
        )
        .await;
    assert_eq!(s, 202);
    let call = ws.until_type("tool_call").await;
    assert_eq!(call["name"], "run_command");
    let (s, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "again"}),
        )
        .await;
    assert_eq!(s, 409, "busy: {v}");
    let started = std::time::Instant::now();
    let (s, v) = h.post(&format!("/sessions/{sid}/stop"), json!({})).await;
    assert_eq!((s, v["stopping"].as_bool()), (200, Some(true)));
    let events = ws.until_status(&["stopped", "done", "error"]).await;
    assert_eq!(
        events.last().map(|e| e["status"].clone()),
        Some(json!("stopped")),
        "{events:#?}"
    );
    assert!(started.elapsed() < Duration::from_secs(15));
    let tool = events
        .iter()
        .find(|e| e["type"] == "tool_result")
        .expect("tool_result");
    assert!(
        tool["content"].as_str().unwrap_or("").contains("cancelled"),
        "{tool}"
    );

    // --- stop while streaming text: partial answer is kept ---
    let (s, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "be slow"}),
        )
        .await;
    assert_eq!(s, 202);
    ws.until_type("text_delta").await;
    h.post(&format!("/sessions/{sid}/stop"), json!({})).await;
    let events = ws.until_status(&["stopped", "done", "error"]).await;
    assert_eq!(
        events.last().map(|e| e["status"].clone()),
        Some(json!("stopped"))
    );
    let all = h.get(&format!("/sessions/{sid}/messages?limit=500")).await;
    let last = all["messages"]
        .as_array()
        .and_then(|a| a.last())
        .cloned()
        .unwrap_or_default();
    assert_eq!(last["role"], "assistant");
    assert!(last["content"].as_str().unwrap_or("").starts_with("word"));

    // --- reconnect: hello reports status; history continues to work ---
    drop(ws);
    let mut ws = h.ws(&sid).await;
    let hello = ws.next().await;
    assert_eq!(hello["status"], "stopped");
    assert_eq!(hello["last_message_id"], last["id"]);

    // A follow-up after stops still produces a valid request (no dangling tool calls).
    h.post(
        &format!("/sessions/{sid}/prompt"),
        json!({"content": "what now?"}),
    )
    .await;
    let events = ws.until_status(&["done", "error"]).await;
    assert_eq!(
        events.last().map(|e| e["status"].clone()),
        Some(json!("done")),
        "{events:#?}"
    );

    // --- push token validation ---
    let (s, _) = h.post("/push-tokens", json!({"token": "nope"})).await;
    assert_eq!(s, 400);
    let (s, _) = h
        .post("/push-tokens", json!({"token": "ExponentPushToken[abc]"}))
        .await;
    assert_eq!(s, 201);
    let (s, v) = h
        .req(
            reqwest::Method::DELETE,
            "/push-tokens",
            Some(json!({"token": "ExponentPushToken[abc]"})),
        )
        .await;
    assert_eq!((s, v["removed"].as_bool()), (200, Some(true)));

    // --- deleting removes from Relay only; folder stays ---
    let (s, _) = h
        .req(reqwest::Method::DELETE, &format!("/sessions/{sid}"), None)
        .await;
    assert_eq!(s, 200);
    let (s, _) = h
        .req(
            reqwest::Method::DELETE,
            &format!("/projects/{project_id}"),
            None,
        )
        .await;
    assert_eq!(s, 200);
    assert!(proj_dir.join("hello.txt").exists());
    let (s, _) = h
        .req(
            reqwest::Method::GET,
            &format!("/projects/{project_id}"),
            None,
        )
        .await;
    assert_eq!(s, 404);

    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn system_prompt_includes_agents_md_and_skills() {
    let dir = std::env::temp_dir().join(format!("relay-sp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("proj")).expect("mkdir");
    std::fs::create_dir_all(dir.join("skills/pdf")).expect("mkdir");
    std::fs::write(dir.join("proj/AGENTS.md"), "Run tests with cargo test.").expect("write");
    std::fs::write(
        dir.join("skills/pdf/SKILL.md"),
        "---\nname: pdf\ndescription: Read PDFs\n---\n",
    )
    .expect("write");
    std::fs::write(dir.join("system.md"), "You are a test agent.").expect("write");
    let project = relay::db::Project {
        id: "p".into(),
        name: "proj".into(),
        path: dir.join("proj").to_string_lossy().into_owned(),
        created_at: String::new(),
    };
    let skills = relay::skills::discover(&[dir.join("skills")]);
    let s = relay::agent::build_system_prompt(
        &dir.join("system.md"),
        &project,
        &dir.join("proj"),
        &skills,
    )
    .await;
    assert!(s.starts_with("You are a test agent."));
    assert!(s.contains("Run tests with cargo test."));
    assert!(s.contains("- pdf: Read PDFs (path: "));
    let _ = std::fs::remove_dir_all(dir);
}
