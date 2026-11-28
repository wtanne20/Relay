//! Browse and install: skills from a (mock) GitHub collection, ready-made
//! tools and MCP registry search.

use std::path::PathBuf;

use axum::extract::{Path, Query};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use relay::config::LoadedConfig;

const COMMIT: &str = "c0ffee0000000000000000000000000000000001";

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("README.md", "# Skills"),
        (
            "skills/pdf/SKILL.md",
            "---\nname: pdf\ndescription: Work with PDF files\n---\nUse the scripts.\n",
        ),
        ("skills/pdf/scripts/fill.py", "print('fill')\n"),
        (
            "skills/greeting/SKILL.md",
            "---\nname: greeting\ndescription: Another greeting skill\n---\nHello.\n",
        ),
        (
            "skills/haiku/SKILL.md",
            "---\nname: haiku\ndescription: >\n  Write haiku\n  in three lines\n---\nFive seven five.\n",
        ),
    ]
}

async fn mock_github(version: &'static str) -> String {
    let tree = move || {
        let entries: Vec<Value> = files()
            .iter()
            .map(|(p, c)| {
                let sha = if *p == "skills/pdf/scripts/fill.py" {
                    format!("{version}-{p}")
                } else {
                    format!("sha-{p}")
                };
                json!({ "path": p, "type": "blob", "sha": sha, "size": c.len() })
            })
            .chain([json!({ "path": "skills", "type": "tree", "sha": "t" })])
            .collect();
        json!({ "sha": "tree", "tree": entries, "truncated": false })
    };
    let app = Router::new()
        .route(
            "/repos/{owner}/{repo}/commits/{git_ref}",
            get(|Path((o, r, _g)): Path<(String, String, String)>| async move {
                if o == "acme" && r == "skills" {
                    Ok(Json(json!({ "sha": COMMIT })))
                } else {
                    Err(axum::http::StatusCode::NOT_FOUND)
                }
            }),
        )
        .route(
            "/repos/{owner}/{repo}/git/trees/{sha}",
            get(move || async move { Json(tree()) }),
        )
        .route(
            "/acme/skills/{commit}/{*path}",
            get(|Path((_c, path)): Path<(String, String)>| async move {
                files()
                    .into_iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, c)| c.to_string())
                    .ok_or(axum::http::StatusCode::NOT_FOUND)
            }),
        )
        .route(
            "/v0/servers",
            get(|Query(q): Query<std::collections::HashMap<String, String>>| async move {
                assert_eq!(q.get("version").map(String::as_str), Some("latest"));
                let search = q.get("search").cloned().unwrap_or_default();
                if let Some(publisher) = search.strip_prefix('.') {
                    // Relay also asks for servers the service publishes itself.
                    assert_eq!(publisher, "paris/");
                    return Json(json!({ "servers": [], "metadata": { "count": 0 } }));
                }
                if q.get("cursor").map(String::as_str) == Some("page2") {
                    // The second page has the publisher's own server.
                    return Json(json!({
                        "servers": [{ "server": {
                            "name": format!("io.github.{search}/{search}-mcp"),
                            "description": "The official one",
                            "remotes": [{ "type": "streamable-http", "url": "https://x.example/mcp" }]
                        } }],
                        "metadata": { "count": 1 }
                    }));
                }
                Json(json!({
                    "servers": [{
                        "server": {
                            "name": "io.github.acme/weather-mcp",
                            "description": format!("Forecasts for {search}"),
                            "version": "1.0.0",
                            "packages": [{
                                "registryType": "npm", "identifier": "@acme/weather", "version": "1.0.0",
                                "transport": { "type": "stdio" },
                                "environmentVariables": [{ "name": "KEY", "isRequired": true, "isSecret": true }]
                            }]
                        },
                        "_meta": { "io.modelcontextprotocol.registry/official": { "status": "active" } }
                    }],
                    "metadata": { "nextCursor": "page2", "count": 1 }
                }))
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(axum::serve(l, app).into_future());
    url
}

struct H {
    base: String,
    token: String,
    http: reqwest::Client,
    dir: PathBuf,
}

impl H {
    async fn start() -> H {
        let dir = std::env::temp_dir().join(format!("relay-catalog-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data/skills/greeting")).unwrap();
        std::fs::write(
            dir.join("data/skills/greeting/SKILL.md"),
            "---\nname: greeting\ndescription: Mine\n---\n",
        )
        .unwrap();
        let cfg_path = dir.join("data/config.toml");
        std::fs::write(
            &cfg_path,
            "bind = \"127.0.0.1:0\"\nshell = \"sh\"\n[push]\nenabled = false\n",
        )
        .unwrap();
        let cfg = LoadedConfig::load(&cfg_path).unwrap();
        let token = cfg.config.token.clone();
        let (app, _state) = relay::build_app(cfg).await.unwrap();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(axum::serve(l, app).into_future());
        H {
            base,
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
        let resp = r.send().await.unwrap();
        let st = resp.status().as_u16();
        (st, resp.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> Value {
        let (st, v) = self.req(reqwest::Method::GET, path, None).await;
        assert_eq!(st, 200, "GET {path}: {v}");
        v
    }
}

fn skill<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {v}"))
}

#[tokio::test]
async fn browses_and_installs_skills_tools_and_mcp_servers() {
    let gh = mock_github("v1").await;
    // SAFETY: this test binary has only this test.
    unsafe {
        std::env::set_var("RELAY_GITHUB_API", &gh);
        std::env::set_var("RELAY_GITHUB_RAW", &gh);
        std::env::set_var("RELAY_MCP_REGISTRY", &gh);
    }
    let h = H::start().await;

    // anthropics/skills is always listed; this mock only knows acme/skills.
    let v = h.get("/catalog/skills").await;
    assert_eq!(v["sources"][0]["source"], "anthropics/skills");
    assert!(
        v["error"].as_str().unwrap().contains("no such repository"),
        "{v}"
    );

    let (st, v) = h
        .req(
            reqwest::Method::POST,
            "/catalog/skills/sources",
            Some(json!({ "source": "nobody/nothing" })),
        )
        .await;
    assert_eq!(st, 400, "{v}");
    let (st, v) = h
        .req(
            reqwest::Method::POST,
            "/catalog/skills/sources",
            Some(json!({ "source": "https://github.com/acme/skills/tree/main/skills" })),
        )
        .await;
    assert_eq!(st, 201, "{v}");
    assert_eq!(v["source"], "acme/skills/skills@main");
    assert_eq!(v["skills"], 3);
    let saved = std::fs::read_to_string(h.dir.join("data/config.toml")).unwrap();
    assert!(saved.contains("acme/skills/skills@main"), "{saved}");

    let src = "acme%2Fskills%2Fskills%40main";
    let v = h.get(&format!("/catalog/skills?source={src}")).await;
    assert_eq!(v["sources"].as_array().unwrap().len(), 2);
    assert_eq!(v["commit"], COMMIT);
    let pdf = skill(&v, "pdf");
    assert_eq!(pdf["path"], "skills/pdf");
    assert_eq!(pdf["files"], 2);
    assert_eq!(pdf["installed"], Value::Null);
    assert_eq!(
        skill(&v, "haiku")["description"],
        "Write haiku in three lines"
    );
    // The user's own greeting skill has the same name.
    assert_eq!(skill(&v, "greeting")["name_taken"], true);

    // Install pdf: both files land in data/skills/pdf with Relay's marker.
    let (st, v) = h
        .req(
            reqwest::Method::POST,
            "/catalog/skills/install",
            Some(json!({ "source": "acme/skills/skills@main", "path": "skills/pdf" })),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["name"], "pdf");
    assert_eq!(v["installed"]["folder"], "pdf");
    let dir = h.dir.join("data/skills/pdf");
    assert_eq!(
        std::fs::read_to_string(dir.join("scripts/fill.py")).unwrap(),
        "print('fill')\n"
    );
    assert!(dir.join(".relay-skill.json").is_file());
    let skills = h.get("/skills").await;
    assert!(
        skills["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "pdf"),
        "{skills}"
    );
    let v = h.get(&format!("/catalog/skills?source={src}")).await;
    assert_eq!(skill(&v, "pdf")["installed"]["update_available"], false);
    assert_eq!(v["installed"][0]["folder"], "pdf");

    // The greeting folder is the user's own: no overwrite, no uninstall.
    let (st, v) = h
        .req(
            reqwest::Method::POST,
            "/catalog/skills/install",
            Some(json!({ "source": "acme/skills/skills@main", "path": "skills/greeting" })),
        )
        .await;
    assert_eq!(st, 409, "{v}");
    let (st, _) = h
        .req(
            reqwest::Method::DELETE,
            "/catalog/skills/installed/greeting",
            None,
        )
        .await;
    assert_eq!(st, 404);
    let (st, _) = h
        .req(
            reqwest::Method::DELETE,
            "/catalog/skills/installed/..",
            None,
        )
        .await;
    assert!(st == 400 || st == 404, "{st}");
    let (st, v) = h
        .req(
            reqwest::Method::POST,
            "/catalog/skills/install",
            Some(json!({ "source": "acme/skills/skills@main", "path": "skills/nope" })),
        )
        .await;
    assert_eq!(st, 400, "{v}");

    // Reinstalling (an update) replaces the folder.
    std::fs::write(dir.join("stale.txt"), "old").unwrap();
    let (st, _) = h
        .req(
            reqwest::Method::POST,
            "/catalog/skills/install",
            Some(json!({ "source": "acme/skills/skills@main", "path": "skills/pdf" })),
        )
        .await;
    assert_eq!(st, 200);
    assert!(!dir.join("stale.txt").exists());

    let (st, _) = h
        .req(
            reqwest::Method::DELETE,
            "/catalog/skills/installed/pdf",
            None,
        )
        .await;
    assert_eq!(st, 204);
    assert!(!dir.exists());
    assert!(h.dir.join("data/skills/greeting/SKILL.md").exists());

    let (st, _) = h
        .req(
            reqwest::Method::DELETE,
            &format!("/catalog/skills/sources?source={src}"),
            None,
        )
        .await;
    assert_eq!(st, 204);
    let v = h.get("/catalog/skills").await;
    assert_eq!(v["sources"].as_array().unwrap().len(), 1);

    // Ready-made tools, for sh; adding one marks it installed.
    let v = h.get("/catalog/tools").await;
    assert_eq!(v["shell"], "sh");
    let tools = v["tools"].as_array().unwrap();
    let search = tools
        .iter()
        .find(|t| t["tool"]["name"] == "search_text")
        .expect("search_text");
    assert!(
        search["tool"]["command"]
            .as_str()
            .unwrap()
            .starts_with("grep")
    );
    assert_eq!(search["installed"], false);
    let (st, v) = h
        .req(
            reqwest::Method::POST,
            "/tools",
            Some(search["tool"].clone()),
        )
        .await;
    assert_eq!(st, 201, "{v}");
    let v = h.get("/catalog/tools").await;
    assert!(
        v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["tool"]["name"] == "search_text" && t["installed"] == true)
    );

    // MCP registry search, turned into launch settings.
    let v = h.get("/catalog/mcp?q=paris").await;
    assert_eq!(v["next_cursor"], Value::Null);
    assert_eq!(v["servers"].as_array().unwrap().len(), 2);
    assert_eq!(v["servers"][0]["id"], "io.github.paris/paris-mcp");
    let s = &v["servers"][1];
    assert_eq!(s["name"], "weather");
    assert_eq!(s["description"], "Forecasts for paris");
    let opt = &s["options"][0];
    assert_eq!(opt["command"], "npx");
    assert_eq!(opt["args"], json!(["-y", "@acme/weather@1.0.0"]));
    assert_eq!(opt["inputs"][0]["name"], "KEY");
    assert_eq!(opt["inputs"][0]["secret"], true);
}
