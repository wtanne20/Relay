//! Zero-configuration setup: detect the GPU and Tailscale, find or download
//! llama.cpp, pick a GGUF model, and produce the phone pairing QR code.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::config::{LLAMA_SERVER_EXE, home_dir};

// ---------- GPU ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub vram_mb: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Hardware {
    pub gpus: Vec<GpuInfo>,
    /// Highest CUDA version the NVIDIA driver supports, e.g. (12, 8).
    pub cuda_driver: Option<(u32, u32)>,
}

impl Hardware {
    pub fn total_vram_mb(&self) -> u64 {
        self.gpus.iter().map(|g| g.vram_mb).sum()
    }
}

/// Uses nvidia-smi when available. Missing tools simply mean "no NVIDIA GPU".
pub async fn detect_hardware() -> Hardware {
    let mut hw = Hardware::default();
    if let Some(out) = run_capture(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .await
    {
        hw.gpus = parse_gpu_csv(&out);
    }
    if !hw.gpus.is_empty()
        && let Some(out) = run_capture("nvidia-smi", &[]).await
    {
        hw.cuda_driver = parse_cuda_version(&out);
    }
    hw
}

pub fn parse_gpu_csv(s: &str) -> Vec<GpuInfo> {
    s.lines()
        .filter_map(|l| {
            let (name, mem) = l.rsplit_once(',')?;
            Some(GpuInfo {
                name: name.trim().to_string(),
                vram_mb: mem.trim().parse().ok()?,
            })
        })
        .collect()
}

/// Reads the CUDA version the driver supports from nvidia-smi's header. Older
/// drivers print "CUDA Version: 13.0", newer ones "CUDA UMD Version: 13.3".
pub fn parse_cuda_version(s: &str) -> Option<(u32, u32)> {
    s.match_indices("CUDA").find_map(|(i, _)| {
        let after = &s[i + "CUDA".len()..];
        let (label, rest) = after.split_once(':')?;
        if !label.trim_end().ends_with("Version") || label.contains('|') || label.len() > 24 {
            return None;
        }
        let ver: String = rest
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let (a, b) = ver.split_once('.')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    })
}

async fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ---------- Tailscale ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Tailscale {
    /// MagicDNS name without the trailing dot, e.g. my-pc.tail1234.ts.net
    pub dns_name: Option<String>,
    pub ip: Option<String>,
}

pub async fn detect_tailscale() -> Option<Tailscale> {
    let mut candidates = vec!["tailscale".to_string()];
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                candidates.push(
                    PathBuf::from(pf)
                        .join("Tailscale")
                        .join("tailscale.exe")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    for c in candidates {
        if let Some(out) = run_capture(&c, &["status", "--json"]).await {
            return parse_tailscale_status(&out);
        }
    }
    None
}

pub fn parse_tailscale_status(json: &str) -> Option<Tailscale> {
    let v: Value = serde_json::from_str(json).ok()?;
    let me = v.get("Self")?;
    let dns_name = me
        .get("DNSName")
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches('.').to_string())
        .filter(|s| !s.is_empty());
    let ip = me
        .get("TailscaleIPs")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .find(|ip| ip.contains('.'))
                .map(str::to_string)
        });
    if dns_name.is_none() && ip.is_none() {
        return None;
    }
    Some(Tailscale { dns_name, ip })
}

// ---------- llama.cpp ----------

