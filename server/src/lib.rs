//! Relay: a personal coding-agent server backed by llama.cpp's llama-server.

pub mod agent;
pub mod api;
pub mod config;
pub mod db;
pub mod hub;
pub mod migrate;
pub mod mockup;
pub mod model_manager;
pub mod process_guard;
pub mod push;
pub mod setup;
pub mod skills;
pub mod tools;
pub mod web;
pub mod ws;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

use crate::api::{AppInner, AppState};
use crate::config::{DEFAULT_SYSTEM_PROMPT, LoadedConfig, SharedConfig};
use crate::db::Db;
use crate::model_manager::ModelManager;
use crate::push::Pusher;

/// Opens the database, prepares the data directory, detects the GPU and
/// Tailscale, starts the model supervisor and returns the router plus shared
/// state.
pub async fn build_app(cfg: LoadedConfig) -> Result<(axum::Router, AppState)> {
    std::fs::create_dir_all(cfg.logs_dir()).context("creating logs folder")?;
    std::fs::create_dir_all(cfg.data_dir.join("skills")).context("creating skills folder")?;
    std::fs::create_dir_all(cfg.data_dir.join("models")).context("creating models folder")?;
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

    let (hardware, tailscale) = tokio::join!(setup::detect_hardware(), setup::detect_tailscale());
    for g in &hardware.gpus {
        tracing::info!("GPU: {} ({} MB)", g.name, g.vram_mb);
    }
    let data_dir = cfg.data_dir.clone();
    let shared = SharedConfig::new(cfg);
    let model = ModelManager::start(shared.clone(), hardware.clone());
    let pusher = Pusher::new(db.clone(), shared.clone());
    let agent = agent::AgentManager::new(db.clone(), model.clone(), shared.clone(), pusher.clone());
    let state: AppState = Arc::new(AppInner {
        cfg: shared,
        hardware,
        tailscale,
        db,
        model,
        agent,
        pusher,
        shutdown: CancellationToken::new(),
        started_at: chrono::Utc::now(),
        phone_seen: std::sync::Mutex::new(api::load_phone_seen(&data_dir)),
        downloads: Default::default(),
        data_dir,
    });
    Ok((api::router(state.clone()), state))
}

/// How [`run_server`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeOutcome {
    /// Another Relay already listens on the port; nothing was started.
    AlreadyRunning,
    /// The server ran and was stopped through the shutdown token.
    Stopped,
}

/// Options for [`run_server`].
pub struct ServeOptions {
    /// Print the QR code and connection details to the console.
    pub console: bool,
    /// Open the pairing page in the browser on the very first run.
    pub open_pairing_page: bool,
    /// Cancel this to stop the server; running tasks and llama-server are
    /// stopped before `run_server` returns.
    pub shutdown: CancellationToken,
}

