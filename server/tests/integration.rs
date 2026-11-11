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
        Self::start_with(Default::default(), "").await
    }

    async fn start_with(mock_opts: common::mock_llama::MockOptions, llama_extra: &str) -> Self {
        let h = Self::launch(mock_opts, llama_extra).await;
        // The model starts unloaded; load it and wait until it's ready.
        h.post("/model/load", json!({})).await;
        h.wait_model("ready").await;
        h
    }

    async fn wait_model(&self, state: &str) {
        for _ in 0..100 {
            if self.get("/model").await["state"] == state {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("model never became {state}");
    }

    async fn launch(mock_opts: common::mock_llama::MockOptions, llama_extra: &str) -> Self {
        let mock = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let mock_port = mock.local_addr().expect("addr").port();
        tokio::spawn(common::mock_llama::serve(mock, mock_opts));

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
                "bind = \"127.0.0.1:0\"\n[llama]\nport = {mock_port}\n{llama_extra}\n[push]\nenabled = false\n"
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

        Harness {
            base: format!("http://127.0.0.1:{port}"),
            ws_base: format!("ws://127.0.0.1:{port}"),
            token,
            http: reqwest::Client::new(),
            dir,
        }
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
    // The arguments also arrive piece by piece while they're written.
    let streamed: String = events
        .iter()
        .filter(|e| e["type"] == "tool_call_delta" && e["name"] == "write_file")
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert_eq!(streamed, call["arguments"].as_str().unwrap_or(""));
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
        instructions: "Prefer small commits.".into(),
        model: None,
        archived: false,
        device_id: None,
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
    assert!(s.contains("Prefer small commits."));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn voice_prompts_ask_for_spoken_replies() {
    let h = Harness::start().await;
    let (_, v) = h.post("/projects/chats/sessions", json!({})).await;
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    let mut ws = h.ws(&sid).await;

    let mut reply = async |body: Value| {
        let (st, v) = h.post(&format!("/sessions/{sid}/prompt"), body).await;
        assert_eq!(st, 202, "{v}");
        let events = ws.until_status(&["done", "error"]).await;
        events
            .iter()
            .rev()
            .find(|e| e["type"] == "message" && e["message"]["role"] == "assistant")
            .map(|e| e["message"]["content"].as_str().unwrap_or("").to_string())
            .expect("a reply")
    };
    assert_eq!(
        reply(json!({"content": "which reply style?"})).await,
        "Style: written"
    );
    assert_eq!(
        reply(json!({"content": "which reply style?", "voice": true})).await,
        "Style: spoken"
    );
    // It applies to that prompt only.
    assert_eq!(
        reply(json!({"content": "which reply style?"})).await,
        "Style: written"
    );
}

#[tokio::test]
async fn settings_skills_and_approvals() {
    let h = Harness::start().await;
    let patch = |body: Value| h.req(reqwest::Method::PATCH, "/settings", Some(body));

    // --- settings ---
    let s = h.get("/settings").await;
    assert_eq!(s["settings"]["agent"]["max_steps"], 50);
    assert_eq!(s["settings"]["sampling"]["temperature"], 0.6);
    assert_eq!(s["tools"].as_array().map(Vec::len), Some(7));
    assert!(
        s["settings"].get("token").is_none(),
        "token must not be exposed"
    );

    let (st, v) =
        patch(json!({"sampling": {"temperature": 0.2}, "agent": {"max_steps": 12}})).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["settings"]["sampling"]["temperature"], 0.2);
    assert_eq!(v["model_restarting"], false);
    let saved = std::fs::read_to_string(h.dir.join("data/config.toml")).expect("config");
    assert!(saved.contains("temperature = 0.2"), "{saved}");
    assert!(saved.contains("max_steps = 12"));
    assert_eq!(h.get("/info").await["max_steps"], 12);

    for bad in [
        json!({"sampling": {"temperature": 9}}),
        json!({"token": "x"}),
        json!({"bind": "0.0.0.0:1"}),
        json!({"agent": {"bogus": 1}}),
        json!({"agent": {"max_steps": null}}),
        json!({"llama": {"extra_args": ["--host", "0.0.0.0"]}}),
        json!({"llama": {"model_path": "/definitely/not/here.gguf"}}),
        json!({"agent": {"approval": "sometimes"}}),
    ] {
        let (st, v) = patch(bad.clone()).await;
        assert_eq!(st, 400, "{bad} -> {v}");
    }
    assert_eq!(
        h.get("/settings").await["settings"]["sampling"]["temperature"],
        0.2
    );

    // Changing a llama-server setting restarts the model.
    let (st, v) = patch(json!({"llama": {"context_size": 32768}})).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["model_restarting"], true);
    for _ in 0..100 {
        if h.get("/model").await["state"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // --- system prompt ---
    let p = h.get("/system-prompt").await;
    assert!(
        p["content"]
            .as_str()
            .unwrap_or("")
            .contains("You are Relay")
    );
    let (st, _) = h
        .req(
            reqwest::Method::PUT,
            "/system-prompt",
            Some(json!({"content": "Be terse."})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(h.get("/system-prompt").await["content"], "Be terse.");
    let (_, v) = h
        .req(
            reqwest::Method::PUT,
            "/system-prompt",
            Some(json!({"content": "  "})),
        )
        .await;
    assert!(
        v["content"]
            .as_str()
            .unwrap_or("")
            .contains("You are Relay"),
        "reset to default"
    );

    // --- skills ---
    let sk = h.get("/skills").await;
    assert_eq!(sk["skills"][0]["name"], "greeting");
    assert_eq!(sk["skills"][0]["enabled"], true);
    let (st, _) = patch(json!({"disabled_skills": ["greeting"]})).await;
    assert_eq!(st, 200);
    assert_eq!(h.get("/skills").await["skills"][0]["enabled"], false);

    // --- models ---
    std::fs::write(h.dir.join("data/models/Tiny-Coder-Q4.gguf"), "GGUF").expect("model");
    let m = h.get("/models").await;
    assert!(m["models"].is_array());
    assert!(m["dirs"].as_array().is_some_and(|d| !d.is_empty()));
    let tiny = m["models"]
        .as_array()
        .and_then(|a| a.iter().position(|x| x["name"] == "Tiny-Coder-Q4.gguf"))
        .expect("tiny model listed");
    let current = "Tiny-Coder-Q4.gguf".to_string();
    assert_eq!(m["models"][tiny]["custom_tags"], false);
    assert_eq!(m["models"][tiny]["tags"], json!(["code"]));
    let put = |body: Value| h.req(reqwest::Method::PUT, "/models/tags", Some(body));
    let (st, v) = put(json!({"name": current, "tags": [" code ", "UI", "code", ""]})).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["tags"], json!(["code", "UI"]));
    let m = h.get("/models").await;
    assert_eq!(m["models"][tiny]["tags"], json!(["code", "UI"]));
    assert_eq!(m["models"][tiny]["custom_tags"], true);
    let (st, _) = put(json!({"name": "../x", "tags": []})).await;
    assert_eq!(st, 400);
    let (st, _) = put(json!({"name": current, "tags": null})).await;
    assert_eq!(st, 200);
    assert_eq!(h.get("/models").await["models"][tiny]["custom_tags"], false);

    // --- project instructions ---
    let proj_dir = h.dir.join("approvals");
    std::fs::create_dir_all(&proj_dir).expect("mkdir");
    let (_, v) = h
        .post("/projects", json!({"path": proj_dir.to_string_lossy()}))
        .await;
    let pid = v["project"]["id"].as_str().expect("id").to_string();
    assert_eq!(v["project"]["instructions"], "");
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/projects/{pid}"),
            Some(json!({"instructions": "Use 4 spaces."})),
        )
        .await;
    assert_eq!(
        (st, v["project"]["instructions"].as_str()),
        (200, Some("Use 4 spaces."))
    );

    // --- approvals ---
    let (st, _) = patch(json!({"agent": {"approval": "commands"}})).await;
    assert_eq!(st, 200);
    let (_, v) = h
        .post(&format!("/projects/{pid}/sessions"), json!({}))
        .await;
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    let mut ws = h.ws(&sid).await;
    assert_eq!(ws.next().await["type"], "hello");

    // Deny: the command never runs.
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "run: echo denied > marker.txt"}),
        )
        .await;
    assert_eq!(st, 202);
    let req = ws.until_type("approval_request").await;
    let call_id = req["approval"]["tool_call_id"]
        .as_str()
        .expect("id")
        .to_string();
    assert_eq!(req["approval"]["name"], "run_command");

    // A client connecting now sees the pending request in hello.
    let mut ws2 = h.ws(&sid).await;
    let hello = ws2.next().await;
    assert_eq!(hello["pending_approval"]["tool_call_id"], call_id.as_str());

    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/approval"),
            json!({"tool_call_id": "nope", "decision": "approve"}),
        )
        .await;
    assert_eq!(st, 409);
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/approval"),
            json!({"tool_call_id": call_id, "decision": "deny"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let resolved = ws.until_type("approval_resolved").await;
    assert_eq!(resolved["approved"], false);
    let result = ws.until_type("tool_result").await;
    assert_eq!(result["is_error"], true);
    assert!(result["content"].as_str().unwrap_or("").contains("denied"));
    let ev = ws.until_status(&["done", "error", "stopped"]).await;
    assert_eq!(ev.last().map(|e| e["status"].clone()), Some(json!("done")));
    assert!(!proj_dir.join("marker.txt").exists());

    // Approve: it runs.
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "run: echo approved"}),
        )
        .await;
    assert_eq!(st, 202);
    let req = ws.until_type("approval_request").await;
    let call_id = req["approval"]["tool_call_id"]
        .as_str()
        .expect("id")
        .to_string();
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/approval"),
            json!({"tool_call_id": call_id, "decision": "approve"}),
        )
        .await;
    assert_eq!(st, 200);
    let result = ws.until_type("tool_result").await;
    assert_eq!(result["is_error"], false, "{result}");
    assert!(
        result["content"]
            .as_str()
            .unwrap_or("")
            .contains("approved")
    );
    ws.until_status(&["done", "error", "stopped"]).await;

    // Nothing pending any more.
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/approval"),
            json!({"tool_call_id": call_id, "decision": "approve"}),
        )
        .await;
    assert_eq!(st, 409);

    // Always allow: saves the suggested rule, and the same command isn't
    // asked about again.
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "run: echo always"}),
        )
        .await;
    assert_eq!(st, 202);
    let req = ws.until_type("approval_request").await;
    assert_eq!(req["approval"]["suggested_rule"]["kind"], "command");
    assert_eq!(req["approval"]["suggested_rule"]["pattern"], "echo always");
    let call_id = req["approval"]["tool_call_id"]
        .as_str()
        .expect("id")
        .to_string();
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/approval"),
            json!({"tool_call_id": call_id, "decision": "always_allow"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["rule"]["pattern"], "echo always");
    let rule_id = v["rule"]["id"].as_i64().expect("rule id");
    ws.until_status(&["done", "error", "stopped"]).await;
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "run: echo always"}),
        )
        .await;
    assert_eq!(st, 202);
    let ev = ws.until_status(&["done", "error", "stopped"]).await;
    assert!(
        !ev.iter().any(|e| e["type"] == "approval_request"),
        "a rule-covered command was asked about"
    );
    let rules = h.get("/approval-rules").await;
    assert_eq!(rules["rules"][0]["uses"], 1, "{rules}");

    // The activity log has every call and how it was allowed.
    let log = h.get(&format!("/activity?session_id={sid}")).await;
    let decisions: Vec<&str> = log["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["decision"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(decisions, ["rule", "allowed", "allowed", "denied"], "{log}");
    assert_eq!(log["entries"][0]["detail"], "echo always");
    assert_eq!(log["entries"][0]["rule_id"], rule_id);
    let denied = h.get("/activity?filter=denied").await;
    assert_eq!(denied["entries"].as_array().map(Vec::len), Some(1));

    // Rules can be added by hand, checked, and removed.
    let (st, _) = h
        .post("/approval-rules", json!({"kind": "command", "pattern": "ls | sh"}))
        .await;
    assert_eq!(st, 400);
    let (st, v) = h
        .post("/approval-rules", json!({"kind": "web", "pattern": "https://www.Example.com/docs"}))
        .await;
    assert_eq!((st, v["pattern"].as_str()), (200, Some("example.com")));
    let (st, _) = h
        .req(reqwest::Method::DELETE, &format!("/approval-rules/{rule_id}"), None)
        .await;
    assert_eq!(st, 200);
    assert_eq!(h.get("/approval-rules").await["rules"].as_array().map(Vec::len), Some(1));

    // Stopping while waiting for approval ends the task.
    let (st, _) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "run: echo never"}),
        )
        .await;
    assert_eq!(st, 202);
    ws.until_type("approval_request").await;
    let (st, _) = h.post(&format!("/sessions/{sid}/stop"), json!({})).await;
    assert_eq!(st, 200);
    let ev = ws.until_status(&["done", "error", "stopped"]).await;
    assert_eq!(
        ev.last().map(|e| e["status"].clone()),
        Some(json!("stopped"))
    );
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn pairing_and_cors() {
    let h = Harness::start().await;

    // The pairing endpoint needs the token like everything else.
    let r = h
        .http
        .get(format!("{}/pairing", h.base))
        .send()
        .await
        .expect("pairing");
    assert_eq!(r.status(), 401);
    let p = h.get("/pairing").await;
    let link = p["link"].as_str().expect("link");
    assert!(link.starts_with("relay://pair?url="), "{link}");
    assert!(link.contains(&h.token));
    assert!(p["svg"].as_str().is_some_and(|s| s.contains("<svg")));

    let preflight = |origin: &'static str| {
        h.http
            .request(reqwest::Method::OPTIONS, format!("{}/info", h.base))
            .header("Origin", origin)
            .header("Access-Control-Request-Method", "GET")
            .header("Access-Control-Request-Headers", "authorization")
            .send()
    };
    // The desktop window and the local dev server may read responses...
    for origin in [
        "http://tauri.localhost",
        "tauri://localhost",
        "http://localhost:8081",
    ] {
        let r = preflight(origin).await.expect("preflight");
        assert_eq!(
            r.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some(origin),
            "{origin}"
        );
    }
    // ...other web pages may not.
    let r = preflight("https://evil.example").await.expect("preflight");
    assert!(r.headers().get("access-control-allow-origin").is_none());
    // CORS never replaces the token.
    let r = h
        .http
        .get(format!("{}/info", h.base))
        .header("Origin", "http://tauri.localhost")
        .send()
        .await
        .expect("info");
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn projects_folder_creates_named_folders() {
    let h = Harness::start().await;
    let patch = |body: Value| h.req(reqwest::Method::PATCH, "/settings", Some(body));

    // Without a projects folder, a name alone is not enough.
    let (st, v) = h.post("/projects", json!({"name": "Alpha"})).await;
    assert_eq!(st, 400, "{v}");

    let (st, _) = patch(json!({"projects_folder": "relative/dir"})).await;
    assert_eq!(st, 400);
    let root = h.dir.join("My Projects");
    let (st, v) = patch(json!({"projects_folder": root.to_string_lossy()})).await;
    assert_eq!(st, 200, "{v}");
    assert!(root.is_dir(), "projects folder is created");
    assert_eq!(
        h.get("/settings").await["settings"]["projects_folder"],
        json!(dunce::canonicalize(&root).expect("canon").to_string_lossy())
    );

    let (st, v) = h
        .post("/projects", json!({"name": "Mileage: tracker?"}))
        .await;
    assert_eq!(st, 201, "{v}");
    let made = root.join("Mileage- tracker-");
    assert!(made.is_dir(), "project folder created");
    assert_eq!(v["project"]["name"], "Mileage: tracker?");

    // An existing folder is reused, never replaced.
    std::fs::create_dir_all(root.join("Beta")).expect("mkdir");
    std::fs::write(root.join("Beta/keep.txt"), "x").expect("write");
    let (st, _) = h
        .post("/projects", json!({"name": "Beta", "path": ""}))
        .await;
    assert_eq!(st, 201);
    assert!(root.join("Beta/keep.txt").is_file());
    let (st, _) = h.post("/projects", json!({"name": "Beta"})).await;
    assert_eq!(st, 409, "same folder twice");
    let (st, _) = h.post("/projects", json!({"name": "CON"})).await;
    assert_eq!(st, 400, "reserved Windows name");
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn plain_chats_titles_and_models() {
    let h = Harness::start().await;

    // The plain chats space exists but is never listed as a project.
    let projects = h.get("/projects").await;
    assert!(
        projects["projects"]
            .as_array()
            .expect("list")
            .iter()
            .all(|p| p["id"] != "chats")
    );
    let (st, _) = h
        .req(reqwest::Method::DELETE, "/projects/chats", None)
        .await;
    assert_eq!(st, 400, "plain chats can't be removed");
    let (st, _) = h
        .req(
            reqwest::Method::PATCH,
            "/projects/chats",
            Some(json!({"name": "x"})),
        )
        .await;
    assert_eq!(st, 400);

    // A chat without a title is "New chat" until the first reply names it.
    let (st, v) = h.post("/projects/chats/sessions", json!({})).await;
    assert_eq!(st, 201, "{v}");
    assert_eq!(v["session"]["title"], "New chat");
    assert_eq!(v["session"]["auto_title"], true);
    let sid = v["session"]["id"].as_str().expect("id").to_string();

    let mut ws = h.ws(&sid).await;
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "What is a GPU?"}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
    let events = ws.until_status(&["done", "error"]).await;
    assert!(
        events.iter().any(|e| e["type"] == "final"),
        "plain chat answered without tools: {events:?}"
    );
    let title = ws.until_type("title").await;
    assert_eq!(title["title"], "Mock chat title");
    let s = h.get(&format!("/sessions/{sid}")).await;
    assert_eq!(s["session"]["title"], "Mock chat title");
    assert_eq!(s["session"]["auto_title"], false);

    // A title the user typed is kept.
    let (st, v) = h
        .post("/projects/chats/sessions", json!({"title": "Mine"}))
        .await;
    assert_eq!(st, 201);
    assert_eq!(v["session"]["auto_title"], false);

    // Archiving hides a chat from the lists until it is restored.
    let mine = v["session"]["id"].as_str().expect("id").to_string();
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/sessions/{mine}"),
            Some(json!({"archived": true})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["session"]["archived"], true);
    let listed = |v: &Value, key: &str, id: &str| {
        v[key]
            .as_array()
            .expect("list")
            .iter()
            .any(|x| x["id"] == id)
    };
    assert!(!listed(&h.get("/recent").await, "sessions", &mine));
    assert!(!listed(
        &h.get("/projects/chats/sessions").await,
        "sessions",
        &mine
    ));
    assert!(listed(&h.get("/archived").await, "sessions", &mine));
    let (st, _) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/sessions/{mine}"),
            Some(json!({"archived": false})),
        )
        .await;
    assert_eq!(st, 200);
    assert!(listed(&h.get("/recent").await, "sessions", &mine));
    assert!(!listed(&h.get("/archived").await, "sessions", &mine));
    let (st, _) = h
        .req(
            reqwest::Method::PATCH,
            "/projects/chats",
            Some(json!({"archived": true})),
        )
        .await;
    assert_eq!(st, 400, "the plain chats space can't be archived");

    // Recent lists chats from every project with the project's name.
    let recent = h.get("/recent?limit=5").await;
    let first = &recent["sessions"][0];
    assert_eq!(first["project_name"], "Chats");
    assert!(first["id"].is_string());

    // Models per chat and per project must be real .gguf files.
    let (st, _) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/sessions/{sid}"),
            Some(json!({"model": "C:/nope.gguf"})),
        )
        .await;
    assert_eq!(st, 400);
    std::fs::create_dir_all(h.dir.join("data/models")).expect("mkdir");
    let model = h.dir.join("data/models/Other-Q4.gguf");
    std::fs::write(&model, b"GGUF").expect("write");
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/sessions/{sid}"),
            Some(json!({"model": model.to_string_lossy()})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["session"]["model"], json!(model.to_string_lossy()));
    assert_eq!(v["session"]["title"], "Mock chat title", "title untouched");
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            &format!("/sessions/{sid}"),
            Some(json!({"model": ""})),
        )
        .await;
    assert_eq!(st, 200);
    assert!(v["session"]["model"].is_null());
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            "/projects/chats",
            Some(json!({"model": model.to_string_lossy()})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["project"]["model"], json!(model.to_string_lossy()));

    // Relay's models folder can live on another drive.
    let big = h.dir.join("big-disk/Relay models");
    let (st, _) = h
        .req(
            reqwest::Method::PATCH,
            "/settings",
            Some(json!({"models_folder": "relative"})),
        )
        .await;
    assert_eq!(st, 400, "must be absolute");
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            "/settings",
            Some(json!({"models_folder": big.to_string_lossy()})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert!(big.is_dir(), "created");
    let big = dunce::canonicalize(&big).expect("canon");

    // Copying a model from another app's folder lands in Relay's own folder
    // and leaves the original alone.
    std::fs::create_dir_all(h.dir.join("other-app/models")).expect("mkdir");
    let outside = h.dir.join("other-app/models/Outside-Q4.gguf");
    std::fs::write(&outside, b"GGUF-outside").expect("write");
    let (st, _) = h
        .post(
            "/models/import",
            json!({"path": h.dir.join("other-app").to_string_lossy()}),
        )
        .await;
    assert_eq!(st, 400, "folders are not model files");
    let (st, v) = h
        .post("/models/import", json!({"path": outside.to_string_lossy()}))
        .await;
    assert_eq!(st, 202, "{v}");
    let copy = big.join("imported/Outside-Q4.gguf");
    for _ in 0..100 {
        if copy.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(std::fs::read(&copy).expect("copied"), b"GGUF-outside");
    assert!(outside.is_file(), "the original stays where it was");
    let (st, _) = h
        .post("/models/import", json!({"path": copy.to_string_lossy()}))
        .await;
    assert_eq!(st, 400, "already in Relay's folder");

    // Mockups: empty sources and unknown pictures are refused.
    let (st, _) = h.post("/mockups", json!({"source": "  "})).await;
    assert_eq!(st, 400);
    let (st, _) = h
        .req(reqwest::Method::GET, "/mockups/..%2F..%2Fconfig.toml", None)
        .await;
    assert_eq!(st, 404);
    let (st, _) = h
        .req(
            reqwest::Method::GET,
            "/mockups/0123456789abcdef01234567.png",
            None,
        )
        .await;
    assert_eq!(st, 404);
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn model_libraries_add_move_and_remove() {
    let h = Harness::start().await;
    let builtin = h.dir.join("data/models");
    let model = builtin.join("pub/repo/tiny.gguf");
    std::fs::create_dir_all(model.parent().expect("parent")).expect("mkdir");
    std::fs::write(&model, vec![7u8; 4096]).expect("write model");

    let v = h.get("/libraries").await;
    let libs = v["libraries"].as_array().expect("list").clone();
    assert_eq!(libs.len(), 1, "{v}");
    assert_eq!(libs[0]["builtin"], true);
    assert_eq!(libs[0]["default"], true);
    assert_eq!(libs[0]["models"].as_array().expect("models").len(), 1);
    assert_eq!(libs[0]["size_bytes"], 4096);
    assert!(libs[0]["disk"]["total_bytes"].as_u64().expect("total") > 0);

    // Refused: relative paths, other apps' folders and nested libraries.
    let (st, _) = h.post("/libraries", json!({"path": "rel/dir"})).await;
    assert_eq!(st, 400);
    let lm = h.dir.join(".lmstudio/models");
    let (st, _) = h
        .post("/libraries", json!({"path": lm.to_string_lossy()}))
        .await;
    assert_eq!(st, 400);
    assert!(!lm.exists(), "another app's folder is never created");
    let nested = builtin.join("inner");
    let (st, _) = h
        .post("/libraries", json!({"path": nested.to_string_lossy()}))
        .await;
    assert_eq!(st, 400);

    let second = h.dir.join("Second Disk/Relay/models");
    let (st, v) = h
        .post(
            "/libraries",
            json!({"path": second.to_string_lossy(), "make_default": true}),
        )
        .await;
    assert_eq!(st, 201, "{v}");
    assert!(second.is_dir());
    let (st, _) = h
        .post("/libraries", json!({"path": second.to_string_lossy()}))
        .await;
    assert_eq!(st, 409, "same library twice");
    let second_s = dunce::canonicalize(&second)
        .expect("canon")
        .to_string_lossy()
        .into_owned();
    let libs = h.get("/libraries").await["libraries"].clone();
    let lib2 = libs
        .as_array()
        .expect("list")
        .iter()
        .find(|l| l["path"] == json!(second_s))
        .expect("second library listed")
        .clone();
    assert_eq!(lib2["default"], true);
    assert_eq!(lib2["builtin"], false);

    // Moving copies, then removes the original.
    let (st, v) = h
        .post(
            "/models/move",
            json!({"path": model.to_string_lossy(), "library": second_s}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
    let moved = second.join("pub/repo/tiny.gguf");
    for _ in 0..100 {
        if moved.is_file() && !model.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(moved.is_file(), "model copied");
    assert_eq!(std::fs::metadata(&moved).expect("meta").len(), 4096);
    assert!(!model.exists(), "original removed after the move");
    assert!(!builtin.join("pub").exists(), "empty folders tidied");
    let (st, _) = h
        .post(
            "/models/move",
            json!({"path": moved.to_string_lossy(), "library": second_s}),
        )
        .await;
    assert_eq!(st, 400, "already there");

    // Removing a library forgets it but keeps its files.
    let (st, _) = h
        .post(
            "/libraries/remove",
            json!({"path": builtin.to_string_lossy()}),
        )
        .await;
    assert_eq!(st, 400, "built-in can't be removed");
    let (st, v) = h.post("/libraries/remove", json!({"path": second_s})).await;
    assert_eq!(st, 200, "{v}");
    assert!(moved.is_file(), "files stay on disk");
    let libs = v["libraries"].as_array().expect("list").clone();
    assert_eq!(libs.len(), 1);
    assert_eq!(libs[0]["default"], true, "built-in is the default again");
    let _ = std::fs::remove_dir_all(&h.dir);
}

/// Sends `n` long prompts to a new plain chat; returns the session id and
/// whether every task finished cleanly.
async fn long_chat(h: &Harness, n: usize) -> (String, Vec<Value>) {
    let (st, v) = h
        .post("/projects/chats/sessions", json!({"title": "Long"}))
        .await;
    assert_eq!(st, 201, "{v}");
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    let mut ws = h.ws(&sid).await;
    let mut all = Vec::new();
    for i in 0..n {
        let text = format!("part {i}: {}", "lorem ipsum dolor sit amet ".repeat(120));
        let (st, v) = h
            .post(&format!("/sessions/{sid}/prompt"), json!({"content": text}))
            .await;
        assert_eq!(st, 202, "{v}");
        let events = ws.until_status(&["done", "error", "stopped"]).await;
        let last = events.last().cloned().unwrap_or(Value::Null);
        assert_eq!(last["status"], "done", "prompt {i}: {events:?}");
        all.extend(events);
    }
    (sid, all)
}

#[tokio::test]
async fn long_chats_are_summarized_before_they_overflow() {
    // Each prompt is about 1,100 tokens; the context fits about six.
    let h = Harness::start_with(Default::default(), "context_size = 8192").await;
    let (sid, events) = long_chat(&h, 14).await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "compacting" && e["active"] == true),
        "Relay said it was summarizing"
    );
    let msgs = h.get(&format!("/sessions/{sid}/messages?limit=200")).await;
    let list = msgs["messages"].as_array().expect("messages");
    let summaries: Vec<&Value> = list.iter().filter(|m| m["role"] == "summary").collect();
    assert!(!summaries.is_empty(), "a summary was stored");
    let s = summaries[summaries.len() - 1];
    assert!(s["content"].as_str().unwrap_or("").starts_with("## Goal"));
    assert!(s["covers_upto"].as_i64().is_some());
    // Nothing is deleted: every prompt and answer is still there.
    assert_eq!(list.iter().filter(|m| m["role"] == "user").count(), 14);
    assert_eq!(list.iter().filter(|m| m["role"] == "assistant").count(), 14);
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn an_overflowing_prompt_is_summarized_and_retried() {
    // Relay thinks the context is huge, but the model refuses past 6,000.
    let opts = common::mock_llama::MockOptions {
        ctx_tokens: 6000,
        ..Default::default()
    };
    let h = Harness::start_with(opts, "context_size = 1000000").await;
    let (sid, _) = long_chat(&h, 9).await;
    let msgs = h.get(&format!("/sessions/{sid}/messages?limit=200")).await;
    let list = msgs["messages"].as_array().expect("messages");
    assert!(
        list.iter().any(|m| m["role"] == "summary"),
        "the refused prompt was summarized and retried"
    );
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn pictures_are_stored_served_and_sent_to_the_model() {
    use base64::Engine;
    let h = Harness::start().await;
    let (st, v) = h
        .post("/projects/chats/sessions", json!({"title": "Pics"}))
        .await;
    assert_eq!(st, 201, "{v}");
    let sid = v["session"]["id"].as_str().expect("id").to_string();

    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::new(3000, 2000))
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("png");
    let url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png.into_inner())
    );

    // Not a picture.
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "what is this?", "images": ["data:image/png;base64,aGVsbG8="]}),
        )
        .await;
    assert_eq!(st, 400, "{v}");

    let mut ws = h.ws(&sid).await;
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "what is this?", "images": [url.clone()]}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
    let name = v["message"]["images"][0]
        .as_str()
        .expect("stored")
        .to_string();
    ws.until_status(&["done", "error"]).await;
    let msgs = h.get(&format!("/sessions/{sid}/messages")).await;
    let answer = msgs["messages"][1]["content"].as_str().unwrap_or("");
    assert!(answer.contains("[1 picture(s)] what is this?"), "{answer}");

    // The app fetches the picture back, scaled down.
    let res = h
        .http
        .get(format!("{}/attachments/{name}", h.base))
        .bearer_auth(&h.token)
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 200);
    let bytes = res.bytes().await.expect("bytes");
    let img = image::load_from_memory(&bytes).expect("picture");
    assert_eq!((img.width(), img.height()), (1568, 1045));
    // Without the token: refused.
    let res = h
        .http
        .get(format!("{}/attachments/{name}", h.base))
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 401);

    // A picture alone is a fine prompt.
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "", "images": [url]}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
    ws.until_status(&["done", "error"]).await;
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn files_are_stored_and_their_text_goes_to_the_model() {
    use base64::Engine;
    let h = Harness::start().await;
    let (st, v) = h
        .post("/projects/chats/sessions", json!({"title": "Files"}))
        .await;
    assert_eq!(st, 201, "{v}");
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    let b64 = |b: &[u8]| {
        format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b)
        )
    };

    let mut ws = h.ws(&sid).await;
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "explain", "files": [
                {"name": "notes.md", "data": b64(b"# Plan\nship it")},
                {"name": "blob.bin", "data": b64(&[0, 159, 146, 150])},
            ]}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
    let files = v["message"]["files"].as_array().expect("files").clone();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["name"], "notes.md");
    assert_eq!(files[0]["kind"], "text");
    assert_eq!(files[1]["kind"], "other");
    ws.until_status(&["done", "error"]).await;
    let msgs = h.get(&format!("/sessions/{sid}/messages")).await;
    // The mock echoes what it was sent: the prompt, then the file's text.
    let answer = msgs["messages"][1]["content"].as_str().unwrap_or("");
    assert!(answer.contains("explain"), "{answer}");
    assert!(answer.contains("Attached file: notes.md"), "{answer}");
    assert!(answer.contains("# Plan\nship it"), "{answer}");
    assert!(answer.contains("Attached file: blob.bin"), "{answer}");
    // Stored messages keep the files; the stored copy downloads, never renders.
    assert_eq!(msgs["messages"][0]["files"][0]["name"], "notes.md");
    let stored = files[0]["stored"].as_str().expect("stored");
    let res = h
        .http
        .get(format!("{}/attachments/{stored}", h.base))
        .bearer_auth(&h.token)
        .send()
        .await
        .expect("get");
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()["content-type"].to_str().ok(),
        Some("application/octet-stream")
    );
    assert_eq!(
        res.bytes().await.expect("bytes").as_ref(),
        b"# Plan\nship it"
    );

    // A file alone is a fine prompt; too many is refused.
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"files": (0..9).map(|i| json!({"name": format!("{i}.txt"), "data": b64(b"x")})).collect::<Vec<_>>()}),
        )
        .await;
    assert_eq!(st, 400, "{v}");
}

