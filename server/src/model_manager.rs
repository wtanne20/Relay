//! llama-server process management: launch, health polling, supervision with
//! bounded auto-restart, logging, and clean shutdown.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::config::{LlamaConfig, SharedConfig};
use crate::setup::{self, Hardware};

const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);
/// How often a loaded model is checked for being unused.
const IDLE_CHECK: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    /// Not loaded, to leave the GPU free; it loads when a chat needs it.
    Idle,
    Starting,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub state: ModelState,
    pub message: Option<String>,
    /// True if Relay found a llama-server already running and is using it.
    pub external: bool,
    pub pid: Option<u32>,
    pub port: u16,
    pub model_path: String,
    /// Unexpected exits in the last 5 minutes.
    pub recent_restarts: usize,
    pub since: String,
    /// 0..1 while downloading llama.cpp during first-run setup.
    pub progress: Option<f32>,
}

enum Cmd {
    Restart,
    /// Load the model if it isn't loaded.
    Wake,
    /// Unload the model now, freeing the GPU.
    Unload,
    Shutdown(oneshot::Sender<()>),
}

/// When the model was last needed, and how many tasks need it now.
struct Activity {
    holds: AtomicUsize,
    last_used: std::sync::Mutex<Instant>,
}

impl Activity {
    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// True when no task needs the model and it hasn't been used for `after`.
    fn unused_for(&self, after: Duration) -> bool {
        self.holds.load(Ordering::SeqCst) == 0
            && self
                .last_used
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed()
                >= after
    }
}

/// Keeps the model loaded while it lives (one per running task).
pub struct ModelHold(Arc<Activity>);

impl Drop for ModelHold {
    fn drop(&mut self) {
        self.0.touch();
        self.0.holds.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct ModelManager {
    status: Arc<RwLock<ModelStatus>>,
    tx: mpsc::Sender<Cmd>,
    activity: Arc<Activity>,
}

impl ModelManager {
    /// Starts the supervisor task and returns immediately.
    pub fn start(shared: Arc<SharedConfig>, hardware: Hardware) -> Self {
        let cfg = shared.get().llama;
        let (data_dir, logs_dir) = shared.read(|l| (l.data_dir.clone(), l.logs_dir()));
        let status = Arc::new(RwLock::new(ModelStatus {
            state: ModelState::Starting,
            message: None,
            external: false,
            pid: None,
            port: cfg.port,
            model_path: cfg.model_path.clone(),
            recent_restarts: 0,
            since: crate::db::now(),
            progress: None,
        }));
        let (tx, rx) = mpsc::channel(16);
        let activity = Arc::new(Activity {
            holds: AtomicUsize::new(0),
            last_used: std::sync::Mutex::new(Instant::now()),
        });
        let sup = Supervisor {
            activity: activity.clone(),
            cfg,
            shared,
            hardware,
            data_dir,
            logs_dir,
            status: status.clone(),
            rx,
            http: reqwest::Client::new(),
        };
        tokio::spawn(sup.run());
        Self {
            status,
            tx,
            activity,
        }
    }

    /// Notes that the model is wanted (a chat was opened or a prompt sent),
    /// and loads it if it was unloaded.
    pub fn wake(&self) {
        self.activity.touch();
        let idle = {
            let mut st = self.status.write().unwrap_or_else(|e| e.into_inner());
            let idle = st.state == ModelState::Idle;
            if idle {
                st.state = ModelState::Starting;
                st.since = crate::db::now();
                st.message = Some("Loading model".into());
            }
            idle
        };
        if idle {
            let _ = self.tx.try_send(Cmd::Wake);
        }
    }

    /// Keeps the model loaded until the returned value is dropped.
    pub fn hold(&self) -> ModelHold {
        self.activity.holds.fetch_add(1, Ordering::SeqCst);
        self.activity.touch();
        ModelHold(self.activity.clone())
    }

    /// Whether a task is using the model now.
    pub fn in_use(&self) -> bool {
        self.activity.holds.load(Ordering::SeqCst) > 0
    }

    /// Unloads the model now; it loads again when a chat needs it.
    pub async fn unload(&self) {
        let _ = self.tx.send(Cmd::Unload).await;
    }

    /// After a settings change: restarts a loaded model so the change takes
    /// effect. An unloaded one stays unloaded and uses it when it next loads.
    pub async fn settings_changed(&self) {
        if self.status().state != ModelState::Idle {
            self.restart().await;
        }
    }

    pub fn status(&self) -> ModelStatus {
        self.status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn is_ready(&self) -> bool {
        self.status.read().unwrap_or_else(|e| e.into_inner()).state == ModelState::Ready
    }

    pub fn base_url(&self) -> String {
        let port = self.status.read().unwrap_or_else(|e| e.into_inner()).port;
        format!("http://127.0.0.1:{port}")
    }

    pub async fn restart(&self) {
        let _ = self.tx.send(Cmd::Restart).await;
    }

    /// Restarts llama-server on another model file (already saved to the
    /// config). Marks the model as loading right away, so a task waiting for
    /// it never talks to the old one.
    pub async fn switch_to(&self, path: &str) {
        {
            let mut st = self.status.write().unwrap_or_else(|e| e.into_inner());
            st.state = ModelState::Starting;
            st.since = crate::db::now();
            st.model_path = path.to_string();
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            st.message = Some(format!("Switching to {name}"));
        }
        self.restart().await;
    }

    /// Stops the child process (if Relay launched it) and waits for it to exit.
    pub async fn shutdown(&self) {
        let (ack_tx, ack_rx) = oneshot::channel();
        if self.tx.send(Cmd::Shutdown(ack_tx)).await.is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(15), ack_rx).await;
        }
    }
}

struct Supervisor {
    activity: Arc<Activity>,
    /// Snapshot taken at the start of each lifecycle.
    cfg: LlamaConfig,
    shared: Arc<SharedConfig>,
    hardware: Hardware,
    data_dir: PathBuf,
    logs_dir: PathBuf,
    status: Arc<RwLock<ModelStatus>>,
    rx: mpsc::Receiver<Cmd>,
    http: reqwest::Client,
}

enum Outcome {
    /// Child exited / failed to come up; counts toward the restart budget.
    Failed(String),
    Restart,
    /// Unloaded because it wasn't used, or when asked.
    Unloaded,
    Shutdown(oneshot::Sender<()>),
}

impl Supervisor {
    /// How long the model may sit unused before it's unloaded; None keeps it.
    fn idle_after(&self) -> Option<Duration> {
        match self.shared.get().llama.idle_unload_minutes {
            0 => None,
            m => Some(Duration::from_secs(u64::from(m) * 60)),
        }
    }

