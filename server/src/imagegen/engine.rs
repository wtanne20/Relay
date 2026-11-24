//! sd-server (stable-diffusion.cpp) process management: find or download
//! it, launch it on 127.0.0.1, and stop it when a chat needs the GPU or no
//! picture was made for a while.
//!
//! sd-server starts without a model. Each generation request names a recipe
//! and its component files, and sd-server loads them when the job runs, so
//! "loaded" here means "a picture was made since sd-server started".

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::config::{SD_SERVER_EXE, SharedConfig};
use crate::gpu::{Gpu, Owner};
use crate::setup::{self, Hardware};

const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
const WATCH_EVERY: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
    /// Not running.
    Idle,
    /// Downloading sd-server.
    Setup,
    Starting,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub state: EngineState,
    pub message: Option<String>,
    /// 0..1 while downloading.
    pub progress: Option<f32>,
    /// True if an sd-server Relay didn't start answers on the port.
    pub external: bool,
    pub port: u16,
    /// Who has the GPU: free, chat or images.
    pub gpu: Owner,
    /// Pictures being made now.
    pub jobs: usize,
    pub exe: String,
}

pub struct Engine {
    shared: Arc<SharedConfig>,
    hardware: Hardware,
    gpu: Arc<Gpu>,
    status: RwLock<EngineStatus>,
    child: Mutex<Option<Child>>,
    /// One setup (download) at a time.
    setup: Mutex<()>,
    external: AtomicBool,
    last_used: std::sync::Mutex<Instant>,
    http: reqwest::Client,
}

impl Engine {
    pub fn new(shared: Arc<SharedConfig>, hardware: Hardware, gpu: Arc<Gpu>) -> Arc<Self> {
        let port = shared.get().images.port;
        let e = Arc::new(Self {
            shared,
            hardware,
            gpu: gpu.clone(),
            status: RwLock::new(EngineStatus {
                state: EngineState::Idle,
                message: None,
                progress: None,
                external: false,
                port,
                gpu: Owner::Free,
                jobs: 0,
                exe: String::new(),
            }),
            child: Mutex::new(None),
            setup: Mutex::new(()),
            external: AtomicBool::new(false),
            last_used: std::sync::Mutex::new(Instant::now()),
            http: reqwest::Client::new(),
        });
        let stopper = Arc::downgrade(&e);
        gpu.set_image_stop(Arc::new(move || {
            let e = stopper.clone();
            Box::pin(async move {
                if let Some(e) = e.upgrade() {
                    e.stop().await;
                }
            })
        }));
        e
    }

