//! Relay: a personal coding-agent server backed by llama.cpp's llama-server.

pub mod agent;
pub mod api;
pub mod config;
pub mod db;
pub mod migrate;
pub mod model_manager;
pub mod push;
pub mod skills;
pub mod tools;
pub mod ws;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

use crate::api::{AppInner, AppState};
use crate::config::{DEFAULT_SYSTEM_PROMPT, LoadedConfig};
use crate::db::Db;
use crate::model_manager::ModelManager;
use crate::push::Pusher;

/// Opens the database, prepares the data directory, starts the model
/// supervisor and returns the router plus shared state.
pub async fn build_app(cfg: LoadedConfig) -> Result<(axum::Router, AppState)> {
    let cfg = Arc::new(cfg);
    std::fs::create_dir_all(cfg.logs_dir()).context("creating logs folder")?;
    std::fs::create_dir_all(cfg.data_dir.join("skills")).context("creating skills folder")?;
    let prompt = cfg.system_prompt_path();
    if !prompt.exists() {
        std::fs::write(&prompt, DEFAULT_SYSTEM_PROMPT)
            .with_context(|| format!("writing {}", prompt.display()))?;
    }

    let db_path = cfg.db_path();
    let db = tokio::task::spawn_blocking(move || Db::open(&db_path)).await??;
    let stale = db.call(|c| db::reset_stale_running(c)).await?;
    if stale > 0 {
        tracing::warn!("marked {stale} interrupted session(s) as stopped");
    }

    let model = ModelManager::start(cfg.config.llama.clone(), cfg.logs_dir());
    let pusher = Pusher::new(db.clone(), cfg.config.push.enabled);
    let agent = agent::AgentManager::new(db.clone(), model.clone(), cfg.clone(), pusher.clone());
    let state: AppState = Arc::new(AppInner {
        cfg,
        db,
        model,
        agent,
        pusher,
        shutdown: CancellationToken::new(),
        started_at: chrono::Utc::now(),
    });
    Ok((api::router(state.clone()), state))
}

/// `relay serve`: runs until Ctrl+C, then stops tasks and llama-server.
pub async fn serve(cfg: LoadedConfig) -> Result<()> {
    let bind = cfg.config.bind.clone();
    let token = cfg.config.token.clone();
    let generated = cfg.token_generated;
    let cfg_path = cfg.path.clone();

    let (app, state) = build_app(cfg).await?;
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding {bind} (is another Relay already running?)"))?;

    println!();
    println!("  Relay {} listening on {bind}", env!("CARGO_PKG_VERSION"));
    println!("  Config: {}", cfg_path.display());
    if generated {
        println!("  A new API token was generated and saved to config.toml.");
    }
    println!("  API token: {token}");
    println!(
        "  Connect from your phone over Tailscale: http://<this PC's Tailscale name or IP>:{}",
        port_of(&bind)
    );
    println!("  Press Ctrl+C to stop.");
    println!();

    let shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl+C received, shutting down");
        }
        shutdown.cancel();
    });

    let graceful = state.shutdown.clone();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move { graceful.cancelled().await });
    let forced = state.shutdown.clone();
    tokio::select! {
        r = server => r.context("server error")?,
        _ = async move {
            forced.cancelled().await;
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        } => tracing::warn!("connections did not close in time"),
    }

    tracing::info!("stopping running tasks");
    state.agent.stop_all().await;
    tracing::info!("stopping llama-server");
    state.model.shutdown().await;
    tracing::info!("bye");
    Ok(())
}

fn port_of(bind: &str) -> &str {
    bind.rsplit(':').next().unwrap_or("8765")
}