/// Looks for an existing llama-server: Relay's own download folder, PATH,
/// and common install locations.
pub fn find_llama_server(data_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let own = data_dir.join("llama.cpp");
    if let Ok(rd) = std::fs::read_dir(&own) {
        // Newest download first (folders are named after the release tag).
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        dirs.reverse();
        candidates.extend(dirs.into_iter().map(|d| d.join(LLAMA_SERVER_EXE)));
    }
    candidates.push(own.join(LLAMA_SERVER_EXE));
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(LLAMA_SERVER_EXE)));
    }
    if let Some(h) = home_dir() {
        candidates.push(h.join("llama.cpp").join(LLAMA_SERVER_EXE));
    }
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\llama.cpp").join(LLAMA_SERVER_EXE));
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(pf).join("llama.cpp").join(LLAMA_SERVER_EXE));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Chooses which release assets to download. Returns the main zip and, for
/// CUDA builds, the matching cudart zip.
pub fn choose_assets(names: &[String], hw: &Hardware, windows: bool) -> Result<Vec<String>> {
    if !windows {
        let cpu = names
            .iter()
            .find(|n| n.contains("bin-ubuntu-x64") && n.ends_with(".zip"))
            .ok_or_else(|| anyhow!("no Linux x64 build in the latest llama.cpp release"))?;
        return Ok(vec![cpu.clone()]);
    }
    if !hw.gpus.is_empty() {
        // CUDA minor-version compatibility: a driver for CUDA X.y runs any X.* build.
        // With an unknown driver version, stay on CUDA 12, which every current driver runs.
        let max_major = hw.cuda_driver.map_or(12, |(major, _)| major);
        let mut cuda: Vec<((u32, u32), &String)> = names
            .iter()
            .filter(|n| n.starts_with("llama-") && n.ends_with("-x64.zip"))
            .filter_map(|n| cuda_version_in(n, "-bin-win-cuda-").map(|v| (v, n)))
            .filter(|((major, _), _)| *major <= max_major)
            .collect();
        cuda.sort();
        if let Some((ver, main)) = cuda.last() {
            let cudart = names.iter().find(|n| {
                n.starts_with("cudart-") && cuda_version_in(n, "-bin-win-cuda-") == Some(*ver)
            });
            let mut out = vec![(*main).clone()];
            if let Some(c) = cudart {
                out.push(c.clone());
            }
            return Ok(out);
        }
    }
    for kind in ["-bin-win-vulkan-x64.zip", "-bin-win-cpu-x64.zip"] {
        if let Some(n) = names
            .iter()
            .find(|n| n.starts_with("llama-") && n.ends_with(kind))
        {
            return Ok(vec![n.clone()]);
        }
    }
    bail!("no suitable Windows build found in the latest llama.cpp release")
}

fn asset_names(release: &Value) -> Vec<String> {
    release
        .get("assets")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The newest non-draft release (GitHub lists newest first) that has a build for this PC.
pub fn pick_release<'a>(
    releases: &'a [Value],
    hw: &Hardware,
    windows: bool,
) -> Result<(&'a Value, Vec<String>)> {
    releases
        .iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .find_map(|r| {
            choose_assets(&asset_names(r), hw, windows)
                .ok()
                .map(|c| (r, c))
        })
        .ok_or_else(|| anyhow!("no recent llama.cpp release has a build for this PC"))
}

fn cuda_version_in(name: &str, marker: &str) -> Option<(u32, u32)> {
    let idx = name.find(marker)?;
    let rest = &name[idx + marker.len()..];
    let ver: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let (a, b) = ver.split_once('.')?;
    Some((a.parse().ok()?, b.trim_end_matches('.').parse().ok()?))
}

