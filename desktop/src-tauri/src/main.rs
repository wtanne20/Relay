//! Relay for Windows.
//!
//! Runs the Relay server in the background (no console window) and shows the
//! Relay app in a desktop window. Closing the window keeps Relay running in
//! the system tray so the phone can still reach it; "Quit Relay" in the tray
//! stops running tasks and llama-server, then exits.
//!
//! If another Relay is already serving on the configured port (for example
//! `relay serve` in a console), the window simply connects to it.

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use relay::config::LoadedConfig;
use relay::{ServeOptions, ServeOutcome};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tokio_util::sync::CancellationToken;

/// What the window needs to talk to the local server.
#[derive(Clone, Serialize)]
struct LocalConnection {
    /// http://127.0.0.1:<port>
    url: String,
    token: String,
    /// The folder that holds config.toml, logs and the database.
    data_dir: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum ServerStatus {
    Starting,
    Running,
    /// Another Relay already serves this port; the window uses it.
    External,
    Failed { message: String },
    Stopped,
}

struct Desktop {
    connection: Option<LocalConnection>,
    /// Set when config.toml could not be loaded.
    load_error: Option<String>,
    status: Mutex<ServerStatus>,
    shutdown: CancellationToken,
    server: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    quitting: AtomicBool,
}

#[tauri::command]
fn local_connection(state: tauri::State<'_, Desktop>) -> Result<LocalConnection, String> {
    match (&state.connection, &state.load_error) {
        (Some(c), _) => Ok(c.clone()),
        (None, Some(e)) => Err(e.clone()),
        (None, None) => Err("Relay is not configured".into()),
    }
}

#[tauri::command]
fn server_status(state: tauri::State<'_, Desktop>) -> ServerStatus {
    state.status.lock().map(|s| s.clone()).unwrap_or(ServerStatus::Starting)
}

/// Opens Relay's data folder (config, logs, models) in Explorer.
#[tauri::command]
fn open_data_folder(state: tauri::State<'_, Desktop>) -> Result<(), String> {
    let dir = state.connection.as_ref().map(|c| c.data_dir.clone()).ok_or("no data folder")?;
    relay::setup::open_in_browser(std::path::Path::new(&dir));
    Ok(())
}

/// Stops the server (tasks and llama-server) and exits.
#[tauri::command]
fn quit(app: AppHandle) {
    begin_quit(&app);
}

fn main() {
    let config_path = relay::config::default_config_path();
    let loaded = LoadedConfig::load(&config_path);
    init_logging(loaded.as_ref().ok().map(|c| c.logs_dir()));

    let (connection, load_error, cfg) = match loaded {
        Ok(cfg) => {
            let port = relay::port_of(&cfg.config.bind).to_string();
            let conn = LocalConnection {
                url: format!("http://127.0.0.1:{port}"),
                token: cfg.config.token.clone(),
                data_dir: cfg.data_dir.display().to_string(),
            };
            (Some(conn), None, Some(cfg))
        }
        Err(e) => {
            tracing::error!("could not load {}: {e:#}", config_path.display());
            (None, Some(format!("Could not load {}: {e:#}", config_path.display())), None)
        }
    };

    let start_hidden = std::env::args().any(|a| a == "--minimized");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .manage(Desktop {
            connection,
            load_error: load_error.clone(),
            status: Mutex::new(if load_error.is_some() {
                ServerStatus::Failed { message: load_error.clone().unwrap_or_default() }
            } else {
                ServerStatus::Starting
            }),
            shutdown: CancellationToken::new(),
            server: Mutex::new(None),
            quitting: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![local_connection, server_status, open_data_folder, quit])
        .setup(move |app| {
            build_tray(app.handle())?;
            if let Some(cfg) = cfg {
                start_server(app.handle().clone(), cfg);
            }
            if !start_hidden {
                show_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps Relay running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let quitting = window.app_handle().state::<Desktop>().quitting.load(Ordering::SeqCst);
                if !quitting {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("could not start the Relay window: {e}");
            return;
        }
    };

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // Exits we did not start ourselves (for example Windows signing
            // out) go through the same clean shutdown.
            let state = app.state::<Desktop>();
            if code.is_none() && !state.quitting.load(Ordering::SeqCst) {
                api.prevent_exit();
                begin_quit(app);
            }
        }
    });
}

fn start_server(app: AppHandle, cfg: LoadedConfig) {
    let shutdown = app.state::<Desktop>().shutdown.clone();
    let handle = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        set_status(&handle, ServerStatus::Running);
        let opts = ServeOptions { console: false, open_pairing_page: false, shutdown };
        let status = match relay::run_server(cfg, opts).await {
            Ok(ServeOutcome::AlreadyRunning) => {
                tracing::info!("another Relay is already running; using it");
                ServerStatus::External
            }
            Ok(ServeOutcome::Stopped) => ServerStatus::Stopped,
            Err(e) => {
                tracing::error!("Relay server stopped with an error: {e:#}");
                ServerStatus::Failed { message: format!("{e:#}") }
            }
        };
        set_status(&handle, status);
    });
    if let Ok(mut slot) = app.state::<Desktop>().server.lock() {
        *slot = Some(task);
    }
}

fn set_status(app: &AppHandle, status: ServerStatus) {
    if let Ok(mut s) = app.state::<Desktop>().status.lock() {
        *s = status;
    }
}

/// Stops the server (waiting up to 20 s for llama-server to exit), then quits.
fn begin_quit(app: &AppHandle) {
    let state = app.state::<Desktop>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    state.shutdown.cancel();
    let task = state.server.lock().ok().and_then(|mut s| s.take());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(task) = task
            && tokio::time::timeout(Duration::from_secs(20), task).await.is_err()
        {
            tracing::warn!("Relay did not stop within 20 seconds; exiting anyway");
        }
        app.exit(0);
    });
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Relay", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open data folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Relay", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &folder, &sep, &quit])?;

    let mut tray = TrayIconBuilder::with_id("relay")
        .tooltip("Relay")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "folder" => {
                if let Some(c) = &app.state::<Desktop>().connection {
                    relay::setup::open_in_browser(std::path::Path::new(&c.data_dir));
                }
            }
            "quit" => begin_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Logs to <data>/logs/desktop.log (there is no console in release builds).
fn init_logging(logs_dir: Option<std::path::PathBuf>) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "relay=info,relay_desktop=info,tower_http=warn".into());
    let file = logs_dir.and_then(|d| {
        std::fs::create_dir_all(&d).ok()?;
        std::fs::OpenOptions::new().create(true).append(true).open(d.join("desktop.log")).ok()
    });
    match file {
        Some(f) => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(f)).try_init();
        }
        None => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
        }
    }
}