/// `relay serve`: runs until Ctrl+C (or the window is closed), then stops
/// tasks and llama-server.
pub async fn serve(cfg: LoadedConfig) -> Result<()> {
    let bind = cfg.config.bind.clone();
    let token = cfg.config.token.clone();
    let data_dir = cfg.data_dir.clone();
    let shutdown = CancellationToken::new();
    let signal = shutdown.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        signal.cancel();
    });
    let opts = ServeOptions {
        console: true,
        open_pairing_page: true,
        shutdown,
    };
    if run_server(cfg, opts).await? == ServeOutcome::AlreadyRunning {
        println!();
        println!(
            "  Relay is already running on this PC (port {} is in use).",
            port_of(&bind)
        );
        println!("  Opening the pairing page instead...");
        let ts = setup::detect_tailscale().await;
        let url = setup::pairing_url(ts.as_ref(), port_of(&bind));
        if let Ok(page) = setup::write_pairing_page(
            &data_dir,
            &url,
            &setup::pairing_payload(&url, &token),
            ts.is_some(),
        ) {
            setup::open_in_browser(&page);
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    Ok(())
}

/// Runs the Relay server until `opts.shutdown` is cancelled. Used by
/// `relay serve` and by the Relay desktop app, which runs the server in the
/// background.
pub async fn run_server(cfg: LoadedConfig, opts: ServeOptions) -> Result<ServeOutcome> {
    let bind = cfg.config.bind.clone();
    let token = cfg.config.token.clone();
    let cfg_path = cfg.path.clone();
    let data_dir = cfg.data_dir.clone();
    let first_run = !data_dir.join("pairing.html").exists();

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            return Ok(ServeOutcome::AlreadyRunning);
        }
        Err(e) => return Err(e).with_context(|| format!("binding {bind}")),
    };
    let (app, state) = build_app(cfg).await?;

    let url = setup::pairing_url(state.tailscale.as_ref(), port_of(&bind));
    let payload = setup::pairing_payload(&url, &token);
    let page = setup::write_pairing_page(&data_dir, &url, &payload, state.tailscale.is_some());

    if opts.console {
        println!();
        println!(
            "  Relay {} is running. Keep this window open (you can minimize it).",
            env!("CARGO_PKG_VERSION")
        );
        println!(
            "  Settings and data: {}",
            cfg_path.parent().unwrap_or(&data_dir).display()
        );
        println!();
        match setup::qr_console(&payload) {
            Ok(qr) => {
                println!("  Scan this with the Relay phone app to connect:");
                println!("{qr}");
            }
            Err(e) => tracing::warn!("could not draw QR code: {e}"),
        }
        println!("  Or enter it by hand:");
        println!("    Address: {url}");
        println!("    Token:   {token}");
        if state.tailscale.is_none() {
            println!();
            println!(
                "  Tailscale was not found on this PC. Install it from https://tailscale.com and"
            );
            println!("  sign in with the same account as your phone, then restart Relay.");
        }
        println!();
        println!("  Press Ctrl+C or close this window to stop Relay.");
        println!();
    }

    match page {
        Ok(p) if first_run && opts.open_pairing_page => setup::open_in_browser(&p),
        Ok(_) => {}
        Err(e) => tracing::warn!("could not write pairing page: {e:#}"),
    }

    let external = opts.shutdown.clone();
    let internal = state.shutdown.clone();
    tokio::spawn(async move {
        external.cancelled().await;
        internal.cancel();
    });

    let graceful = state.shutdown.clone();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
    Ok(ServeOutcome::Stopped)
}

/// `relay pair`: shows the QR code (console and browser) without starting
/// the server.
pub async fn pair(cfg: LoadedConfig) -> Result<()> {
    let ts = setup::detect_tailscale().await;
    let url = setup::pairing_url(ts.as_ref(), port_of(&cfg.config.bind));
    let payload = setup::pairing_payload(&url, &cfg.config.token);
    let page = setup::write_pairing_page(&cfg.data_dir, &url, &payload, ts.is_some())?;
    println!("{}", setup::qr_console(&payload)?);
    println!("  Address: {url}");
    println!("  Token:   {}", cfg.config.token);
    if ts.is_none() {
        println!("  Tailscale was not found on this PC; install it from https://tailscale.com");
    }
    setup::open_in_browser(&page);
    Ok(())
}

/// Ctrl+C everywhere; on Windows also closing the console window, logoff and
/// system shutdown, so llama-server is stopped instead of left running.
async fn wait_for_shutdown_signal() {
    #[cfg(windows)]
    {
        use tokio::signal::windows;
        let (mut close, mut logoff, mut shutdown) = match (
            windows::ctrl_close(),
            windows::ctrl_logoff(),
            windows::ctrl_shutdown(),
        ) {
            (Ok(c), Ok(l), Ok(s)) => (c, l, s),
            _ => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("Ctrl+C received, shutting down"),
            _ = close.recv() => tracing::info!("console closing, shutting down"),
            _ = logoff.recv() => tracing::info!("user logging off, shutting down"),
            _ = shutdown.recv() => tracing::info!("system shutting down"),
        }
    }
    #[cfg(not(windows))]
    {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl+C received, shutting down");
        }
    }
}

/// The port part of a bind address such as `0.0.0.0:8765`.
pub fn port_of(bind: &str) -> &str {
    bind.rsplit(':').next().unwrap_or("8765")
}