/// Downloads the latest llama.cpp release suited to this PC into
/// `<data>/llama.cpp/<tag>/` and returns the llama-server path.
/// `progress(message, fraction)` is called as it goes.
pub async fn download_llama_server(
    data_dir: &Path,
    hw: &Hardware,
    progress: &(dyn Fn(String, Option<f32>) + Send + Sync),
) -> Result<PathBuf> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("relay/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()?;
    progress("Looking up the latest llama.cpp release".into(), None);
    // Not /releases/latest: llama.cpp publishes its builds (b1234 tags) as prereleases,
    // and "latest" can be a release with no binaries at all.
    let releases: Value = http
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=20")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("could not reach GitHub to download llama.cpp")?
        .error_for_status()?
        .json()
        .await?;
    let (release, chosen) = pick_release(
        releases.as_array().map(Vec::as_slice).unwrap_or_default(),
        hw,
        cfg!(windows),
    )?;
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or("latest")
        .to_string();
    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let dest = data_dir.join("llama.cpp").join(&tag);
    let downloads = data_dir.join("downloads");
    tokio::fs::create_dir_all(&dest).await?;
    tokio::fs::create_dir_all(&downloads).await?;
    for name in &chosen {
        let url = assets
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|a| a.get("browser_download_url").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("asset {name} has no download URL"))?;
        let zip = downloads.join(name);
        download_file(&http, url, &zip, &|f| {
            progress(format!("Downloading {name}"), Some(f))
        })
        .await?;
        progress(format!("Extracting {name}"), None);
        extract_zip(&zip, &dest).await?;
        let _ = tokio::fs::remove_file(&zip).await;
    }
    let exe = find_file(&dest, LLAMA_SERVER_EXE, 3).ok_or_else(|| {
        anyhow!("downloaded llama.cpp {tag} but {LLAMA_SERVER_EXE} was not in it")
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
    progress(format!("Installed llama.cpp {tag}"), Some(1.0));
    Ok(exe)
}

async fn download_file(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<()> {
    let mut resp = http.get(url).send().await?.error_for_status()?;
    let total = resp.content_length();
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut done: u64 = 0;
    let mut last_report = 0u64;
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(120), resp.chunk())
        .await
        .map_err(|_| anyhow!("download stalled"))??
    {
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if let Some(t) = total
            && t > 0
            && done - last_report > t / 100
        {
            last_report = done;
            progress(done as f32 / t as f32);
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

async fn extract_zip(zip: &Path, dest: &Path) -> Result<()> {
    let status = if cfg!(windows) {
        let script = format!(
            "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
            zip.display().to_string().replace('\'', "''"),
            dest.display().to_string().replace('\'', "''")
        );
        let mut cmd = tokio::process::Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        #[cfg(windows)]
        {
            cmd.creation_flags(0x0800_0000);
        }
        cmd.status().await
    } else {
        tokio::process::Command::new("unzip")
            .arg("-o")
            .arg("-q")
            .arg(zip)
            .arg("-d")
            .arg(dest)
            .status()
            .await
    }
    .context("could not run the zip extractor")?;
    if !status.success() {
        bail!("extracting {} failed", zip.display());
    }
    Ok(())
}

fn find_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let rd = std::fs::read_dir(dir).ok()?;
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .find_map(|d| find_file(&d, name, depth - 1))
}

// ---------- models ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelFile {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
}

/// All GGUF files under the given folders (vision projectors excluded).
/// Multi-part models are listed once, by their first part.
pub fn discover_models(dirs: &[PathBuf]) -> Vec<ModelFile> {
    let mut out: Vec<ModelFile> = Vec::new();
    for d in dirs {
        walk_ggufs(d, 6, &mut out);
    }
    out.sort_by_key(|m| m.name.to_lowercase());
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn walk_ggufs(dir: &Path, depth: usize, out: &mut Vec<ModelFile>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                walk_ggufs(&p, depth - 1, out);
            }
            continue;
        }
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let lower = name.to_lowercase();
        if !lower.ends_with(".gguf") || lower.starts_with("mmproj") {
            continue;
        }
        // Split models: keep only part 1 ("-00001-of-00003.gguf").
        if let Some(idx) = lower.find("-of-") {
            let part = &lower[..idx];
            if !part.ends_with("00001") {
                continue;
            }
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(ModelFile {
            path: p.to_string_lossy().into_owned(),
            name,
            size_bytes: size,
        });
    }
}

/// Picks the largest model that leaves ~2.5 GB of VRAM for context; with no
/// GPU information, or nothing fitting, the smallest model.
pub fn pick_model(models: &[ModelFile], vram_mb: u64) -> Option<ModelFile> {
    if models.is_empty() {
        return None;
    }
    let budget = vram_mb.saturating_sub(2560) * 1024 * 1024;
    let fitting = models
        .iter()
        .filter(|m| m.size_bytes <= budget)
        .max_by_key(|m| m.size_bytes);
    fitting
        .or_else(|| models.iter().min_by_key(|m| m.size_bytes))
        .cloned()
}

// ---------- pairing ----------

/// The address the phone should use: the Tailscale IP (works even when
/// MagicDNS is off), else the MagicDNS name, else this machine's name.
pub fn pairing_url(ts: Option<&Tailscale>, port: &str) -> String {
    let host = ts
        .and_then(|t| t.ip.clone().or_else(|| t.dns_name.clone()))
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "localhost".into());
    format!("http://{host}:{port}")
}

