//! Update checks against Relay's GitHub releases, and installing a new
//! version. Every download's SHA-256 is checked against the one GitHub
//! publishes before it is used.
//!
//! - Windows: the installer is downloaded and run (it closes and reopens
//!   Relay).
//! - Linux AppImage: the new AppImage is downloaded next to the old one and
//!   renamed over it, then the desktop app restarts into it.
//! - Linux arm64 (Proot): the new tarball is downloaded, its `relay-desktop`
//!   unpacked and renamed over the running binary, then the app restarts.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const REPO: &str = "wtanne20/Relay";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Answers are reused for this long, so opening the app doesn't ask GitHub
/// every time.
const CACHE: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Hex SHA-256, as GitHub reports it.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub published_at: String,
    /// RelaySetup-<version>.exe
    pub windows: Option<Asset>,
    /// Relay.apk
    pub android: Option<Asset>,
    /// Relay-Dev.apk
    pub android_dev: Option<Asset>,
    /// Relay-<version>-x86_64.AppImage
    pub appimage: Option<Asset>,
    /// Relay-<version>-arm64-proot.tar.gz
    pub proot: Option<Asset>,
}

/// How this copy of Relay was installed, which decides how it updates.
#[derive(Debug, Clone, PartialEq)]
pub enum Package {
    /// Installed with RelaySetup.exe.
    WindowsInstaller,
    /// This AppImage file; an update replaces it.
    AppImage(PathBuf),
    /// The `relay-desktop` binary from the Proot tarball; an update replaces it.
    ProotBinary(PathBuf),
}

impl Package {
    /// The release file this package updates from.
    pub fn asset<'a>(&self, r: &'a Release) -> Option<&'a Asset> {
        match self {
            Package::WindowsInstaller => r.windows.as_ref(),
            Package::AppImage(_) => r.appimage.as_ref(),
            Package::ProotBinary(_) => r.proot.as_ref(),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Package::WindowsInstaller => "Windows installer",
            Package::AppImage(_) => "AppImage",
            Package::ProotBinary(_) => "Proot download",
        }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Install {
    #[default]
    Idle,
    Downloading {
        done_bytes: u64,
        total_bytes: u64,
    },
    /// The new version is in place (Linux) or its installer is running
    /// (Windows); Relay is about to close and open again.
    Installing,
    Failed {
        error: String,
    },
}

/// "v1.2.3" / "1.2.3" -> [1, 2, 3]. Anything after a '-' is ignored.
fn parts(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('-')
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// True when `candidate` is a later version than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parts(candidate), parts(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// Reads GitHub's "latest release" answer.
pub fn parse_release(v: &Value) -> Result<Release> {
    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("GitHub's answer has no version"))?;
    let prefix = format!("https://github.com/{REPO}/releases/download/");
    let assets: Vec<Asset> = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let url = a["browser_download_url"].as_str()?;
            // Only files attached to Relay's own releases.
            if !url
                .to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
            {
                return None;
            }
            Some(Asset {
                name: a["name"].as_str()?.to_string(),
                url: url.to_string(),
                size: a["size"].as_u64().unwrap_or(0),
                sha256: a["digest"]
                    .as_str()
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .map(str::to_ascii_lowercase),
            })
        })
        .collect();
    let find = |f: &dyn Fn(&str) -> bool| assets.iter().find(|a| f(&a.name)).cloned();
    Ok(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: v["body"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(4000)
            .collect(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        published_at: v["published_at"].as_str().unwrap_or("").to_string(),
        windows: find(&|n| n.starts_with("RelaySetup") && n.ends_with(".exe")),
        android: find(&|n| n == "Relay.apk"),
        android_dev: find(&|n| n == "Relay-Dev.apk"),
        appimage: find(&|n| n.starts_with("Relay-") && n.ends_with("-x86_64.AppImage")),
        proot: find(&|n| n.starts_with("Relay-") && n.ends_with("-arm64-proot.tar.gz")),
    })
}

#[derive(Default)]
pub struct Updater {
    cache: Mutex<Option<(Instant, Release)>>,
    install: Mutex<Install>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("relay/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()?)
}

impl Updater {
    pub fn install_state(&self) -> Install {
        lock(&self.install).clone()
    }

