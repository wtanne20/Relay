//! Relay for Windows and Linux.
//!
//! One app, two roles, chosen on first launch:
//!
//! - **Brain**: runs the Relay server in the background (no console window)
//!   and shows the Relay app in a desktop window. This computer runs the
//!   model and keeps the chats.
//! - **Linked computer**: pairs with a brain elsewhere on the tailnet using
//!   the 6-digit code the brain shows, then keeps a link open so the brain's
//!   agent can work in this computer's files. The window shows the brain's
//!   chats. Nothing listens on this computer; it dials out to the brain.
//!
//! Closing the window keeps Relay running in the system tray (so the phone
//! can still reach a brain, and a linked computer stays linked); "Quit Relay"
//! in the tray stops everything and exits.
//!
//! If another Relay is already serving on the configured port (for example
//! `relay serve` in a console), a brain window simply connects to it.

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use relay::config::LoadedConfig;
use relay::link::{DeviceConfig, FoundBrain, LinkState};
use relay::{ServeOptions, ServeOutcome};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tokio_util::sync::CancellationToken;

/// What the window needs to talk to its brain.
#[derive(Clone, Serialize)]
struct LocalConnection {
    /// http://127.0.0.1:<port> for a brain; the brain's address when linked.
    url: String,
    token: String,
    /// The folder that holds config.toml / device.toml, logs and the database.
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

/// What this computer is.
#[derive(Clone, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum AppMode {
    /// First launch: nothing chosen yet.
    Setup { hostname: String, home: String },
    Brain { hostname: String },
    Device {
        hostname: String,
        device_id: String,
        name: String,
        brain_name: String,
        brain_url: String,
        root: String,
        link: LinkState,
    },
}

/// Which role is running.
enum Role {
    Setup,
    Brain,
    Device(DeviceConfig),
}

struct Desktop {
    data_dir: PathBuf,
    config_path: PathBuf,
    role: Mutex<Role>,
    connection: Mutex<Option<LocalConnection>>,
    /// Set when config.toml could not be loaded.
    load_error: Mutex<Option<String>>,
    status: Mutex<ServerStatus>,
    link: Arc<Mutex<LinkState>>,
    /// Cancels whatever role is running (the server or the link).
    shutdown: Mutex<CancellationToken>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    quitting: AtomicBool,
}

impl Desktop {
    fn mode(&self) -> AppMode {
        let hostname = relay::link::hostname();
        match &*self.role.lock().unwrap_or_else(|e| e.into_inner()) {
            Role::Setup => AppMode::Setup {
                hostname,
                home: relay::link::home_dir().display().to_string(),
            },
            Role::Brain => AppMode::Brain { hostname },
            Role::Device(cfg) => AppMode::Device {
                hostname,
                device_id: cfg.device_id.clone(),
                name: cfg.name.clone(),
                brain_name: cfg.brain_name.clone(),
                brain_url: cfg.brain_url.clone(),
                root: cfg.root.display().to_string(),
                link: self.link.lock().map(|l| l.clone()).unwrap_or(LinkState::Connecting),
            },
        }
    }
}

#[tauri::command]
fn local_connection(state: tauri::State<'_, Desktop>) -> Result<LocalConnection, String> {
    if let Some(c) = state.connection.lock().map_err(|e| e.to_string())?.clone() {
        return Ok(c);
    }
    match state.load_error.lock().map_err(|e| e.to_string())?.clone() {
        Some(e) => Err(e),
        None => Err("Relay is not set up yet".into()),
    }
}

#[tauri::command]
fn server_status(state: tauri::State<'_, Desktop>) -> ServerStatus {
    state.status.lock().map(|s| s.clone()).unwrap_or(ServerStatus::Starting)
}

#[tauri::command]
fn app_mode(state: tauri::State<'_, Desktop>) -> AppMode {
    state.mode()
}

/// First launch: this computer becomes the brain.
#[tauri::command]
fn become_brain(app: AppHandle) -> Result<(), String> {
    let state = app.state::<Desktop>();
    let cfg = LoadedConfig::load(&state.config_path).map_err(|e| format!("{e:#}"))?;
    start_brain(&app, cfg);
    Ok(())
}