#[tokio::test]
async fn the_model_loads_only_when_a_chat_needs_it() {
    let h = Harness::launch(Default::default(), "").await;
    // Nothing is loaded until a chat is opened or a prompt is sent.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.get("/model").await["state"], "idle");
    let s = h.get("/settings").await;
    assert_eq!(s["settings"]["llama"]["idle_unload_minutes"], 15);

    // A prompt loads it, and the task waits for it.
    let (st, v) = h.post("/projects/chats/sessions", json!({})).await;
    assert_eq!(st, 201, "{v}");
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    let (st, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "hello"}),
        )
        .await;
    assert_eq!(st, 202, "{v}");
    h.wait_model("ready").await;
    for _ in 0..100 {
        let s = h.get(&format!("/sessions/{sid}")).await;
        if s["session"]["status"] == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let s = h.get(&format!("/sessions/{sid}")).await;
    assert_eq!(s["session"]["status"], "done", "{s}");

    // Changing the timer doesn't restart the model.
    let (st, v) = h
        .req(
            reqwest::Method::PATCH,
            "/settings",
            Some(json!({"llama": {"idle_unload_minutes": 5}})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["model_restarting"], false);
    assert_eq!(v["settings"]["llama"]["idle_unload_minutes"], 5);

    // With unloading off, the model loads as soon as Relay starts.
    let h2 = Harness::launch(Default::default(), "idle_unload_minutes = 0").await;
    h2.wait_model("ready").await;
}

/// A computer pairs with a 6-digit code, dials in to the brain, and a
/// project whose folder is on that computer has its files written there by
/// the brain's agent, without the brain ever connecting to the computer.
#[tokio::test]
async fn linked_device_pairs_with_a_code_and_hosts_a_project() {
    let h = Harness::start().await;

    // The brain says who it is, without a token (for discovery).
    let hello: Value = h
        .http
        .get(format!("{}/hello", h.base))
        .header("accept", "application/json")
        .send()
        .await
        .expect("hello")
        .json()
        .await
        .expect("json");
    assert_eq!(hello["relay"], "brain");
    assert_eq!(h.get("/info").await["role"], "brain");

    // The "laptop": a home folder with a project folder in it.
    let home = h.dir.join("laptop-home");
    let app = home.join("code/app");
    std::fs::create_dir_all(&app).expect("mkdir");
    std::fs::write(app.join("AGENTS.md"), "Laptop rules.").expect("agents");
    let dev_cfg = h.dir.join("laptop-data/device.toml");

    // Without an active code, or with a wrong one, pairing is refused.
    let err = relay::link::pair_and_save(&dev_cfg, &h.base, "123456", Some("laptop"), Some(&home))
        .await
        .expect_err("no code yet");
    assert!(format!("{err:#}").contains("no pairing code"), "{err:#}");
    let (s, v) = h.post("/devices/pair-code", json!({})).await;
    assert_eq!(s, 200, "{v}");
    let code = v["code"].as_str().expect("code").to_string();
    assert_eq!(code.len(), 6);
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert!(
        relay::link::pair_and_save(&dev_cfg, &h.base, wrong, Some("laptop"), Some(&home))
            .await
            .is_err()
    );
    let cfg = relay::link::pair_and_save(&dev_cfg, &h.base, &code, Some("laptop"), Some(&home))
        .await
        .expect("pair");
    assert_eq!(cfg.api_token, h.token, "the window gets the brain's API token");
    assert_eq!(relay::link::DeviceConfig::load(&dev_cfg).as_ref(), Some(&cfg));
    // The code is single-use.
    assert!(
        relay::link::pair_and_save(&dev_cfg, &h.base, &code, Some("laptop"), Some(&home))
            .await
            .is_err()
    );

    // Paired but not connected yet: listed, offline.
    let devices = h.get("/devices").await;
    assert_eq!(devices["devices"][0]["name"], "laptop");
    assert_eq!(devices["devices"][0]["connection"], "link");
    assert_eq!(devices["devices"][0]["connected"], false);

    // The laptop dials in.
    let shutdown = tokio_util::sync::CancellationToken::new();
    let states = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = states.clone();
    tokio::spawn(relay::link::run_device_link(cfg.clone(), shutdown.clone(), move |s| {
        seen.lock().unwrap().push(s)
    }));
    let mut connected = false;
    for _ in 0..100 {
        if h.get("/devices").await["devices"][0]["connected"] == true {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(connected, "device never connected");
    assert!(states.lock().unwrap().contains(&relay::link::LinkState::Connected));
    let device_id = cfg.device_id.clone();

    // Browse the laptop's folders from the brain.
    let b = h.get(&format!("/devices/{device_id}/browse")).await;
    assert_eq!(b["folders"], json!(["code"]), "{b}");

    // A project on the laptop; it says where it lives.
    let (s, v) = h
        .post(
            "/projects",
            json!({"device_id": device_id, "path": app.to_string_lossy()}),
        )
        .await;
    assert_eq!(s, 201, "{v}");
    assert_eq!(v["project"]["name"], "app");
    assert_eq!(v["project"]["location"]["kind"], "device");
    assert_eq!(v["project"]["location"]["name"], "laptop");
    assert_eq!(v["project"]["location"]["online"], true);
    let pid = v["project"]["id"].as_str().expect("id").to_string();
    // A folder outside what the laptop shares is refused.
    let (s, _) = h
        .post(
            "/projects",
            json!({"device_id": device_id, "path": h.dir.to_string_lossy()}),
        )
        .await;
    assert_eq!(s, 400);

    // The brain's agent writes a file in the laptop's project folder.
    let (_, v) = h.post(&format!("/projects/{pid}/sessions"), json!({})).await;
    let sid = v["session"]["id"].as_str().expect("id").to_string();
    let mut ws = h.ws(&sid).await;
    ws.next().await;
    let (s, v) = h
        .post(
            &format!("/sessions/{sid}/prompt"),
            json!({"content": "create hello.txt containing hi"}),
        )
        .await;
    assert_eq!(s, 202, "{v}");
    let events = ws.until_status(&["done", "error", "stopped"]).await;
    assert_eq!(
        events.last().map(|e| e["status"].clone()),
        Some(json!("done")),
        "{events:#?}"
    );
    assert_eq!(
        std::fs::read_to_string(app.join("hello.txt")).expect("written on the laptop"),
        "hi"
    );
    // Files on the laptop download through the brain.
    let r = h
        .http
        .get(format!("{}/projects/{pid}/download?path=hello.txt", h.base))
        .bearer_auth(&h.token)
        .send()
        .await
        .expect("download");
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.expect("body"), "hi");

    // The laptop goes away: the project shows offline and prompts fail fast.
    shutdown.cancel();
    let mut offline = false;
    for _ in 0..100 {
        if h.get(&format!("/projects/{pid}")).await["project"]["location"]["online"] == false {
            offline = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(offline, "device never went offline");

    // Removing the device revokes its key.
    let (s, _) = h.req(reqwest::Method::DELETE, &format!("/devices/{device_id}"), None).await;
    assert_eq!(s, 200);
    let mut req = format!("{}/devices/link", h.ws_base)
        .into_client_request()
        .expect("req");
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", cfg.device_key).parse().expect("header"),
    );
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
    let _ = std::fs::remove_dir_all(&h.dir);
}