    /// The latest release, from the cache unless `refresh`.
    pub async fn latest(&self, refresh: bool) -> Result<Release> {
        if !refresh
            && let Some((at, r)) = lock(&self.cache).as_ref()
            && at.elapsed() < CACHE
        {
            return Ok(r.clone());
        }
        let v: Value = client()?
            .get(format!(
                "https://api.github.com/repos/{REPO}/releases/latest"
            ))
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("could not reach GitHub")?
            .error_for_status()
            .context("GitHub didn't answer")?
            .json()
            .await?;
        let r = parse_release(&v)?;
        *lock(&self.cache) = Some((Instant::now(), r.clone()));
        Ok(r)
    }

    /// Starts downloading and running the Windows installer of the latest
    /// release in the background.
    pub async fn install(self: &std::sync::Arc<Self>, dir: PathBuf) -> Result<Release> {
        if !cfg!(windows) {
            bail!("this Relay updates itself from its own app window");
        }
        self.install_package(Package::WindowsInstaller, dir, |_| {})
            .await
    }

    /// Starts downloading the latest release for `package` in the
    /// background. Windows runs the installer; on Linux the new version
    /// replaces the old file and `on_ready` gets its path, to restart into.
    /// `dir` is where downloads wait (Windows only; Linux downloads next to
    /// the file it replaces, so the swap is a rename on one disk).
    pub async fn install_package(
        self: &std::sync::Arc<Self>,
        package: Package,
        dir: PathBuf,
        on_ready: impl FnOnce(PathBuf) + Send + 'static,
    ) -> Result<Release> {
        if matches!(
            self.install_state(),
            Install::Downloading { .. } | Install::Installing
        ) {
            bail!("an update is already being installed");
        }
        let release = self.latest(true).await?;
        if !newer(&release.version, CURRENT) {
            bail!("Relay {CURRENT} is already the latest version");
        }
        let asset = package
            .asset(&release)
            .cloned()
            .ok_or_else(|| anyhow!("Relay {} has no {} yet", release.version, package.label()))?;
        let Some(sha) = asset.sha256.clone() else {
            bail!("GitHub didn't publish a checksum for the update, so Relay won't use it");
        };
        *lock(&self.install) = Install::Downloading {
            done_bytes: 0,
            total_bytes: asset.size,
        };
        let this = self.clone();
        let version = release.version.clone();
        tokio::spawn(async move {
            let result = async {
                match &package {
                    Package::WindowsInstaller => {
                        let path = this.download(&asset, &sha, &dir).await?;
                        launch(&path)?;
                        Ok(None)
                    }
                    Package::AppImage(old) => this
                        .replace_appimage(&asset, &sha, old, &version)
                        .await
                        .map(Some),
                    Package::ProotBinary(old) => {
                        this.replace_binary(&asset, &sha, old).await.map(Some)
                    }
                }
            }
            .await;
            *lock(&this.install) = match result {
                Ok(new_path) => {
                    if let Some(p) = new_path {
                        on_ready(p);
                    }
                    Install::Installing
                }
                Err(e) => {
                    tracing::warn!("update failed: {e:#}");
                    Install::Failed {
                        error: format!("{e:#}"),
                    }
                }
            };
        });
        Ok(release)
    }

    /// Puts the new AppImage where the old one is. A file named for the old
    /// version (Relay-1.0.11-x86_64.AppImage) is replaced by one named for
    /// the new version; any other name (Relay.AppImage) is kept. The running
    /// app keeps its copy open, so renaming over it is safe.
    async fn replace_appimage(
        &self,
        asset: &Asset,
        sha: &str,
        old: &Path,
        version: &str,
    ) -> Result<PathBuf> {
        let dir = old
            .parent()
            .ok_or_else(|| anyhow!("can't tell which folder the AppImage is in"))?;
        let name = old
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("can't read the AppImage's name"))?;
        let new_name = renamed_for(name, CURRENT, version);
        let new_path = dir.join(&new_name);
        let part = dir.join(format!(".{new_name}.part"));
        self.download_to(asset, sha, &part)
            .await
            .map_err(|e| writable_hint(e, dir))?;
        set_executable(&part).await?;
        tokio::fs::rename(&part, &new_path).await?;
        if new_path != old {
            let _ = tokio::fs::remove_file(old).await;
        }
        Ok(new_path)
    }