/// Brains on the tailnet (and on this computer).
#[tauri::command]
async fn find_brains() -> Vec<FoundBrain> {
    relay::link::discover_brains().await
}

/// Pairs with a brain using its code, then stays linked.
#[tauri::command]
async fn link_to_brain(
    app: AppHandle,
    brain: String,
    code: String,
    root: Option<String>,
) -> Result<AppMode, String> {
    let state = app.state::<Desktop>();
    let path = DeviceConfig::path_in(&state.data_dir);
    let root = root.filter(|r| !r.trim().is_empty()).map(PathBuf::from);
    let cfg = relay::link::pair_and_save(&path, &brain, &code, None, root.as_deref())
        .await
        .map_err(|e| format!("{e:#}"))?;
    start_device(&app, cfg);
    set_autostart(true);
    Ok(app.state::<Desktop>().mode())
}

/// Forgets the brain and goes back to the first-launch choice.
#[tauri::command]
fn unlink(app: AppHandle) -> Result<(), String> {
    let state = app.state::<Desktop>();
    stop_role(&state);
    let path = DeviceConfig::path_in(&state.data_dir);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    set_autostart(false);
    *state.role.lock().map_err(|e| e.to_string())? = Role::Setup;
    *state.connection.lock().map_err(|e| e.to_string())? = None;
    update_tray(&app);
    Ok(())
}

/// Opens Relay's data folder (config, logs, models).
#[tauri::command]
fn open_data_folder(state: tauri::State<'_, Desktop>) -> Result<(), String> {
    relay::setup::open_in_browser(&state.data_dir);
    Ok(())
}

/// Stops the server or link and exits.
#[tauri::command]
fn quit(app: AppHandle) {
    begin_quit(&app);
}

fn main() {
    let config_path = relay::config::default_config_path();
    let data_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(relay::config::default_data_dir);
    let device = DeviceConfig::load(&DeviceConfig::path_in(&data_dir));
    // An existing brain (config or database) stays a brain; a fresh install
    // asks what this computer should be.
    let is_brain = device.is_none()
        && (config_path.exists() || data_dir.join("relay.db").exists());
    let logs = data_dir.join("logs");
    init_logging(Some(logs));

    let (brain_cfg, load_error) = if is_brain {
        match LoadedConfig::load(&config_path) {
            Ok(cfg) => (Some(cfg), None),
            Err(e) => {
                tracing::error!("could not load {}: {e:#}", config_path.display());
                (None, Some(format!("Could not load {}: {e:#}", config_path.display())))
            }
        }
    } else {
        (None, None)
    };

    let start_hidden = std::env::args().any(|a| a == "--minimized");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .manage(Desktop {
            data_dir,
            config_path,
            role: Mutex::new(Role::Setup),
            connection: Mutex::new(None),
            load_error: Mutex::new(load_error.clone()),
            status: Mutex::new(match &load_error {
                Some(m) => ServerStatus::Failed { message: m.clone() },
                None => ServerStatus::Starting,
            }),
            link: Arc::new(Mutex::new(LinkState::Connecting)),
            shutdown: Mutex::new(CancellationToken::new()),
            task: Mutex::new(None),
            quitting: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            local_connection,
            server_status,
            app_mode,
            become_brain,
            find_brains,
            link_to_brain,
            unlink,
            open_data_folder,
            quit
        ])
        .setup(move |app| {
            build_tray(app.handle())?;
            if let Some(cfg) = brain_cfg {
                start_brain(app.handle(), cfg);
            } else if let Some(dev) = device {
                start_device(app.handle(), dev);
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
            // Exits we did not start ourselves (for example signing out)
            // go through the same clean shutdown.
            let state = app.state::<Desktop>();
            if code.is_none() && !state.quitting.load(Ordering::SeqCst) {
                api.prevent_exit();
                begin_quit(app);
            }
        }
    });
}

/// Stops the running role (server or link) without waiting.
fn stop_role(state: &Desktop) {
    if let Ok(mut s) = state.shutdown.lock() {
        s.cancel();
        *s = CancellationToken::new();
    }
}