    /// Watches for crashes and unloads the image model when unused.
    pub fn spawn_watcher(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(WATCH_EVERY).await;
                let Some(e) = weak.upgrade() else { return };
                e.watch_once().await;
            }
        });
    }

    async fn watch_once(&self) {
        let exited = {
            let mut child = self.child.lock().await;
            match child.as_mut().map(|c| c.try_wait()) {
                Some(Ok(Some(code))) => {
                    *child = None;
                    Some(code.to_string())
                }
                _ => None,
            }
        };
        if let Some(code) = exited {
            tracing::warn!("sd-server exited ({code})");
            self.set(
                EngineState::Idle,
                Some(format!(
                    "The image model stopped ({code}); see logs/sd-server.log"
                )),
            );
            self.gpu.images_stopped();
            return;
        }
        let minutes = self.shared.get().images.idle_unload_minutes;
        let running = self.status().state == EngineState::Ready && !self.is_external();
        if running && minutes > 0 && self.gpu.image_jobs() == 0 {
            let idle = self
                .last_used
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .elapsed();
            if idle >= Duration::from_secs(u64::from(minutes) * 60) {
                tracing::info!("image model unused for {minutes} min; stopping it");
                self.gpu.release_images().await;
            }
        }
    }

    pub fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    pub fn is_external(&self) -> bool {
        self.external.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> EngineStatus {
        let mut s = self
            .status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        s.gpu = self.gpu.owner();
        s.jobs = self.gpu.image_jobs();
        s
    }

    fn set(&self, state: EngineState, message: Option<String>) {
        let mut s = self.status.write().unwrap_or_else(|e| e.into_inner());
        s.state = state;
        s.message = message;
        s.progress = None;
    }

    fn set_message(&self, message: impl Into<String>, progress: Option<f32>) {
        let mut s = self.status.write().unwrap_or_else(|e| e.into_inner());
        s.message = Some(message.into());
        s.progress = progress;
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.shared.get().images.port)
    }

    async fn health_ok(&self) -> bool {
        matches!(
            self.http
                .get(format!("{}/sdcpp/v1/recipes", self.base_url()))
                .timeout(Duration::from_secs(3))
                .send()
                .await,
            Ok(r) if r.status().is_success()
        )
    }

    /// Starts sd-server if it isn't running and waits until it answers.
    pub async fn ensure_running(&self) -> Result<()> {
        {
            let mut child = self.child.lock().await;
            if let Some(c) = child.as_mut() {
                if matches!(c.try_wait(), Ok(None)) {
                    if self.status().state == EngineState::Ready {
                        return Ok(());
                    }
                } else {
                    *child = None;
                }
            }
        }
        if self.health_ok().await {
            let ours = self.child.lock().await.is_some();
            self.external.store(!ours, Ordering::SeqCst);
            self.status
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .external = !ours;
            self.set(
                EngineState::Ready,
                (!ours).then(|| "Using sd-server already running on this port".into()),
            );
            return Ok(());
        }
        self.external.store(false, Ordering::SeqCst);
        self.status
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .external = false;
        let exe = match self.find_or_download().await {
            Ok(p) => p,
            Err(e) => {
                self.set(EngineState::Error, Some(format!("{e:#}")));
                return Err(e);
            }
        };
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            drop(guard);
            return self.wait_healthy().await;
        }
        self.set(
            EngineState::Starting,
            Some("Starting the image engine".into()),
        );
        let child = match self.spawn(&exe).await {
            Ok(c) => c,
            Err(e) => {
                self.set(EngineState::Error, Some(format!("{e:#}")));
                return Err(e);
            }
        };
        *guard = Some(child);
        drop(guard);
        self.wait_healthy().await
    }

    async fn wait_healthy(&self) -> Result<()> {
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        loop {
            if self.health_ok().await {
                self.set(EngineState::Ready, None);
                self.touch();
                return Ok(());
            }
            {
                let mut child = self.child.lock().await;
                match child.as_mut().map(|c| c.try_wait()) {
                    Some(Ok(None)) => {}
                    Some(Ok(Some(code))) => {
                        *child = None;
                        let msg = format!(
                            "sd-server exited while starting ({code}); see logs/sd-server.log"
                        );
                        self.set(EngineState::Error, Some(msg.clone()));
                        bail!(msg);
                    }
                    _ => {
                        self.set(EngineState::Idle, None);
                        bail!("the image engine was stopped");
                    }
                }
            }
            if Instant::now() > deadline {
                self.stop().await;
                let msg = "sd-server did not start in time; see logs/sd-server.log".to_string();
                self.set(EngineState::Error, Some(msg.clone()));
                bail!(msg);
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    /// Stops sd-server (if Relay started it), freeing the GPU.
    pub async fn stop(&self) {
        let child = self.child.lock().await.take();
        if let Some(mut c) = child {
            tracing::info!("stopping sd-server");
            let _ = c.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(10), c.wait()).await;
        }
        if !self.is_external() {
            let st = self.status().state;
            if st != EngineState::Error && st != EngineState::Setup {
                self.set(EngineState::Idle, None);
            }
        }
        self.gpu.images_stopped();
    }

    /// The configured sd-server, one Relay downloaded before, or a fresh
    /// download. Saves the path to config.toml.
    async fn find_or_download(&self) -> Result<PathBuf> {
        let _one = self.setup.lock().await;
        let cfg = self.shared.get().images;
        let configured = cfg.sd_server_path.trim();
        if !configured.is_empty() {
            let p = PathBuf::from(configured);
            let p = if p.is_dir() { p.join(SD_SERVER_EXE) } else { p };
            if p.is_file() {
                self.status.write().unwrap_or_else(|e| e.into_inner()).exe =
                    p.to_string_lossy().into_owned();
                return Ok(p);
            }
            bail!("sd-server not found at {}", p.display());
        }
        let data_dir = self.shared.read(|l| l.data_dir.clone());
        let exe = match find_sd_server(&data_dir) {
            Some(p) => p,
            None => {
                self.set(
                    EngineState::Setup,
                    Some("Downloading the image engine".into()),
                );
                let progress = |msg: String, frac: Option<f32>| {
                    let msg = match frac {
                        Some(f) if f < 1.0 => format!("{msg} ({:.0}%)", f * 100.0),
                        _ => msg,
                    };
                    self.set_message(msg, frac);
                };
                download_sd_server(&data_dir, &self.hardware, &progress)
                    .await
                    .map_err(|e| {
                        anyhow!(
                            "could not download stable-diffusion.cpp ({e:#}). Download sd-server from github.com/leejet/stable-diffusion.cpp/releases and set its path in Image models"
                        )
                    })?
            }
        };
        let path = exe.to_string_lossy().into_owned();
        self.shared
            .update(|c| c.images.sd_server_path = path.clone())
            .map_err(|e| anyhow!("{e}"))?;
        self.status.write().unwrap_or_else(|e| e.into_inner()).exe = path;
        Ok(exe)
    }

    async fn spawn(&self, exe: &Path) -> Result<Child> {
        let (cfg, roots, logs) = self
            .shared
            .read(|l| (l.config.images.clone(), l.image_model_dirs(), l.logs_dir()));
        for r in roots.iter().take(1) {
            for cat in MODEL_CATEGORIES {
                let _ = std::fs::create_dir_all(r.join(cat));
            }
        }
        let mut args: Vec<String> = vec![
            "--listen-ip".into(),
            "127.0.0.1".into(),
            "--listen-port".into(),
            cfg.port.to_string(),
        ];
        for r in roots.iter().filter(|r| r.is_dir()) {
            args.push("--model-dir".into());
            args.push(r.to_string_lossy().into_owned());
        }
        if cfg.offload_to_cpu {
            args.push("--offload-to-cpu".into());
        }
        args.extend(sanitize_args(&cfg.extra_args));

        tokio::fs::create_dir_all(&logs).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs.join("sd-server.log"))
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
        let mut cmd = Command::new(exe);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // sd-server writes dgl/ and output/ under its working folder.
        let work = self.shared.read(|l| l.data_dir.join("image-engine"));
        let _ = std::fs::create_dir_all(&work);
        cmd.current_dir(&work);
        if let Some(dir) = exe.parent() {
            // So the CUDA/ggml DLLs next to the exe are found.
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut dirs = vec![dir.to_path_buf()];
            dirs.extend(std::env::split_paths(&path));
            if let Ok(joined) = std::env::join_paths(dirs) {
                cmd.env("PATH", joined);
            }
        }
        #[cfg(windows)]
        {
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

    // ---------- sd-server API ----------

    pub async fn get_json(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base_url()))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("could not reach the image engine")?;
        read_json(resp).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base_url()))
            .json(body)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .context("could not reach the image engine")?;
        read_json(resp).await
    }
}

