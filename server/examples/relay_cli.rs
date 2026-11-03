//! Small command-line client: sends a prompt and prints streamed events.
//!
//!   cargo run --example relay_cli -- --project C:\code\myapp "create hello.txt containing hi"
//!
//! The token is read from --token, the RELAY_TOKEN variable, or ./config.toml.

use std::pin::Pin;

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[derive(Parser)]
struct Args {
    /// Relay base URL.
    #[arg(long, default_value = "http://127.0.0.1:8765")]
    url: String,
    /// API token (defaults to RELAY_TOKEN or the token in ./config.toml).
    #[arg(long)]
    token: Option<String>,
    /// Project id or absolute folder path (created if it doesn't exist yet).
    #[arg(long)]
    project: Option<String>,
    /// Existing session id (otherwise a new session is created).
    #[arg(long)]
    session: Option<String>,
    /// Print reasoning deltas too.
    #[arg(long)]
    reasoning: bool,
    /// The prompt.
    prompt: String,
}

fn token(args: &Args) -> Result<String> {
    if let Some(t) = &args.token {
        return Ok(t.clone());
    }
    if let Ok(t) = std::env::var("RELAY_TOKEN") {
        return Ok(t);
    }
    let text = std::fs::read_to_string("config.toml")
        .context("no --token, RELAY_TOKEN or ./config.toml")?;
    let cfg: relay::config::Config = toml::from_str(&text)?;
    Ok(cfg.token)
}

async fn call(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    token: &str,
    body: Option<Value>,
) -> Result<Value> {
    let mut r = http.request(method, &url).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    let resp = r
        .send()
        .await
        .with_context(|| format!("request to {url}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("{url}: {status} {}", v["error"].as_str().unwrap_or(""));
    }
    Ok(v)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let token = token(&args)?;
    let http = reqwest::Client::new();
    let base = args.url.trim_end_matches('/').to_string();

    let session_id = match &args.session {
        Some(s) => s.clone(),
        None => {
            let Some(p) = &args.project else {
                bail!("pass --project or --session")
            };
            let projects = call(
                &http,
                reqwest::Method::GET,
                format!("{base}/projects"),
                &token,
                None,
            )
            .await?;
            let found = projects["projects"].as_array().and_then(|a| {
                a.iter().find(|x| {
                    x["id"] == p.as_str()
                        || x["path"]
                            .as_str()
                            .is_some_and(|q| q.eq_ignore_ascii_case(p))
                })
            });
            let project_id = match found {
                Some(x) => x["id"].as_str().unwrap_or_default().to_string(),
                None => {
                    let v = call(
                        &http,
                        reqwest::Method::POST,
                        format!("{base}/projects"),
                        &token,
                        Some(json!({"path": p})),
                    )
                    .await?;
                    v["project"]["id"].as_str().unwrap_or_default().to_string()
                }
            };
            let v = call(
                &http,
                reqwest::Method::POST,
                format!("{base}/projects/{project_id}/sessions"),
                &token,
                Some(json!({})),
            )
            .await?;
            v["session"]["id"].as_str().unwrap_or_default().to_string()
        }
    };
    eprintln!("session {session_id}");

    let ws_url = format!(
        "{}/sessions/{session_id}/ws",
        base.replacen("http", "ws", 1)
    );
    let mut req = ws_url.into_client_request()?;
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse()?);
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .context("websocket connect")?;

    call(
        &http,
        reqwest::Method::POST,
        format!("{base}/sessions/{session_id}/prompt"),
        &token,
        Some(json!({"content": args.prompt})),
    )
    .await?;

    use futures_core::Stream;
    loop {
        let Some(msg) = std::future::poll_fn(|cx| Pin::new(&mut ws).poll_next(cx)).await else {
            break;
        };
        let Message::Text(t) = msg? else { continue };
        let ev: Value = serde_json::from_str(t.as_str())?;
        match ev["type"].as_str().unwrap_or("") {
            "text_delta" => print!("{}", ev["text"].as_str().unwrap_or("")),
            "reasoning_delta" if args.reasoning => {
                print!("\x1b[2m{}\x1b[0m", ev["text"].as_str().unwrap_or(""))
            }
            "tool_call" => println!(
                "\n[tool] {} {}",
                ev["name"].as_str().unwrap_or(""),
                ev["arguments"].as_str().unwrap_or("")
            ),
            "tool_result" => {
                let c = ev["content"].as_str().unwrap_or("");
                println!(
                    "[result{}] {}",
                    if ev["is_error"] == true { " ERROR" } else { "" },
                    c.lines().take(8).collect::<Vec<_>>().join("\n         ")
                );
            }
            "error" => eprintln!("\n[error] {}", ev["message"].as_str().unwrap_or("")),
            "status" => {
                let s = ev["status"].as_str().unwrap_or("");
                eprintln!("\n[status] {s}");
                if matches!(s, "done" | "error" | "stopped") {
                    break;
                }
            }
            _ => {}
        }
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    Ok(())
}