    /// Unpacks `relay-desktop` from the Proot tarball over the running one.
    async fn replace_binary(&self, asset: &Asset, sha: &str, old: &Path) -> Result<PathBuf> {
        let dir = old
            .parent()
            .ok_or_else(|| anyhow!("can't tell which folder Relay is in"))?;
        let work = dir.join(".relay-update");
        let _ = tokio::fs::remove_dir_all(&work).await;
        tokio::fs::create_dir_all(&work)
            .await
            .map_err(|e| writable_hint(e.into(), dir))?;
        let result = async {
            let tarball = work.join("relay.tar.gz");
            self.download_to(asset, sha, &tarball).await?;
            let out = tokio::process::Command::new("tar")
                .arg("-xzf")
                .arg(&tarball)
                .arg("-C")
                .arg(&work)
                .output()
                .await
                .context("could not run tar to unpack the update")?;
            if !out.status.success() {
                bail!(
                    "could not unpack the update: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            let bin = find_file(&work, "relay-desktop")
                .ok_or_else(|| anyhow!("the update has no relay-desktop in it"))?;
            let part = dir.join(".relay-desktop.part");
            tokio::fs::copy(&bin, &part).await?;
            set_executable(&part).await?;
            tokio::fs::rename(&part, old).await?;
            Ok(old.to_path_buf())
        }
        .await;
        let _ = tokio::fs::remove_dir_all(&work).await;
        result
    }

    async fn download(&self, asset: &Asset, sha: &str, dir: &Path) -> Result<PathBuf> {
        tokio::fs::create_dir_all(dir).await?;
        let name: String = asset
            .name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            .collect();
        let path = dir.join(&name);
        let tmp = dir.join(format!("{name}.part"));
        self.download_to(asset, sha, &tmp).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(path)
    }

    /// Downloads `asset` to `path`, reporting progress, and deletes it again
    /// unless its SHA-256 is `sha`.
    async fn download_to(&self, asset: &Asset, sha: &str, path: &Path) -> Result<()> {
        let mut res = client()?
            .get(&asset.url)
            .send()
            .await
            .context("could not download the update")?
            .error_for_status()?;
        let mut file = tokio::fs::File::create(path)
            .await
            .with_context(|| format!("could not write {}", path.display()))?;
        let mut hash = Sha256::new();
        let mut done = 0u64;
        let copied: Result<()> = async {
            while let Some(chunk) = res.chunk().await.context("the download broke off")? {
                hash.update(&chunk);
                file.write_all(&chunk).await?;
                done += chunk.len() as u64;
                *lock(&self.install) = Install::Downloading {
                    done_bytes: done,
                    total_bytes: asset.size.max(done),
                };
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        drop(file);
        if let Err(e) = copied {
            let _ = tokio::fs::remove_file(path).await;
            return Err(e);
        }
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != sha {
            let _ = tokio::fs::remove_file(path).await;
            bail!("the download doesn't match GitHub's checksum, so Relay didn't use it");
        }
        Ok(())
    }
}

/// "Relay-1.0.11-x86_64.AppImage" -> "Relay-1.0.12-x86_64.AppImage"; a name
/// without the old version stays as it is.
pub fn renamed_for(name: &str, current: &str, version: &str) -> String {
    if !current.is_empty() && name.contains(current) {
        name.replacen(current, version, 1)
    } else {
        name.to_string()
    }
}

fn writable_hint(e: anyhow::Error, dir: &Path) -> anyhow::Error {
    let denied = e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
    });
    if denied {
        anyhow!(
            "Relay can't write to {}, so it can't update itself there. Move Relay to a folder you own (like ~/Applications) and try again",
            dir.display()
        )
    } else {
        e
    }
}

#[cfg(unix)]
async fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// The first file called `name` under `dir` (a couple of levels deep).
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            let ft = entry.file_type().ok()?;
            if ft.is_file() && entry.file_name() == name {
                return Some(path);
            }
            if ft.is_dir()
                && depth > 0
                && let Some(found) = walk(&path, name, depth - 1)
            {
                return Some(found);
            }
        }
        None
    }
    walk(dir, name, 3)
}

