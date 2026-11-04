//! Model downloads from Hugging Face: search GGUF repositories, list a
//! repository's GGUF files, and download them into `<data>/models` in the
//! background (like LM Studio's model browser).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const HF: &str = "https://huggingface.co";

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("relay/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .build()?)
}

/// `owner/name`, letters, digits, `-`, `_` and `.` only.
pub fn valid_repo(repo: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    matches!(repo.split_once('/'), Some((a, b)) if ok(a) && ok(b))
}

/// A path inside a repository: relative, no `..`, ends in .gguf.
pub fn valid_file(file: &str) -> bool {
    !file.is_empty()
        && !file.starts_with('/')
        && !file.contains('\\')
        && file
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && file.to_ascii_lowercase().ends_with(".gguf")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RepoHit {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    pub updated: Option<String>,
}

pub async fn search(query: &str) -> Result<Vec<RepoHit>> {
    let resp: Value = client()?
        .get(format!("{HF}/api/models"))
        .query(&[
            ("search", query),
            ("filter", "gguf"),
            ("sort", "downloads"),
            ("direction", "-1"),
            ("limit", "40"),
        ])
        .send()
        .await
        .context("could not reach Hugging Face")?
        .error_for_status()?
        .json()
        .await?;
    Ok(resp
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(RepoHit {
                        id: m.get("id")?.as_str()?.to_string(),
                        downloads: m.get("downloads").and_then(Value::as_u64).unwrap_or(0),
                        likes: m.get("likes").and_then(Value::as_u64).unwrap_or(0),
                        updated: m
                            .get("lastModified")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// One downloadable model: a single .gguf, or the parts of a split one.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GgufFile {
    /// The first (or only) file; what you pass to download.
    pub file: String,
    pub parts: Vec<String>,
    pub size_bytes: u64,
    /// Quantization from the name, e.g. "Q4_K_M".
    pub quant: Option<String>,
}

/// "model-00002-of-00003.gguf" -> Some(("model", 2, 3)).
fn split_part(name: &str) -> Option<(&str, u32, u32)> {
    let stem = name.strip_suffix(".gguf")?;
    let (head, of) = stem.rsplit_once("-of-")?;
    let (base, idx) = head.rsplit_once('-')?;
    Some((base, idx.parse().ok()?, of.parse().ok()?))
}

pub fn quant_of(name: &str) -> Option<String> {
    let upper = name.to_ascii_uppercase();
    let stem = upper.trim_end_matches(".GGUF");
    const QUANTS: &[&str] = &[
        "IQ1_S", "IQ1_M", "IQ2_XXS", "IQ2_XS", "IQ2_S", "IQ2_M", "IQ3_XXS", "IQ3_XS", "IQ3_S",
        "IQ3_M", "IQ4_XS", "IQ4_NL", "TQ1_0", "TQ2_0", "Q2_K_XL", "Q2_K_L", "Q2_K_S", "Q2_K",
        "Q3_K_XL", "Q3_K_L", "Q3_K_M", "Q3_K_S", "Q4_K_XL", "Q4_K_L", "Q4_K_M", "Q4_K_S", "Q4_0",
        "Q4_1", "Q5_K_XL", "Q5_K_L", "Q5_K_M", "Q5_K_S", "Q5_0", "Q5_1", "Q6_K_XL", "Q6_K_L",
        "Q6_K", "Q8_K_XL", "Q8_0", "MXFP4", "BF16", "F16", "F32",
    ];
    // Longest match wins, so "UD-Q2_K_XL" is Q2_K_XL rather than Q2_K.
    QUANTS
        .iter()
        .filter(|q| stem.contains(*q))
        .max_by_key(|q| q.len())
        .map(|q| q.to_string())
}

/// Groups a repository tree listing into downloadable models.
pub fn group_files(tree: &[Value]) -> Vec<GgufFile> {
    let mut singles: Vec<GgufFile> = Vec::new();
    let mut split: std::collections::BTreeMap<String, Vec<(u32, String, u64)>> = Default::default();
    for e in tree {
        if e.get("type").and_then(Value::as_str) != Some("file") {
            continue;
        }
        let Some(path) = e.get("path").and_then(Value::as_str) else {
            continue;
        };
        let name = path.rsplit('/').next().unwrap_or(path);
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".gguf") || lower.contains("mmproj") || !valid_file(path) {
            continue;
        }
        let size = e
            .get("lfs")
            .and_then(|l| l.get("size"))
            .or_else(|| e.get("size"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        match split_part(path) {
            Some((base, idx, _)) => {
                split
                    .entry(base.to_string())
                    .or_default()
                    .push((idx, path.to_string(), size))
            }
            None => singles.push(GgufFile {
                file: path.to_string(),
                parts: vec![path.to_string()],
                size_bytes: size,
                quant: quant_of(name),
            }),
        }
    }
    for (_, mut parts) in split {
        parts.sort();
        let first = parts[0].1.clone();
        singles.push(GgufFile {
            quant: quant_of(&first),
            size_bytes: parts.iter().map(|p| p.2).sum(),
            parts: parts.into_iter().map(|p| p.1).collect(),
            file: first,
        });
    }
    singles.sort_by_key(|f| f.size_bytes);
    singles
}

/// The repository's vision file (mmproj), if it has one: F16 first, then
/// BF16, then the smallest other.
pub fn pick_projector(tree: &[Value]) -> Option<GgufFile> {
    tree.iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("file"))
        .filter_map(|e| {
            let path = e.get("path").and_then(Value::as_str)?;
            let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
            if !name.starts_with("mmproj") || !name.ends_with(".gguf") || !valid_file(path) {
                return None;
            }
            let size = e
                .get("lfs")
                .and_then(|l| l.get("size"))
                .or_else(|| e.get("size"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let rank = if name.contains("bf16") {
                1
            } else if name.contains("f16") {
                0
            } else {
                2
            };
            Some((rank, size, path.to_string()))
        })
        .min()
        .map(|(_, size, path)| GgufFile {
            quant: quant_of(&path),
            parts: vec![path.clone()],
            file: path,
            size_bytes: size,
        })
}

pub async fn files(repo: &str) -> Result<Vec<GgufFile>> {
    Ok(files_and_projector(repo).await?.0)
}

/// The repository's models, and its vision file if any.
pub async fn files_and_projector(repo: &str) -> Result<(Vec<GgufFile>, Option<GgufFile>)> {
    if !valid_repo(repo) {
        bail!("not a Hugging Face repository name: {repo}");
    }
    let tree: Value = client()?
        .get(format!("{HF}/api/models/{repo}/tree/main?recursive=true"))
        .send()
        .await
        .context("could not reach Hugging Face")?
        .error_for_status()?
        .json()
        .await?;
    let tree = tree.as_array().map(Vec::as_slice).unwrap_or_default();
    Ok((group_files(tree), pick_projector(tree)))
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Downloading,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct Download {
    pub id: String,
    pub repo: String,
    pub file: String,
    pub dest: String,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub state: DownloadState,
    pub error: Option<String>,
    #[serde(skip)]
    cancel: CancellationToken,
}

/// Background downloads, newest first.
#[derive(Default)]
pub struct Downloads {
    list: Mutex<Vec<Download>>,
}

impl Downloads {
    pub fn list(&self) -> Vec<Download> {
        self.list.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Download)) {
        let mut l = self.list.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = l.iter_mut().find(|d| d.id == id) {
            f(d);
        }
    }

    pub fn cancel(&self, id: &str) -> bool {
        let l = self.list.lock().unwrap_or_else(|e| e.into_inner());
        match l.iter().find(|d| d.id == id) {
            Some(d) => {
                d.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Starts downloading `file` (and its other parts, if split) from `repo`
    /// into `<models_dir>/<owner>/<name>/`.
    pub fn start(
        self: &Arc<Self>,
        models_dir: &Path,
        repo: &str,
        model: GgufFile,
    ) -> Result<Download> {
        let dir = repo
            .split('/')
            .fold(models_dir.to_path_buf(), |p, s| p.join(s));
        let dest = model.file.split('/').fold(dir.clone(), |p, s| p.join(s));
        {
            let l = self.list.lock().unwrap_or_else(|e| e.into_inner());
            if l.iter().any(|d| {
                d.repo == repo && d.file == model.file && d.state == DownloadState::Downloading
            }) {
                bail!("that model is already downloading");
            }
        }
        let d = Download {
            id: uuid::Uuid::new_v4().to_string(),
            repo: repo.to_string(),
            file: model.file.clone(),
            dest: dest.to_string_lossy().into_owned(),
            total_bytes: model.size_bytes,
            done_bytes: 0,
            state: DownloadState::Downloading,
            error: None,
            cancel: CancellationToken::new(),
        };
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(0, d.clone());
        let this = self.clone();
        let (id, repo, cancel) = (d.id.clone(), repo.to_string(), d.cancel.clone());
        tokio::spawn(async move {
            let r = tokio::select! {
                r = this.run(&id, &repo, &dir, &model.parts) => r,
                _ = cancel.cancelled() => Err(anyhow!("cancelled")),
            };
            match r {
                Ok(()) => this.update(&id, |d| d.state = DownloadState::Done),
                Err(e) if cancel.is_cancelled() => {
                    tracing::info!("download {repo} cancelled");
                    let _ = e;
                    remove_partials(&dir, &model.parts).await;
                    this.update(&id, |d| d.state = DownloadState::Cancelled);
                }
                Err(e) => {
                    tracing::warn!("download {repo} failed: {e:#}");
                    remove_partials(&dir, &model.parts).await;
                    this.update(&id, |d| {
                        d.state = DownloadState::Failed;
                        d.error = Some(format!("{e:#}"));
                    });
                }
            }
        });
        Ok(d)
    }

    async fn run(&self, id: &str, repo: &str, dir: &Path, parts: &[String]) -> Result<()> {
        let http = client()?;
        let mut base: u64 = 0;
        for part in parts {
            let dest = part.split('/').fold(dir.to_path_buf(), |p, s| p.join(s));
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if dest.is_file() {
                // Already downloaded (an earlier run); never overwrite.
                base += dest.metadata().map(|m| m.len()).unwrap_or(0);
                continue;
            }
            let url = format!("{HF}/{repo}/resolve/main/{part}?download=true");
            let tmp = part_path(&dest);
            let done = fetch(&http, &url, &tmp, |n| {
                let now = base + n;
                self.update(id, |d| d.done_bytes = now);
            })
            .await?;
            tokio::fs::rename(&tmp, &dest).await?;
            base += done;
        }
        Ok(())
    }
}

/// Parallel connections for one large file. A single HTTP stream from the
/// Hugging Face CDN is often capped at a few MB/s; several ranges at once
/// fill the line.
const SEGMENTS: u64 = 8;
/// Files smaller than this download over one connection.
const SEGMENT_MIN: u64 = 128 * 1024 * 1024;

/// Total size, if the server serves byte ranges.
async fn ranged_size(http: &reqwest::Client, url: &str) -> Option<u64> {
    let resp = http
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .ok()?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return None;
    }
    // "bytes 0-0/12345"
    let total = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .parse()
        .ok()?;
    Some(total)
}

/// Downloads `url` into `tmp`, reporting the bytes done so far. Returns the size.
async fn fetch(
    http: &reqwest::Client,
    url: &str,
    tmp: &Path,
    progress: impl Fn(u64),
) -> Result<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(total) = ranged_size(http, url).await.filter(|t| *t >= SEGMENT_MIN) else {
        return fetch_single(http, url, tmp, progress).await;
    };
    let file = tokio::fs::File::create(tmp).await?;
    file.set_len(total).await?;
    drop(file);

    let done = Arc::new(AtomicU64::new(0));
    let step = total.div_ceil(SEGMENTS);
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..SEGMENTS {
        let start = i * step;
        if start >= total {
            break;
        }
        let end = (start + step).min(total); // exclusive
        let (http, url, tmp, done) = (
            http.clone(),
            url.to_string(),
            tmp.to_path_buf(),
            done.clone(),
        );
        tasks.spawn(async move { fetch_range(&http, &url, &tmp, start, end, &done).await });
    }
    loop {
        tokio::select! {
            r = tasks.join_next() => match r {
                None => break,
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(e))) => return Err(e),
                Some(Err(e)) => bail!("download task failed: {e}"),
            },
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        progress(done.load(Ordering::Relaxed));
    }
    progress(done.load(Ordering::Relaxed));
    let got = done.load(Ordering::Relaxed);
    if got != total {
        bail!("download incomplete ({got} of {total} bytes)");
    }
    Ok(total)
}

/// One byte range `[start, end)`, written in place. Retries a few times,
/// carrying on from where it stopped.
async fn fetch_range(
    http: &reqwest::Client,
    url: &str,
    tmp: &Path,
    start: u64,
    end: u64,
    done: &std::sync::atomic::AtomicU64,
) -> Result<()> {
    use std::sync::atomic::Ordering;
    use tokio::io::AsyncSeekExt;
    let mut pos = start;
    let mut attempt: u64 = 0;
    while pos < end {
        let r: Result<()> = async {
            let resp = http
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={pos}-{}", end - 1))
                .send()
                .await?
                .error_for_status()?;
            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                bail!("the server stopped serving byte ranges");
            }
            let mut f = tokio::fs::OpenOptions::new().write(true).open(tmp).await?;
            f.seek(std::io::SeekFrom::Start(pos)).await?;
            let mut f = tokio::io::BufWriter::with_capacity(4 * 1024 * 1024, f);
            let mut resp = resp;
            while let Some(chunk) = tokio::time::timeout(Duration::from_secs(120), resp.chunk())
                .await
                .map_err(|_| anyhow!("download stalled"))??
            {
                let take = (chunk.len() as u64).min(end - pos) as usize;
                f.write_all(&chunk[..take]).await?;
                pos += take as u64;
                done.fetch_add(take as u64, Ordering::Relaxed);
                if pos >= end {
                    break;
                }
            }
            f.flush().await?;
            Ok(())
        }
        .await;
        if pos >= end {
            return Ok(());
        }
        attempt += 1;
        match r {
            Err(e) if attempt > 5 => return Err(e),
            Ok(()) if attempt > 5 => bail!("download kept stopping early"),
            Err(e) => tracing::info!("download range retry: {e:#}"),
            Ok(()) => {}
        }
        tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
    }
    Ok(())
}

/// One connection, start to end.
async fn fetch_single(
    http: &reqwest::Client,
    url: &str,
    tmp: &Path,
    progress: impl Fn(u64),
) -> Result<u64> {
    let mut resp = http.get(url).send().await?.error_for_status()?;
    let file = tokio::fs::File::create(tmp).await?;
    let mut file = tokio::io::BufWriter::with_capacity(4 * 1024 * 1024, file);
    let mut done: u64 = 0;
    while let Some(chunk) = tokio::time::timeout(Duration::from_secs(120), resp.chunk())
        .await
        .map_err(|_| anyhow!("download stalled"))??
    {
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done);
    }
    file.flush().await?;
    Ok(done)
}

impl Downloads {
    /// Copies a GGUF file that's elsewhere on the PC (another app's folder)
    /// into `<models_dir>/imported/`, so Relay owns its own copy. The source
    /// is only read, never moved or changed. Shows in the downloads list.
    pub fn import(self: &Arc<Self>, models_dir: &Path, source: &Path) -> Result<Download> {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| anyhow!("not a file: {}", source.display()))?;
        let dir = models_dir.join("imported");
        let dest = dir.join(&name);
        if dest.exists() {
            bail!("Relay already has a copy of {name}");
        }
        let total = source.metadata()?.len();
        {
            let l = self.list.lock().unwrap_or_else(|e| e.into_inner());
            if l.iter()
                .any(|d| d.dest == dest.to_string_lossy() && d.state == DownloadState::Downloading)
            {
                bail!("{name} is already being copied");
            }
        }
        let d = Download {
            id: uuid::Uuid::new_v4().to_string(),
            repo: "this PC".into(),
            file: name,
            dest: dest.to_string_lossy().into_owned(),
            total_bytes: total,
            done_bytes: 0,
            state: DownloadState::Downloading,
            error: None,
            cancel: CancellationToken::new(),
        };
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(0, d.clone());
        let this = self.clone();
        let (id, cancel, src) = (d.id.clone(), d.cancel.clone(), source.to_path_buf());
        tokio::spawn(async move {
            let tmp = part_path(&dest);
            let r = tokio::select! {
                r = this.copy(&id, &src, &dir, &tmp, &dest) => r,
                _ = cancel.cancelled() => Err(anyhow!("cancelled")),
            };
            match r {
                Ok(()) => this.update(&id, |d| d.state = DownloadState::Done),
                Err(e) => {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    let cancelled = cancel.is_cancelled();
                    if !cancelled {
                        tracing::warn!("copying {} failed: {e:#}", src.display());
                    }
                    this.update(&id, |d| {
                        if cancelled {
                            d.state = DownloadState::Cancelled;
                        } else {
                            d.state = DownloadState::Failed;
                            d.error = Some(format!("{e:#}"));
                        }
                    });
                }
            }
        });
        Ok(d)
    }

    async fn copy(&self, id: &str, src: &Path, dir: &Path, tmp: &Path, dest: &Path) -> Result<()> {
        use tokio::io::AsyncReadExt;
        tokio::fs::create_dir_all(dir).await?;
        let mut from = tokio::fs::File::open(src).await?;
        let mut to = tokio::fs::File::create(tmp).await?;
        let mut buf = vec![0u8; 8 * 1024 * 1024];
        let mut done: u64 = 0;
        loop {
            let n = from.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n]).await?;
            done += n as u64;
            self.update(id, |d| d.done_bytes = done);
        }
        to.flush().await?;
        drop(to);
        tokio::fs::rename(tmp, dest).await?;
        Ok(())
    }

    /// Copies a model's files (`(from, to)` pairs, all parts of a split
    /// model) to another library. The originals are left in place; the
    /// caller removes them once the copy is in use. Shows in the downloads
    /// list as "Moving".
    pub fn move_files(
        self: &Arc<Self>,
        name: &str,
        files: Vec<(PathBuf, PathBuf)>,
    ) -> Result<Download> {
        let Some((_, first_dest)) = files.first() else {
            bail!("nothing to move");
        };
        for (_, to) in &files {
            if to.exists() {
                bail!("{} is already in that library", to.display());
            }
        }
        let total = files
            .iter()
            .map(|(f, _)| f.metadata().map(|m| m.len()).unwrap_or(0))
            .sum();
        {
            let l = self.list.lock().unwrap_or_else(|e| e.into_inner());
            if l.iter().any(|d| {
                d.state == DownloadState::Downloading
                    && files.iter().any(|(f, t)| {
                        d.dest == t.to_string_lossy() || d.dest == f.to_string_lossy()
                    })
            }) {
                bail!("{name} is already being moved");
            }
        }
        let d = Download {
            id: uuid::Uuid::new_v4().to_string(),
            repo: "move".into(),
            file: name.to_string(),
            dest: first_dest.to_string_lossy().into_owned(),
            total_bytes: total,
            done_bytes: 0,
            state: DownloadState::Downloading,
            error: None,
            cancel: CancellationToken::new(),
        };
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(0, d.clone());
        let this = self.clone();
        let (id, cancel) = (d.id.clone(), d.cancel.clone());
        tokio::spawn(async move {
            let r = tokio::select! {
                r = this.copy_all(&id, &files) => r,
                _ = cancel.cancelled() => Err(anyhow!("cancelled")),
            };
            match r {
                Ok(()) => this.update(&id, |d| d.state = DownloadState::Done),
                Err(e) => {
                    // Remove anything half-copied; the originals are untouched.
                    for (_, to) in &files {
                        let _ = tokio::fs::remove_file(part_path(to)).await;
                        let _ = tokio::fs::remove_file(to).await;
                    }
                    let cancelled = cancel.is_cancelled();
                    if !cancelled {
                        tracing::warn!("moving a model failed: {e:#}");
                    }
                    this.update(&id, |d| {
                        if cancelled {
                            d.state = DownloadState::Cancelled;
                        } else {
                            d.state = DownloadState::Failed;
                            d.error = Some(format!("{e:#}"));
                        }
                    });
                }
            }
        });
        Ok(d)
    }

    async fn copy_all(&self, id: &str, files: &[(PathBuf, PathBuf)]) -> Result<()> {
        use tokio::io::AsyncReadExt;
        let mut done: u64 = 0;
        for (from_path, to_path) in files {
            if let Some(dir) = to_path.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            let tmp = part_path(to_path);
            let mut from = tokio::fs::File::open(from_path).await?;
            let mut to = tokio::fs::File::create(&tmp).await?;
            let mut buf = vec![0u8; 8 * 1024 * 1024];
            loop {
                let n = from.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                to.write_all(&buf[..n]).await?;
                done += n as u64;
                self.update(id, |d| d.done_bytes = done);
            }
            to.flush().await?;
            drop(to);
            // Check the copy is complete before it replaces anything.
            let want = from_path.metadata()?.len();
            let got = tokio::fs::metadata(&tmp).await?.len();
            if want != got {
                bail!(
                    "the copy of {} is incomplete ({got} of {want} bytes)",
                    from_path.display()
                );
            }
            tokio::fs::rename(&tmp, to_path).await?;
        }
        Ok(())
    }

    /// State of one entry, if it's still listed.
    pub fn state(&self, id: &str) -> Option<DownloadState> {
        let l = self.list.lock().unwrap_or_else(|e| e.into_inner());
        l.iter().find(|d| d.id == id).map(|d| d.state)
    }

    /// The finished copy of `source`, if one was imported this session.
    pub fn imported_copy_done(&self, id: &str) -> Option<String> {
        let l = self.list.lock().unwrap_or_else(|e| e.into_inner());
        l.iter()
            .find(|d| d.id == id && d.state == DownloadState::Done)
            .map(|d| d.dest.clone())
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

async fn remove_partials(dir: &Path, parts: &[String]) {
    for part in parts {
        let dest = part.split('/').fold(dir.to_path_buf(), |p, s| p.join(s));
        let _ = tokio::fs::remove_file(part_path(&dest)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Serves `data` with byte-range support, like the Hugging Face CDN.
    async fn range_server(data: Arc<Vec<u8>>, ranges: bool) -> String {
        use axum::http::{HeaderMap, StatusCode, header};
        let app = axum::Router::new().route(
            "/f",
            axum::routing::get(move |h: HeaderMap| {
                let data = data.clone();
                async move {
                    let range = h
                        .get(header::RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("bytes="))
                        .and_then(|v| v.split_once('-'))
                        .and_then(|(a, b)| {
                            Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                        });
                    match range {
                        Some((a, b)) if ranges => {
                            let b = b.min(data.len() - 1);
                            (
                                StatusCode::PARTIAL_CONTENT,
                                [(
                                    header::CONTENT_RANGE,
                                    format!("bytes {a}-{b}/{}", data.len()),
                                )],
                                data[a..=b].to_vec(),
                            )
                        }
                        _ => (
                            StatusCode::OK,
                            [(header::CONTENT_RANGE, String::new())],
                            data.to_vec(),
                        ),
                    }
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = l.local_addr().expect("addr");
        tokio::spawn(async move { axum::serve(l, app).await });
        format!("http://{addr}/f")
    }

    #[tokio::test]
    async fn downloads_in_parallel_ranges_and_without() {
        let data: Vec<u8> = (0..(SEGMENT_MIN as usize + 12_345))
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        let data = Arc::new(data);
        let dir = std::env::temp_dir().join(format!("relay-fetch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let http = client().expect("client");
        for ranges in [true, false] {
            let url = range_server(data.clone(), ranges).await;
            let tmp = dir.join(format!("m-{ranges}.gguf.part"));
            let last = std::sync::Mutex::new(0u64);
            let n = fetch(&http, &url, &tmp, |d| *last.lock().expect("lock") = d)
                .await
                .expect("fetch");
            assert_eq!(n, data.len() as u64);
            assert_eq!(*last.lock().expect("lock"), n);
            assert!(
                std::fs::read(&tmp).expect("read") == *data,
                "bytes match (ranges={ranges})"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validates_names() {
        assert!(valid_repo("unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF"));
        assert!(!valid_repo("../etc"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a/.."));
        assert!(valid_file("Q4_K_M/model-Q4_K_M-00001-of-00002.gguf"));
        assert!(!valid_file("../x.gguf"));
        assert!(!valid_file("/abs.gguf"));
        assert!(!valid_file("readme.md"));
    }

    #[test]
    fn reads_quant_names() {
        assert_eq!(quant_of("M-UD-Q2_K_XL.gguf").as_deref(), Some("Q2_K_XL"));
        assert_eq!(quant_of("M-Q2_K.gguf").as_deref(), Some("Q2_K"));
        assert_eq!(quant_of("M-UD-TQ1_0.gguf").as_deref(), Some("TQ1_0"));
        assert_eq!(quant_of("M-IQ2_XXS.gguf").as_deref(), Some("IQ2_XXS"));
        assert_eq!(quant_of("M-Q4_K_M.gguf").as_deref(), Some("Q4_K_M"));
        assert_eq!(quant_of("readme"), None);
    }

    #[test]
    fn groups_split_and_single_files() {
        let tree = vec![
            json!({"type": "file", "path": "README.md", "size": 10}),
            json!({"type": "file", "path": "m-Q8_0.gguf", "size": 1, "lfs": {"size": 800}}),
            json!({"type": "file", "path": "m-Q4_K_M.gguf", "lfs": {"size": 400}}),
            json!({"type": "file", "path": "mmproj-F16.gguf", "lfs": {"size": 50}}),
            json!({"type": "directory", "path": "BF16"}),
            json!({"type": "file", "path": "BF16/m-BF16-00002-of-00002.gguf", "lfs": {"size": 700}}),
            json!({"type": "file", "path": "BF16/m-BF16-00001-of-00002.gguf", "lfs": {"size": 900}}),
        ];
        let p = pick_projector(&tree).expect("vision file");
        assert_eq!((p.file.as_str(), p.size_bytes), ("mmproj-F16.gguf", 50));
        assert!(pick_projector(&tree[..3]).is_none());
        let g = group_files(&tree);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].file, "m-Q4_K_M.gguf");
        assert_eq!(g[0].quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(g[1].size_bytes, 800);
        assert_eq!(g[2].file, "BF16/m-BF16-00001-of-00002.gguf");
        assert_eq!(g[2].parts.len(), 2);
        assert_eq!(g[2].size_bytes, 1600);
        assert_eq!(g[2].quant.as_deref(), Some("BF16"));
    }
}