fn start_brain(app: &AppHandle, cfg: LoadedConfig) {
    let state = app.state::<Desktop>();
    stop_role(&state);
    let port = relay::port_of(&cfg.config.bind).to_string();
    let conn = LocalConnection {
        url: format!("http://127.0.0.1:{port}"),
        token: cfg.config.token.clone(),
        data_dir: cfg.data_dir.display().to_string(),
    };
    if let Ok(mut c) = state.connection.lock() {
        *c = Some(conn);
    }
    if let Ok(mut r) = state.role.lock() {
        *r = Role::Brain;
    }
    if let Ok(mut e) = state.load_error.lock() {
        *e = None;
    }
    let shutdown = state.shutdown.lock().map(|s| s.clone()).unwrap_or_default();
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
    if let Ok(mut slot) = state.task.lock() {
        *slot = Some(task);
    }
    update_tray(app);
}

fn start_device(app: &AppHandle, cfg: DeviceConfig) {
    let state = app.state::<Desktop>();
    stop_role(&state);
    if let Ok(mut c) = state.connection.lock() {
        *c = Some(LocalConnection {
            url: cfg.brain_url.clone(),
            token: cfg.api_token.clone(),
            data_dir: state.data_dir.display().to_string(),
        });
    }
    if let Ok(mut r) = state.role.lock() {
        *r = Role::Device(cfg.clone());
    }
    set_status(app, ServerStatus::External);
    let shutdown = state.shutdown.lock().map(|s| s.clone()).unwrap_or_default();
    let link = Arc::clone(&state.link);
    if let Ok(mut l) = link.lock() {
        *l = LinkState::Connecting;
    }
    let handle = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let on_state = {
            let link = Arc::clone(&link);
            let handle = handle.clone();
            move |s: LinkState| {
                if let Ok(mut l) = link.lock() {
                    *l = s;
                }
                update_tray(&handle);
            }
        };
        if let Err(e) = relay::link::run_device_link(cfg, shutdown, on_state).await {
            tracing::error!("device link stopped: {e:#}");
            if let Ok(mut l) = link.lock() {
                *l = LinkState::Offline { message: format!("{e:#}") };
            }
        }
    });
    if let Ok(mut slot) = state.task.lock() {
        *slot = Some(task);
    }
    update_tray(app);
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
    if let Ok(s) = state.shutdown.lock() {
        s.cancel();
    }
    let task = state.task.lock().ok().and_then(|mut s| s.take());
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

/// The tray tooltip says what this computer is right now.
fn tray_tooltip(app: &AppHandle) -> String {
    match app.state::<Desktop>().mode() {
        AppMode::Setup { .. } => "Relay: not set up".into(),
        AppMode::Brain { .. } => "Relay: this computer is the brain".into(),
        AppMode::Device { brain_name, link, .. } => {
            let brain = if brain_name.is_empty() { "the brain".to_string() } else { brain_name };
            match link {
                LinkState::Connected => format!("Relay: linked to {brain}"),
                LinkState::Connecting => format!("Relay: connecting to {brain}…"),
                LinkState::Offline { .. } => format!("Relay: can't reach {brain}"),
                LinkState::Revoked => format!("Relay: {brain} removed this computer"),
            }
        }
    }
}

fn update_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id("relay") {
        let _ = tray.set_tooltip(Some(tray_tooltip(app)));
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
            "folder" => relay::setup::open_in_browser(&app.state::<Desktop>().data_dir),
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

/// Linux: start Relay (in the tray) on login, so a linked computer stays
/// linked after a restart. Uses the XDG autostart folder every desktop
/// (GNOME, KDE, XFCE, ...) reads. Elsewhere this does nothing.
fn set_autostart(on: bool) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| relay::link::home_dir().join(".config"))
        .join("autostart");
    let file = dir.join("relay.desktop");
    if !on {
        let _ = std::fs::remove_file(&file);
        return;
    }
    // An AppImage runs from a temporary mount; $APPIMAGE is the real file.
    let exe = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else { return };
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Relay\nComment=Keeps this computer linked to your Relay brain\nExec=\"{}\" --minimized\nIcon=relay-desktop\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    if std::fs::create_dir_all(&dir).is_ok()
        && let Err(e) = std::fs::write(&file, entry)
    {
        tracing::warn!("could not set Relay to start on login: {e}");
    }
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