/// Runs the installer. Windows asks for permission; the installer closes
/// Relay, updates it and opens it again.
#[cfg(windows)]
fn launch(path: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `start` goes through the shell, which shows the permission prompt the
    // installer needs.
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/RELAUNCH=1"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("could not start the installer")?;
    Ok(())
}

#[cfg(not(windows))]
fn launch(_path: &Path) -> Result<()> {
    bail!("Relay can only install updates itself on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions() {
        assert!(newer("1.0.2", "1.0.1"));
        assert!(newer("v1.1.0", "1.0.9"));
        assert!(newer("2.0", "1.9.9"));
        assert!(!newer("1.0.1", "1.0.1"));
        assert!(!newer("1.0.0", "1.0.1"));
        assert!(!newer("1.0.1-beta", "1.0.1"));
    }

    #[test]
    fn reads_a_release() {
        let v = json!({
            "tag_name": "v1.0.2",
            "body": "Fixes",
            "html_url": "https://github.com/wtanne20/Relay/releases/tag/v1.0.2",
            "published_at": "2026-11-05T00:00:00Z",
            "assets": [
                {"name": "Relay.apk", "size": 10, "digest": "sha256:ABC",
                 "browser_download_url": "https://github.com/wtanne20/Relay/releases/download/v1.0.2/Relay.apk"},
                {"name": "Relay-Dev.apk", "size": 20,
                 "browser_download_url": "https://github.com/wtanne20/Relay/releases/download/v1.0.2/Relay-Dev.apk"},
                {"name": "RelaySetup-1.0.2.exe", "size": 30, "digest": "sha256:def",
                 "browser_download_url": "https://github.com/wtanne20/Relay/releases/download/v1.0.2/RelaySetup-1.0.2.exe"},
                {"name": "Relay-1.0.2-x86_64.AppImage", "size": 40, "digest": "sha256:0A",
                 "browser_download_url": "https://github.com/wtanne20/Relay/releases/download/v1.0.2/Relay-1.0.2-x86_64.AppImage"},
                {"name": "Relay-1.0.2-arm64-proot.tar.gz", "size": 50, "digest": "sha256:0b",
                 "browser_download_url": "https://github.com/wtanne20/Relay/releases/download/v1.0.2/Relay-1.0.2-arm64-proot.tar.gz"},
                {"name": "RelaySetup-evil.exe", "size": 1,
                 "browser_download_url": "https://example.com/RelaySetup-evil.exe"}
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "1.0.2");
        assert_eq!(r.android.as_ref().unwrap().sha256.as_deref(), Some("abc"));
        assert_eq!(r.android_dev.as_ref().unwrap().size, 20);
        assert_eq!(r.windows.as_ref().unwrap().name, "RelaySetup-1.0.2.exe");
        assert_eq!(
            r.appimage.as_ref().unwrap().name,
            "Relay-1.0.2-x86_64.AppImage"
        );
        assert_eq!(
            r.proot.as_ref().unwrap().name,
            "Relay-1.0.2-arm64-proot.tar.gz"
        );
        assert_eq!(
            Package::AppImage("/x".into())
                .asset(&r)
                .unwrap()
                .sha256
                .as_deref(),
            Some("0a")
        );
        assert!(parse_release(&json!({"assets": []})).is_err());
    }

    #[test]
    fn renames_versioned_appimages() {
        assert_eq!(
            renamed_for("Relay-1.0.11-x86_64.AppImage", "1.0.11", "1.0.12"),
            "Relay-1.0.12-x86_64.AppImage"
        );
        assert_eq!(
            renamed_for("Relay.AppImage", "1.0.11", "1.0.12"),
            "Relay.AppImage"
        );
    }

    #[test]
    fn finds_the_binary_in_an_unpacked_tarball() {
        let dir = std::env::temp_dir().join(format!("relay-find-{}", std::process::id()));
        let inner = dir.join("Relay-1.0.2-arm64-proot");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("relay-desktop"), b"x").unwrap();
        assert_eq!(
            find_file(&dir, "relay-desktop"),
            Some(inner.join("relay-desktop"))
        );
        assert_eq!(find_file(&dir, "nope"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