async fn read_json(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v
            .pointer("/error/message")
            .or_else(|| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| text.chars().take(300).collect());
        bail!("the image engine said {}: {msg}", status.as_u16());
    }
    Ok(v)
}

/// Model folders created in Relay's own image model folder, so it's clear
/// where files go. Same layout as ComfyUI's `models` folder.
pub const MODEL_CATEGORIES: &[&str] = &[
    "checkpoints",
    "diffusion_models",
    "text_encoders",
    "vae",
    "loras",
];

/// Relay owns --listen-ip/--listen-port/--model-dir; drop them from user args.
fn sanitize_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if matches!(a, "--listen-ip" | "--listen-port" | "--model-dir") {
            i += 2;
            continue;
        }
        if a.starts_with("--listen-ip=") || a.starts_with("--listen-port=") {
            i += 1;
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out
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

// ---------- download ----------

/// An sd-server Relay downloaded before (newest first).
pub fn find_sd_server(data_dir: &Path) -> Option<PathBuf> {
    let own = data_dir.join("stable-diffusion.cpp");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&own)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs.into_iter()
        .filter(|d| !setup::has_foreign_dlls(d))
        .find_map(|d| setup::find_file(&d, SD_SERVER_EXE, 3))
}

/// Picks the release assets for this PC: the CUDA 12 build and its runtime
/// on NVIDIA, else Vulkan, else the CPU build.
pub fn choose_sd_assets(names: &[String], hw: &Hardware, windows: bool) -> Result<Vec<String>> {
    let find = |pred: &dyn Fn(&str) -> bool| names.iter().find(|n| pred(n)).cloned();
    if !windows {
        return find(&|n: &str| {
            n.starts_with("sd-")
                && n.contains("-bin-Linux-")
                && n.ends_with(".zip")
                && !["rocm", "vulkan", "cuda", "musa", "sycl"]
                    .iter()
                    .any(|k| n.to_ascii_lowercase().contains(k))
        })
        .map(|n| vec![n])
        .ok_or_else(|| anyhow!("no Linux build in the latest stable-diffusion.cpp release"));
    }
    let nvidia_ok = !hw.gpus.is_empty() && hw.cuda_driver.is_none_or(|(major, _)| major >= 12);
    if nvidia_ok
        && let Some(main) =
            find(&|n: &str| n.starts_with("sd-") && n.ends_with("-bin-win-cuda12-x64.zip"))
    {
        let mut out = vec![main];
        if let Some(rt) = find(&|n: &str| {
            n.starts_with("cudart-") && n.contains("cu12") && n.ends_with("-x64.zip")
        }) {
            out.push(rt);
        }
        return Ok(out);
    }
    for kind in [
        "-bin-win-vulkan-x64.zip",
        "-bin-win-avx2-x64.zip",
        "-bin-win-cpu-x64.zip",
    ] {
        if let Some(n) = find(&|n: &str| n.starts_with("sd-") && n.ends_with(kind)) {
            return Ok(vec![n]);
        }
    }
    bail!("no suitable Windows build in the latest stable-diffusion.cpp release")
}

