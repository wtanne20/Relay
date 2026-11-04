//! Update checks against Relay's GitHub releases, and installing a new
//! version on Windows: the installer is downloaded, its SHA-256 checked
//! against the one GitHub publishes, and run (it closes and reopens Relay).

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
    /// The installer is running; Relay is about to close.
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
            bail!("Relay can only install updates itself on Windows");
        }
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
        let asset = release
            .windows
            .clone()
            .ok_or_else(|| anyhow!("Relay {} has no Windows installer yet", release.version))?;
        let Some(sha) = asset.sha256.clone() else {
            bail!("GitHub didn't publish a checksum for the installer, so Relay won't run it");
        };
        *lock(&self.install) = Install::Downloading {
            done_bytes: 0,
            total_bytes: asset.size,
        };
        let this = self.clone();
        tokio::spawn(async move {
            let result = async {
                let path = this.download(&asset, &sha, &dir).await?;
                launch(&path)
            }
            .await;
            *lock(&this.install) = match result {
                Ok(()) => Install::Installing,
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

    async fn download(&self, asset: &Asset, sha: &str, dir: &Path) -> Result<PathBuf> {
        tokio::fs::create_dir_all(dir).await?;
        let name: String = asset
            .name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            .collect();
        let path = dir.join(&name);
        let tmp = dir.join(format!("{name}.part"));
        let mut res = client()?
            .get(&asset.url)
            .send()
            .await
            .context("could not download the installer")?
            .error_for_status()?;
        let mut file = tokio::fs::File::create(&tmp).await?;
        let mut hash = Sha256::new();
        let mut done = 0u64;
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
        drop(file);
        let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if got != sha {
            let _ = tokio::fs::remove_file(&tmp).await;
            bail!(
                "the downloaded installer doesn't match GitHub's checksum, so Relay didn't run it"
            );
        }
        tokio::fs::rename(&tmp, &path).await?;
        Ok(path)
    }
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
                {"name": "RelaySetup-evil.exe", "size": 1,
                 "browser_download_url": "https://example.com/RelaySetup-evil.exe"}
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "1.0.2");
        assert_eq!(r.android.as_ref().unwrap().sha256.as_deref(), Some("abc"));
        assert_eq!(r.android_dev.as_ref().unwrap().size, 20);
        assert_eq!(r.windows.as_ref().unwrap().name, "RelaySetup-1.0.2.exe");
        assert!(parse_release(&json!({"assets": []})).is_err());
    }
}