    /// Waits, unloaded, until the model is needed. None means shut down.
    async fn wait_idle(&mut self) -> Option<()> {
        {
            let mut st = self.status.write().unwrap_or_else(|e| e.into_inner());
            if st.state != ModelState::Idle {
                st.since = crate::db::now();
            }
            st.state = ModelState::Idle;
            st.message = None;
            st.pid = None;
            st.external = false;
            st.progress = None;
        }
        tracing::info!("model: unloaded; it loads again when a chat needs it");
        loop {
            match self.rx.recv().await {
                Some(Cmd::Wake) | Some(Cmd::Restart) => return Some(()),
                Some(Cmd::Unload) => {}
                Some(Cmd::Shutdown(ack)) => {
                    let _ = ack.send(());
                    return None;
                }
                None => return None,
            }
        }
    }

    fn set(&self, state: ModelState, message: Option<String>, pid: Option<u32>, external: bool) {
        let mut s = self.status.write().unwrap_or_else(|e| e.into_inner());
        if s.state != state {
            s.since = crate::db::now();
        }
        s.state = state;
        s.message = message;
        s.pid = pid;
        s.external = external;
        s.progress = None;
        match state {
            ModelState::Error => {
                tracing::error!("model: {}", s.message.clone().unwrap_or_default())
            }
            _ => tracing::info!(
                "model: {:?} {}",
                state,
                s.message.clone().unwrap_or_default()
            ),
        }
    }

    fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}/health", self.cfg.port)
    }

    async fn health_ok(&self) -> bool {
        matches!(
            self.http.get(self.health_url()).timeout(Duration::from_secs(3)).send().await,
            Ok(r) if r.status().is_success()
        )
    }

    async fn run(mut self) {
        let mut failures: VecDeque<Instant> = VecDeque::new();
        // With unloading on, nothing loads until a chat needs it.
        if self.idle_after().is_some() && self.wait_idle().await.is_none() {
            return;
        }
        loop {
            let outcome = self.run_once().await;
            match outcome {
                Outcome::Unloaded => {
                    failures.clear();
                    self.status
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .recent_restarts = 0;
                    if self.wait_idle().await.is_none() {
                        return;
                    }
                }
                Outcome::Shutdown(ack) => {
                    let _ = ack.send(());
                    return;
                }
                Outcome::Restart => {
                    failures.clear();
                    self.status
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .recent_restarts = 0;
                }
                Outcome::Failed(msg) => {
                    let now = Instant::now();
                    while failures
                        .front()
                        .is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW)
                    {
                        failures.pop_front();
                    }
                    if failures.len() >= MAX_RESTARTS {
                        self.set(
                            ModelState::Error,
                            Some(format!(
                                "{msg}. Gave up after {MAX_RESTARTS} restarts in 5 minutes; use restart to try again."
                            )),
                            None,
                            false,
                        );
                        self.status
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .recent_restarts = failures.len();
                        // Wait for an explicit restart (or shutdown).
                        loop {
                            match self.rx.recv().await {
                                Some(Cmd::Restart) => break,
                                Some(Cmd::Wake) => {}
                                Some(Cmd::Unload) => {
                                    if self.wait_idle().await.is_none() {
                                        return;
                                    }
                                    break;
                                }
                                Some(Cmd::Shutdown(ack)) => {
                                    let _ = ack.send(());
                                    return;
                                }
                                None => return,
                            }
                        }
                        failures.clear();
                        self.status
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .recent_restarts = 0;
                    } else {
                        failures.push_back(now);
                        self.status
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .recent_restarts = failures.len();
                        tracing::warn!(
                            "llama-server failed ({msg}); restarting ({}/{MAX_RESTARTS})",
                            failures.len()
                        );
                        self.set(
                            ModelState::Starting,
                            Some(format!("Restarting after failure: {msg}")),
                            None,
                            false,
                        );
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }

    /// One lifecycle: use an existing server or launch one, wait until ready,
    /// then supervise until it exits or a command arrives.
    async fn run_once(&mut self) -> Outcome {
        self.cfg = self.shared.get().llama;
        {
            let mut st = self.status.write().unwrap_or_else(|e| e.into_inner());
            st.port = self.cfg.port;
            st.model_path = self.cfg.model_path.clone();
        }
        self.set(ModelState::Starting, None, None, false);

        if self.health_ok().await {
            self.set(
                ModelState::Ready,
                Some("Using llama-server already running on this port".into()),
                None,
                true,
            );
            return self.watch_external().await;
        }

        if let Err(e) = self.ensure_setup().await {
            self.set(ModelState::Error, Some(format!("{e:#}")), None, false);
            return self.wait_command().await;
        }
        self.status
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .model_path = self.cfg.model_path.clone();

        let mut child = match self.spawn().await {
            Ok(c) => c,
            Err(e) => {
                self.set(ModelState::Error, Some(format!("{e:#}")), None, false);
                // Configuration problems won't fix themselves; wait for a command.
                return self.wait_command().await;
            }
        };
        let pid = child.id();
        self.set(
            ModelState::Starting,
            Some("Loading model".into()),
            pid,
            false,
        );

        // Wait for /health while watching the child and commands.
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(self.cfg.health_timeout_seconds);
        loop {
            tokio::select! {
                st = child.wait() => {
                    let code = st.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                    return Outcome::Failed(format!("llama-server exited during startup ({code}); see logs/llama-server.log"));
                }
                cmd = self.rx.recv() => {
                    if matches!(cmd, Some(Cmd::Wake)) {
                        continue;
                    }
                    kill(&mut child).await;
                    return cmd_outcome(cmd);
                }
                _ = tokio::time::sleep_until(deadline) => {
                    kill(&mut child).await;
                    return Outcome::Failed(format!(
                        "llama-server did not become healthy within {}s",
                        self.cfg.health_timeout_seconds
                    ));
                }
                _ = tokio::time::sleep(Duration::from_millis(1000)) => {
                    if self.health_ok().await {
                        break;
                    }
                }
            }
        }
        self.set(ModelState::Ready, None, pid, false);
        self.activity.touch();

        loop {
            tokio::select! {
                st = child.wait() => {
                    let code = st.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                    return Outcome::Failed(format!("llama-server exited unexpectedly ({code})"));
                }
                cmd = self.rx.recv() => {
                    if matches!(cmd, Some(Cmd::Wake)) {
                        continue;
                    }
                    kill(&mut child).await;
                    return cmd_outcome(cmd);
                }
                _ = tokio::time::sleep(IDLE_CHECK) => {
                    if let Some(after) = self.idle_after()
                        && self.activity.unused_for(after)
                    {
                        tracing::info!("model unused for {} min; unloading", after.as_secs() / 60);
                        kill(&mut child).await;
                        return Outcome::Unloaded;
                    }
                }
            }
        }
    }

    /// After an error that won't fix itself: waits for a command.
    async fn wait_command(&mut self) -> Outcome {
        loop {
            match self.rx.recv().await {
                Some(Cmd::Wake) => {}
                cmd => return cmd_outcome(cmd),
            }
        }
    }

    /// For a server Relay did not launch: poll health; report when it goes away.
    async fn watch_external(&mut self) -> Outcome {
        let mut misses = 0;
        loop {
            tokio::select! {
                cmd = self.rx.recv() => match cmd {
                    // Relay didn't start this server, so it can't unload it.
                    Some(Cmd::Wake) | Some(Cmd::Unload) => {}
                    cmd => return cmd_outcome(cmd),
                },
                _ = tokio::time::sleep(Duration::from_secs(10)) => {
                    if self.health_ok().await {
                        misses = 0;
                    } else {
                        misses += 1;
                        if misses >= 3 {
                            return Outcome::Failed("external llama-server stopped responding".into());
                        }
                    }
                }
            }
        }
    }

    /// First-run setup: find or download llama-server and pick a model when
    /// they aren't configured (only if `auto_setup` is on). Saves the result
    /// to config.toml.
    async fn ensure_setup(&mut self) -> Result<()> {
        if !self.cfg.auto_setup {
            return Ok(());
        }
        let exe = self.cfg.executable();
        let broken = exe.parent().is_some_and(setup::has_foreign_dlls);
        if broken {
            tracing::warn!(
                "{} has DLLs for another CPU architecture; downloading llama.cpp again",
                exe.display()
            );
        }
        let exe_ok = !self.cfg.llama_server_path.trim().is_empty() && exe.is_file() && !broken;
        if !exe_ok {
            let found = match setup::find_llama_server(&self.data_dir) {
                Some(p) => p,
                None => {
                    let status = self.status.clone();
                    let progress = move |msg: String, frac: Option<f32>| {
                        let mut s = status.write().unwrap_or_else(|e| e.into_inner());
                        s.message = Some(match frac {
                            Some(f) if f < 1.0 => format!("{msg} ({:.0}%)", f * 100.0),
                            _ => msg,
                        });
                        s.progress = frac;
                    };
                    self.set(
                        ModelState::Starting,
                        Some("Setting up llama.cpp".into()),
                        None,
                        false,
                    );
                    tracing::info!("llama-server not found; downloading llama.cpp");
                    setup::download_llama_server(&self.data_dir, &self.hardware, &progress)
                        .await
                        .map_err(|e| anyhow!("could not download llama.cpp automatically ({e:#}). Install it and set llama_server_path in config.toml"))?
                }
            };
            let path = found.to_string_lossy().into_owned();
            tracing::info!("using llama-server at {path}");
            self.cfg = self
                .shared
                .update(|c| c.llama.llama_server_path = path.clone())
                .map_err(|e| anyhow!("{e}"))?
                .llama;
        }
        let model_ok =
            !self.cfg.model_path.trim().is_empty() && PathBuf::from(&self.cfg.model_path).is_file();
        if !model_ok {
            let dirs = self.shared.read(|l| l.model_dirs());
            let models = tokio::task::spawn_blocking(move || setup::discover_models(&dirs)).await?;
            let Some(pick) = setup::pick_model(&models, self.hardware.total_vram_mb()) else {
                let data = self.data_dir.join("models");
                bail!(
                    "no model yet. Open Settings, then Download models to get one (or copy a .gguf file into {}), then tap Restart model",
                    data.display()
                );
            };
            tracing::info!("selected model {}", pick.path);
            self.cfg = self
                .shared
                .update(|c| c.llama.model_path = pick.path.clone())
                .map_err(|e| anyhow!("{e}"))?
                .llama;
        }
        Ok(())
    }

    async fn spawn(&self) -> Result<Child> {
        let exe = self.cfg.executable();
        if self.cfg.llama_server_path.trim().is_empty() {
            bail!(
                "llama-server is not set up: set llama_server_path in config.toml, or turn auto_setup on"
            );
        }
        if !exe.is_file() {
            bail!("llama-server not found at {}", exe.display());
        }
        if self.cfg.model_path.trim().is_empty() {
            bail!("no model selected: choose one in the app (Settings → Model)");
        }
        if !PathBuf::from(&self.cfg.model_path).is_file() {
            bail!("model file not found at {}", self.cfg.model_path);
        }

        let mut args: Vec<String> = vec![
            "-m".into(),
            self.cfg.model_path.clone(),
            "--port".into(),
            self.cfg.port.to_string(),
        ];
        args.extend(sanitize_args(&self.cfg.server_args()));
        // A vision file next to the model lets it read pictures.
        if !args
            .iter()
            .any(|a| a.starts_with("--mmproj") || a == "--no-mmproj" || a == "-mm")
            && let Some(p) = crate::images::mmproj_for(Path::new(&self.cfg.model_path))
        {
            tracing::info!("vision file: {}", p.display());
            args.push("--mmproj".into());
            args.push(p.to_string_lossy().into_owned());
        }

        tokio::fs::create_dir_all(&self.logs_dir).await?;
        let log_path = self.logs_dir.join("llama-server.log");
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;
        let banner = format!(
            "\n===== {} starting {} {} =====\n",
            crate::db::now(),
            exe.display(),
            args.join(" ")
        );
        file.write_all(banner.as_bytes()).await?;
        let log = Arc::new(Mutex::new(file));

        tracing::info!("launching {} {}", exe.display(), args.join(" "));
        let mut cmd = Command::new(&exe);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = exe.parent() {
            // So CUDA/ggml DLLs next to the exe are found.
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            // CREATE_NEW_PROCESS_GROUP: keep Ctrl+C in Relay's console from
            // reaching llama-server; Relay stops it itself.
            // CREATE_NO_WINDOW: no extra console window.
            cmd.creation_flags(0x0000_0200 | 0x0800_0000);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to launch {}: {e}", exe.display()))?;
        crate::process_guard::tie_to_relay(&child);
        if let Some(out) = child.stdout.take() {
            tokio::spawn(pipe_to_log(out, log.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pipe_to_log(err, log));
        }
        Ok(child)
    }
}

fn cmd_outcome(cmd: Option<Cmd>) -> Outcome {
    match cmd {
        Some(Cmd::Restart) | Some(Cmd::Wake) => Outcome::Restart,
        Some(Cmd::Unload) => Outcome::Unloaded,
        Some(Cmd::Shutdown(ack)) => Outcome::Shutdown(ack),
        None => Outcome::Shutdown(oneshot::channel().0),
    }
}

/// Relay owns -m/--model, --port and --host; drop them from user args and
/// always bind to localhost.
fn sanitize_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if matches!(a, "-m" | "--model" | "--port" | "--host") {
            i += 2;
            continue;
        }
        if a.starts_with("--port=") || a.starts_with("--host=") || a.starts_with("--model=") {
            i += 1;
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out.push("--host".into());
    out.push("127.0.0.1".into());
    out
}

async fn kill(child: &mut Child) {
    tracing::info!("stopping llama-server");
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
}

async fn pipe_to_log<R: AsyncRead + Unpin>(r: R, log: Arc<Mutex<tokio::fs::File>>) {
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut f = log.lock().await;
        let _ = f.write_all(line.as_bytes()).await;
        let _ = f.write_all(b"\n").await;
    }
    let _ = log.lock().await.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_forces_localhost() {
        let args: Vec<String> = ["--jinja", "--host", "0.0.0.0", "--port", "9", "-c", "10"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = sanitize_args(&args);
        assert_eq!(out, ["--jinja", "-c", "10", "--host", "127.0.0.1"]);
    }
}