/// Downloads the newest stable-diffusion.cpp release for this PC into
/// `<data>/stable-diffusion.cpp/<tag>-x64/` and returns sd-server's path.
pub async fn download_sd_server(
    data_dir: &Path,
    hw: &Hardware,
    progress: &(dyn Fn(String, Option<f32>) + Send + Sync),
) -> Result<PathBuf> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("relay/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()?;
    progress(
        "Looking up the latest stable-diffusion.cpp release".into(),
        None,
    );
    let releases: Value = http
        .get("https://api.github.com/repos/leejet/stable-diffusion.cpp/releases?per_page=10")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("could not reach GitHub")?
        .error_for_status()?
        .json()
        .await?;
    let empty = Vec::new();
    let (release, chosen) = releases
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .find_map(|r| {
            let names: Vec<String> = r
                .get("assets")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            choose_sd_assets(&names, hw, cfg!(windows))
                .ok()
                .map(|c| (r, c))
        })
        .ok_or_else(|| anyhow!("no recent stable-diffusion.cpp release has a build for this PC"))?;
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or("latest")
        .to_string();
    let dest = data_dir
        .join("stable-diffusion.cpp")
        .join(format!("{tag}-x64"));
    let downloads = data_dir.join("downloads");
    tokio::fs::create_dir_all(&dest).await?;
    tokio::fs::create_dir_all(&downloads).await?;
    for name in &chosen {
        let url = release
            .get("assets")
            .and_then(Value::as_array)
            .and_then(|a| {
                a.iter()
                    .find(|x| x.get("name").and_then(Value::as_str) == Some(name))
            })
            .and_then(|a| a.get("browser_download_url").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("asset {name} has no download URL"))?;
        let zip = downloads.join(name);
        setup::download_file(&http, url, &zip, &|f| {
            progress(format!("Downloading {name}"), Some(f))
        })
        .await?;
        progress(format!("Extracting {name}"), None);
        setup::extract_zip(&zip, &dest).await?;
        let _ = tokio::fs::remove_file(&zip).await;
    }
    let exe = setup::find_file(&dest, SD_SERVER_EXE, 3).ok_or_else(|| {
        anyhow!("downloaded stable-diffusion.cpp {tag} but {SD_SERVER_EXE} was not in it")
    })?;
    // CUDA runtime DLLs must sit next to the exe.
    if let Some(exe_dir) = exe.parent()
        && exe_dir != dest
        && let Ok(rd) = std::fs::read_dir(&dest)
    {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")) {
                let _ = std::fs::copy(&p, exe_dir.join(e.file_name()));
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
    }
    progress(format!("Installed stable-diffusion.cpp {tag}"), Some(1.0));
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::GpuInfo;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const RELEASE: &[&str] = &[
        "cudart-sd-bin-win-cu12-x64.zip",
        "sd-master-ff4fdbb-bin-win-avx2-x64.zip",
        "sd-master-ff4fdbb-bin-win-cuda12-x64.zip",
        "sd-master-ff4fdbb-bin-win-vulkan-x64.zip",
        "sd-master-ff4fdbb-bin-win-rocm-6.4.2-x64.zip",
        "sd-master-ff4fdbb-bin-Linux-Ubuntu-24.04-x86_64.zip",
        "sd-master-ff4fdbb-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip",
    ];

    #[test]
    fn picks_cuda_with_runtime_on_nvidia() {
        let hw = Hardware {
            gpus: vec![GpuInfo {
                name: "RTX".into(),
                vram_mb: 16000,
            }],
            cuda_driver: Some((12, 8)),
        };
        assert_eq!(
            choose_sd_assets(&names(RELEASE), &hw, true).unwrap(),
            names(&[
                "sd-master-ff4fdbb-bin-win-cuda12-x64.zip",
                "cudart-sd-bin-win-cu12-x64.zip"
            ])
        );
    }

    #[test]
    fn falls_back_to_vulkan_and_linux_cpu() {
        let hw = Hardware::default();
        assert_eq!(
            choose_sd_assets(&names(RELEASE), &hw, true).unwrap(),
            names(&["sd-master-ff4fdbb-bin-win-vulkan-x64.zip"])
        );
        assert_eq!(
            choose_sd_assets(&names(RELEASE), &hw, false).unwrap(),
            names(&["sd-master-ff4fdbb-bin-Linux-Ubuntu-24.04-x86_64.zip"])
        );
    }

    #[test]
    fn sanitize_keeps_relay_in_charge_of_the_port() {
        let a = names(&[
            "--listen-port",
            "1",
            "--offload-to-cpu",
            "--listen-ip=0.0.0.0",
        ]);
        assert_eq!(sanitize_args(&a), names(&["--offload-to-cpu"]));
    }
}