/// What the QR code encodes. The app parses this to fill in the connection.
pub fn pairing_payload(url: &str, token: &str) -> String {
    format!(
        "relay://pair?url={}&token={}",
        percent_encode(url),
        percent_encode(token)
    )
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A QR code drawn with Unicode half blocks for the console.
pub fn qr_console(payload: &str) -> Result<String> {
    let code = qrcode::QrCode::new(payload.as_bytes())?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

/// The pairing QR code as an SVG document.
pub fn qr_svg(payload: &str, size: u32) -> Result<String> {
    let code = qrcode::QrCode::new(payload.as_bytes())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(size, size)
        .quiet_zone(true)
        .build())
}

/// Writes pairing.html (QR code + instructions) into the data folder.
pub fn write_pairing_page(
    data_dir: &Path,
    url: &str,
    payload: &str,
    tailscale_found: bool,
) -> Result<PathBuf> {
    let svg = qr_svg(payload, 320)?;
    let warning = if tailscale_found {
        String::new()
    } else {
        "<p class=warn>Tailscale was not detected on this PC. Install it from tailscale.com and sign in on both the PC and phone, then open <b>Relay → Settings → Pair your phone</b> again.</p>".to_string()
    };
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Pair your phone with Relay</title>
<style>
body{{margin:0;font-family:system-ui,Segoe UI,sans-serif;background:#141414;color:#e6e6e6;display:flex;min-height:100vh;align-items:center;justify-content:center}}
main{{max-width:520px;padding:32px;text-align:center}}
.qr{{background:#fff;display:inline-block;padding:12px;border-radius:12px;margin:16px 0}}
code{{background:#202020;padding:2px 6px;border-radius:6px}}
ol{{text-align:left;line-height:1.6}}.warn{{color:#f0c46c}}
</style></head><body><main>
<h1>Pair your phone</h1>
<div class="qr">{svg}</div>
<ol>
<li>Make sure Tailscale is on, on both this PC and your phone.</li>
<li>Open the Relay app and tap <b>Scan QR code</b>.</li>
<li>Point the camera at this code. That's it.</li>
</ol>
<p>Server address: <code>{url}</code></p>
{warning}
<p style="color:#c2c2c2;font-size:13px">This code contains your Relay access token. Don't share it. You can see this code again any time in the Relay app: Settings, Pair your phone.</p>
</main></body></html>"#
    );
    let path = data_dir.join("pairing.html");
    std::fs::write(&path, html)?;
    Ok(path)
}

/// Opens a file or URL with the default program (best effort).
pub fn open_in_browser(path: &Path) {
    let p = path.to_string_lossy().into_owned();
    let result = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &p])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&p).spawn()
    } else {
        std::process::Command::new("xdg-open")
            .arg(&p)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    if let Err(e) = result {
        tracing::debug!("could not open {p}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const RELEASE: &[&str] = &[
        "cudart-llama-bin-win-cuda-12.4-x64.zip",
        "cudart-llama-bin-win-cuda-13.1-x64.zip",
        "llama-b7000-bin-macos-arm64.zip",
        "llama-b7000-bin-ubuntu-x64.zip",
        "llama-b7000-bin-win-cpu-x64.zip",
        "llama-b7000-bin-win-cuda-12.4-x64.zip",
        "llama-b7000-bin-win-cuda-13.1-x64.zip",
        "llama-b7000-bin-win-vulkan-x64.zip",
    ];

    fn nvidia(cuda: (u32, u32)) -> Hardware {
        Hardware {
            gpus: vec![GpuInfo {
                name: "RTX".into(),
                vram_mb: 32768,
            }],
            cuda_driver: Some(cuda),
        }
    }

    #[test]
    fn picks_cuda_matching_driver() {
        let a = choose_assets(&names(RELEASE), &nvidia((13, 2)), true).unwrap();
        assert_eq!(
            a,
            [
                "llama-b7000-bin-win-cuda-13.1-x64.zip",
                "cudart-llama-bin-win-cuda-13.1-x64.zip"
            ]
        );
        let a = choose_assets(&names(RELEASE), &nvidia((12, 8)), true).unwrap();
        assert_eq!(
            a,
            [
                "llama-b7000-bin-win-cuda-12.4-x64.zip",
                "cudart-llama-bin-win-cuda-12.4-x64.zip"
            ]
        );
    }

    #[test]
    fn falls_back_without_nvidia() {
        let a = choose_assets(&names(RELEASE), &Hardware::default(), true).unwrap();
        assert_eq!(a, ["llama-b7000-bin-win-vulkan-x64.zip"]);
        let a = choose_assets(&names(RELEASE), &Hardware::default(), false).unwrap();
        assert_eq!(a, ["llama-b7000-bin-ubuntu-x64.zip"]);
    }

    #[test]
    fn parses_nvidia_smi() {
        let gpus = parse_gpu_csv("NVIDIA GeForce RTX 5090, 32607\n");
        assert_eq!(
            gpus,
            vec![GpuInfo {
                name: "NVIDIA GeForce RTX 5090".into(),
                vram_mb: 32607
            }]
        );
        let head = "| NVIDIA-SMI 580.88   Driver Version: 580.88   CUDA Version: 13.0     |";
        assert_eq!(parse_cuda_version(head), Some((13, 0)));
        let new = "| NVIDIA-SMI 610.88   Driver Version: 610.88   CUDA UMD Version: 13.3     |";
        assert_eq!(parse_cuda_version(new), Some((13, 3)));
        assert_eq!(parse_cuda_version("no gpu here"), None);
    }

    #[test]
    fn same_major_cuda_build_is_allowed() {
        let names = names(&[
            "cudart-llama-bin-win-cuda-12.4-x64.zip",
            "cudart-llama-bin-win-cuda-13.4-x64.zip",
            "llama-b12063-bin-win-cuda-12.4-x64.zip",
            "llama-b12063-bin-win-cuda-13.4-x64.zip",
            "llama-b12063-bin-win-vulkan-x64.zip",
        ]);
        let a = choose_assets(&names, &nvidia((13, 3)), true).unwrap();
        assert_eq!(
            a,
            [
                "llama-b12063-bin-win-cuda-13.4-x64.zip",
                "cudart-llama-bin-win-cuda-13.4-x64.zip"
            ]
        );
        let mut unknown = nvidia((0, 0));
        unknown.cuda_driver = None;
        let a = choose_assets(&names, &unknown, true).unwrap();
        assert_eq!(
            a,
            [
                "llama-b12063-bin-win-cuda-12.4-x64.zip",
                "cudart-llama-bin-win-cuda-12.4-x64.zip"
            ]
        );
    }

    #[test]
    fn skips_releases_without_binaries() {
        let rel = |tag: &str, assets: &[&str]| serde_json::json!({ "tag_name": tag, "assets": assets.iter().map(|n| serde_json::json!({"name": n})).collect::<Vec<_>>() });
        let releases = vec![
            rel("v0.8.0", &["nightly-tag.txt"]),
            rel(
                "b12063",
                &[
                    "llama-b12063-bin-win-vulkan-x64.zip",
                    "llama-b12063-bin-ubuntu-x64.zip",
                ],
            ),
            rel("b12061", &["llama-b12061-bin-win-vulkan-x64.zip"]),
        ];
        let (r, chosen) = pick_release(&releases, &Hardware::default(), true).unwrap();
        assert_eq!(r["tag_name"], "b12063");
        assert_eq!(chosen, ["llama-b12063-bin-win-vulkan-x64.zip"]);
        assert!(pick_release(&releases[..1], &Hardware::default(), true).is_err());
    }

    #[test]
    fn parses_tailscale() {
        let j = r#"{"Self":{"DNSName":"my-pc.tail1234.ts.net.","TailscaleIPs":["100.101.102.103","fd7a::1"]}}"#;
        let t = parse_tailscale_status(j).unwrap();
        assert_eq!(t.dns_name.as_deref(), Some("my-pc.tail1234.ts.net"));
        assert_eq!(t.ip.as_deref(), Some("100.101.102.103"));
        assert_eq!(pairing_url(Some(&t), "8765"), "http://100.101.102.103:8765");
    }

    #[test]
    fn model_choice_fits_vram() {
        let gb = 1024 * 1024 * 1024;
        let m = |n: &str, s: u64| ModelFile {
            path: n.into(),
            name: n.into(),
            size_bytes: s * gb,
        };
        let models = vec![m("small", 5), m("q6", 22), m("huge", 40)];
        assert_eq!(pick_model(&models, 32768).unwrap().name, "q6");
        assert_eq!(pick_model(&models, 4096).unwrap().name, "small");
        assert!(pick_model(&[], 32768).is_none());
    }

    #[test]
    fn discovers_models_skipping_projectors_and_extra_parts() {
        let base = std::env::temp_dir().join(format!("relay-models-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("pub/Model-GGUF")).unwrap();
        for f in [
            "a-Q6_K.gguf",
            "mmproj-a.gguf",
            "big-00001-of-00002.gguf",
            "big-00002-of-00002.gguf",
            "notes.txt",
        ] {
            std::fs::write(base.join("pub/Model-GGUF").join(f), b"x").unwrap();
        }
        let found: Vec<String> = discover_models(std::slice::from_ref(&base))
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(found, ["a-Q6_K.gguf", "big-00001-of-00002.gguf"]);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn pairing_payload_round_trip() {
        let p = pairing_payload("http://my-pc.ts.net:8765", "abc123");
        assert_eq!(
            p,
            "relay://pair?url=http%3A%2F%2Fmy-pc.ts.net%3A8765&token=abc123"
        );
        assert!(qr_console(&p).unwrap().lines().count() > 10);
    }
}
