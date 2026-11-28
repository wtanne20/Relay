//! Browse and install: skills from public GitHub collections (folders with a
//! SKILL.md, like `anthropics/skills`), ready-made tools (custom tools with
//! commands for the user's shell), and MCP servers from the official MCP
//! registry, turned into launch settings Relay can save.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::tools::custom::{CustomParam, CustomTool};

/// Collections shown even when the user hasn't added any.
pub const DEFAULT_SKILL_SOURCES: &[&str] = &["anthropics/skills"];
/// The file Relay writes into each skill folder it installs.
pub const MARKER: &str = ".relay-skill.json";

const MAX_SKILLS: usize = 300;
const MAX_FILES: usize = 400;
const MAX_BYTES: u64 = 50 * 1024 * 1024;
const MAX_SOURCES: usize = 30;
const CACHE_FOR: Duration = Duration::from_secs(15 * 60);

fn github_api() -> String {
    std::env::var("RELAY_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into())
}
fn github_raw() -> String {
    std::env::var("RELAY_GITHUB_RAW").unwrap_or_else(|_| "https://raw.githubusercontent.com".into())
}
fn mcp_registry() -> String {
    std::env::var("RELAY_MCP_REGISTRY")
        .unwrap_or_else(|_| "https://registry.modelcontextprotocol.io".into())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("relay/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60))
        .build()?)
}

// ---------- skill sources ----------

/// A GitHub repository, optionally a branch and a folder inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub repo: String,
    pub git_ref: Option<String>,
    pub path: String,
}

impl Source {
    /// Accepts `owner/repo`, `owner/repo/some/folder`, `owner/repo@branch`,
    /// and github.com links, including `/tree/<branch>/<folder>` ones.
    pub fn parse(input: &str) -> Result<Source, String> {
        let bad = || {
            format!(
                "'{}' isn't a GitHub collection. Use owner/repo or a github.com link",
                input.trim()
            )
        };
        let mut s = input.trim().trim_end_matches('/').to_string();
        for prefix in ["https://", "http://"] {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest.to_string();
            }
        }
        if let Some(rest) = s.strip_prefix("www.") {
            s = rest.to_string();
        }
        let from_link = s.starts_with("github.com/");
        if let Some(rest) = s.strip_prefix("github.com/") {
            s = rest.to_string();
        }
        let (body, at_ref) = match s.split_once('@') {
            Some((a, b)) => (a.to_string(), Some(b.to_string())),
            None => (s.clone(), None),
        };
        let parts: Vec<&str> = body.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            return Err(bad());
        }
        let repo = format!("{}/{}", parts[0], parts[1].trim_end_matches(".git"));
        if !crate::hub::valid_repo(&repo) {
            return Err(bad());
        }
        let mut rest = &parts[2..];
        let mut git_ref = at_ref;
        if from_link && rest.len() >= 2 && (rest[0] == "tree" || rest[0] == "blob") {
            git_ref = Some(rest[1].to_string());
            rest = &rest[2..];
        }
        let path = rest.join("/");
        if !path.is_empty() && !crate::hub::valid_path(&path) {
            return Err(bad());
        }
        if let Some(r) = &git_ref
            && (r.is_empty()
                || r.contains("..")
                || !r
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)))
        {
            return Err(bad());
        }
        Ok(Source {
            repo,
            git_ref,
            path,
        })
    }

    /// The canonical spelling saved in config.toml.
    pub fn id(&self) -> String {
        let mut s = self.repo.clone();
        if !self.path.is_empty() {
            s.push('/');
            s.push_str(&self.path);
        }
        if let Some(r) = &self.git_ref {
            s.push('@');
            s.push_str(r);
        }
        s
    }
}

/// The user's collections after the built-in ones, without duplicates.
pub fn all_sources(user: &[String]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = DEFAULT_SKILL_SOURCES
        .iter()
        .map(|s| (s.to_string(), true))
        .collect();
    for s in user {
        if !out.iter().any(|(o, _)| o == s) {
            out.push((s.clone(), false));
        }
    }
    out
}

pub fn validate_sources(sources: &[String]) -> Result<(), String> {
    if sources.len() > MAX_SOURCES {
        return Err(format!("at most {MAX_SOURCES} skill collections"));
    }
    for s in sources {
        let parsed = Source::parse(s)?;
        if parsed.id() != *s {
            return Err(format!(
                "skill collection '{s}' should be written '{}'",
                parsed.id()
            ));
        }
    }
    Ok(())
}

// ---------- listing a collection ----------

#[derive(Debug, Clone, Serialize)]
pub struct RemoteSkill {
    /// Folder in the repository, e.g. `skills/pdf`.
    pub path: String,
    pub name: String,
    pub description: String,
    pub files: usize,
    pub bytes: u64,
    /// Changes whenever a file in the folder changes.
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone)]
struct TreeFile {
    path: String,
    sha: String,
    size: u64,
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub repo: String,
    pub commit: String,
    pub skills: Vec<RemoteSkill>,
    files: Vec<TreeFile>,
}

type Cache = Mutex<HashMap<String, (Instant, Arc<Listing>)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(Default::default)
}

async fn github_json(url: &str) -> Result<Value> {
    let resp = client()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("could not reach GitHub ({url})"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("GitHub has no such repository or branch");
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        bail!("GitHub is limiting requests from this computer. Try again in a few minutes");
    }
    if !status.is_success() {
        bail!("GitHub answered {status}");
    }
    Ok(resp.json().await?)
}

async fn raw_bytes(repo: &str, commit: &str, path: &str) -> Result<Vec<u8>> {
    let encoded: Vec<String> = path.split('/').map(urlencode).collect();
    let url = format!("{}/{repo}/{commit}/{}", github_raw(), encoded.join("/"));
    let resp = client()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("downloading {path}"))?;
    if !resp.status().is_success() {
        bail!("downloading {path}: GitHub answered {}", resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn is_skill_file(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|f| f.eq_ignore_ascii_case("skill.md"))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(a, _)| a).unwrap_or("")
}

/// Skill folders in a tree: the shallowest folder holding a SKILL.md, under
/// `prefix`.
fn skill_dirs(files: &[TreeFile], prefix: &str) -> Vec<String> {
    let mut dirs: Vec<String> = files
        .iter()
        .filter(|f| is_skill_file(&f.path))
        .map(|f| parent(&f.path).to_string())
        .filter(|d| under(d, prefix))
        .collect();
    dirs.sort_by_key(|d| (d.matches('/').count(), d.clone()));
    let mut keep: Vec<String> = Vec::new();
    for d in dirs {
        if !keep.iter().any(|k| under(&d, k) && *k != d) && !keep.contains(&d) {
            keep.push(d);
        }
    }
    keep.sort();
    keep
}

/// Whether `path` is `dir` or inside it ("" is the whole repository).
fn under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

fn files_in<'a>(files: &'a [TreeFile], dir: &str) -> Vec<&'a TreeFile> {
    files
        .iter()
        .filter(|f| {
            if dir.is_empty() {
                true
            } else {
                f.path.starts_with(&format!("{dir}/"))
            }
        })
        .collect()
}

fn folder_version(files: &[&TreeFile], dir: &str) -> String {
    let mut h = Sha256::new();
    for f in files {
        h.update(&f.path.as_bytes()[dir.len()..]);
        h.update([0]);
        h.update(f.sha.as_bytes());
        h.update([0]);
    }
    let d = h.finalize();
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Lists the skills in a collection (cached for a few minutes).
pub async fn list(source: &Source, refresh: bool) -> Result<Arc<Listing>> {
    let key = source.id();
    if !refresh
        && let Some((at, l)) = cache().lock().unwrap().get(&key)
        && at.elapsed() < CACHE_FOR
    {
        return Ok(l.clone());
    }
    let api = github_api();
    let git_ref = source.git_ref.clone().unwrap_or_else(|| "HEAD".into());
    let commit = github_json(&format!(
        "{api}/repos/{}/commits/{}",
        source.repo,
        urlencode(&git_ref)
    ))
    .await?;
    let commit = commit
        .get("sha")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("GitHub didn't say which commit {git_ref} is"))?
        .to_string();
    let tree = github_json(&format!(
        "{api}/repos/{}/git/trees/{commit}?recursive=1",
        source.repo
    ))
    .await?;
    let files: Vec<TreeFile> = tree
        .get("tree")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("blob"))
        .filter_map(|e| {
            Some(TreeFile {
                path: e.get("path")?.as_str()?.to_string(),
                sha: e.get("sha")?.as_str()?.to_string(),
                size: e.get("size").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .filter(|f| crate::hub::valid_path(&f.path))
        .collect();
    let dirs: Vec<String> = skill_dirs(&files, &source.path)
        .into_iter()
        .take(MAX_SKILLS)
        .collect();
    if dirs.is_empty() {
        bail!(
            "no skills found in {} (a skill is a folder with a SKILL.md)",
            source.id()
        );
    }

    // Read each SKILL.md for its name and description, a few at a time.
    let sem = Arc::new(tokio::sync::Semaphore::new(8));
    let mut set = tokio::task::JoinSet::new();
    for (i, dir) in dirs.iter().enumerate() {
        let file = files
            .iter()
            .find(|f| parent(&f.path) == dir && is_skill_file(&f.path))
            .map(|f| f.path.clone())
            .unwrap_or_default();
        let (repo, commit, sem) = (source.repo.clone(), commit.clone(), sem.clone());
        set.spawn(async move {
            let _permit = sem.acquire().await;
            let text = raw_bytes(&repo, &commit, &file)
                .await
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            (i, crate::skills::parse_frontmatter(&text))
        });
    }
    let mut meta: Vec<(Option<String>, Option<String>)> = vec![(None, None); dirs.len()];
    while let Some(r) = set.join_next().await {
        if let Ok((i, m)) = r {
            meta[i] = m;
        }
    }
    let skills = dirs
        .iter()
        .zip(meta)
        .map(|(dir, (name, description))| {
            let inside = files_in(&files, dir);
            RemoteSkill {
                name: name.unwrap_or_else(|| folder_name(dir, &source.repo)),
                description: description.unwrap_or_default(),
                files: inside.len(),
                bytes: inside.iter().map(|f| f.size).sum(),
                version: folder_version(&inside, dir),
                url: if dir.is_empty() {
                    format!("https://github.com/{}", source.repo)
                } else {
                    format!("https://github.com/{}/tree/{commit}/{dir}", source.repo)
                },
                path: dir.clone(),
            }
        })
        .collect();
    let listing = Arc::new(Listing {
        repo: source.repo.clone(),
        commit,
        skills,
        files,
    });
    cache()
        .lock()
        .unwrap()
        .insert(key, (Instant::now(), listing.clone()));
    Ok(listing)
}

/// The folder an installed skill gets in `<data>/skills`.
fn folder_name(dir: &str, repo: &str) -> String {
    let last = if dir.is_empty() {
        repo.rsplit('/').next().unwrap_or("skill")
    } else {
        dir.rsplit('/').next().unwrap_or(dir)
    };
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches('.').to_string();
    if clean.is_empty() {
        "skill".into()
    } else {
        clean
    }
}

// ---------- installed skills ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallMarker {
    pub source: String,
    pub repo: String,
    pub path: String,
    pub commit: String,
    pub version: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Installed {
    pub folder: String,
    #[serde(flatten)]
    pub marker: InstallMarker,
}

/// Skills Relay installed into `skills_dir`.
pub fn installed(skills_dir: &Path) -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(skills_dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if let Ok(text) = std::fs::read_to_string(p.join(MARKER))
            && let Ok(marker) = serde_json::from_str::<InstallMarker>(&text)
        {
            out.push(Installed {
                folder: e.file_name().to_string_lossy().into_owned(),
                marker,
            });
        }
    }
    out.sort_by(|a, b| a.folder.cmp(&b.folder));
    out
}

/// Downloads a skill folder into `skills_dir/<folder>`. Replaces a folder
/// Relay installed from the same place (an update), never anything else.
pub async fn install(
    source: &Source,
    path: &str,
    skills_dir: &Path,
) -> Result<Installed, InstallError> {
    let listing = list(source, false).await.map_err(InstallError::Remote)?;
    let skill = listing
        .skills
        .iter()
        .find(|s| s.path == path)
        .cloned()
        .ok_or_else(|| {
            InstallError::Invalid(format!("{} has no skill at '{path}'", source.id()))
        })?;
    let files: Vec<TreeFile> = files_in(&listing.files, path)
        .into_iter()
        .cloned()
        .collect();
    if files.len() > MAX_FILES {
        return Err(InstallError::Invalid(format!(
            "this skill has {} files; Relay installs at most {MAX_FILES}",
            files.len()
        )));
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    if total > MAX_BYTES {
        return Err(InstallError::Invalid(format!(
            "this skill is {} MB; Relay installs at most {} MB",
            total / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        )));
    }

    let folder = folder_name(path, &source.repo);
    let target = skills_dir.join(&folder);
    let existing = std::fs::read_to_string(target.join(MARKER))
        .ok()
        .and_then(|t| serde_json::from_str::<InstallMarker>(&t).ok());
    if target.exists() {
        match &existing {
            Some(m) if m.repo == listing.repo && m.path == path => {}
            _ => {
                return Err(InstallError::Conflict(format!(
                    "there is already a skill folder named '{folder}' in {}",
                    skills_dir.display()
                )));
            }
        }
    }

    tokio::fs::create_dir_all(skills_dir)
        .await
        .map_err(|e| InstallError::Remote(e.into()))?;
    let staging = skills_dir.join(format!(".installing-{}", uuid::Uuid::new_v4().simple()));
    let result = download_into(&listing, &files, path, &staging).await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(InstallError::Remote(e));
    }
    let marker = InstallMarker {
        source: source.id(),
        repo: listing.repo.clone(),
        path: path.to_string(),
        commit: listing.commit.clone(),
        version: skill.version.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    let write = async {
        tokio::fs::write(staging.join(MARKER), serde_json::to_string_pretty(&marker)?).await?;
        if target.exists() {
            let old = skills_dir.join(format!(".old-{}", uuid::Uuid::new_v4().simple()));
            tokio::fs::rename(&target, &old).await?;
            if let Err(e) = tokio::fs::rename(&staging, &target).await {
                let _ = tokio::fs::rename(&old, &target).await;
                return Err(anyhow::Error::from(e));
            }
            let _ = tokio::fs::remove_dir_all(&old).await;
        } else {
            tokio::fs::rename(&staging, &target).await?;
        }
        anyhow::Ok(())
    };
    if let Err(e) = write.await {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(InstallError::Remote(e.context("saving the skill")));
    }
    Ok(Installed { folder, marker })
}

async fn download_into(listing: &Listing, files: &[TreeFile], dir: &str, to: &Path) -> Result<()> {
    tokio::fs::create_dir_all(to).await?;
    let sem = Arc::new(tokio::sync::Semaphore::new(6));
    let mut set = tokio::task::JoinSet::new();
    for f in files {
        let rel = if dir.is_empty() {
            f.path.clone()
        } else {
            f.path[dir.len() + 1..].to_string()
        };
        if !crate::hub::valid_path(&rel) || rel.split('/').any(|p| p == MARKER) {
            continue;
        }
        let dest: PathBuf = rel.split('/').fold(to.to_path_buf(), |p, s| p.join(s));
        let (repo, commit, path, sem) = (
            listing.repo.clone(),
            listing.commit.clone(),
            f.path.clone(),
            sem.clone(),
        );
        set.spawn(async move {
            let _permit = sem.acquire().await;
            let bytes = raw_bytes(&repo, &commit, &path).await?;
            if let Some(p) = dest.parent() {
                tokio::fs::create_dir_all(p).await?;
            }
            tokio::fs::write(&dest, bytes).await?;
            anyhow::Ok(())
        });
    }
    while let Some(r) = set.join_next().await {
        r.map_err(|e| anyhow!("download failed: {e}"))??;
    }
    Ok(())
}

/// Removes a skill Relay installed. Folders without Relay's marker are the
/// user's own and are left alone.
pub fn uninstall(skills_dir: &Path, folder: &str) -> Result<(), InstallError> {
    if folder.is_empty()
        || folder.starts_with('.')
        || folder.contains(['/', '\\'])
        || folder == ".."
    {
        return Err(InstallError::Invalid("bad folder name".into()));
    }
    let dir = skills_dir.join(folder);
    if !dir.join(MARKER).is_file() {
        return Err(InstallError::NotFound);
    }
    std::fs::remove_dir_all(&dir).map_err(|e| InstallError::Remote(e.into()))?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("skill not found")]
    NotFound,
    #[error("{0:#}")]
    Remote(anyhow::Error),
}

// ---------- ready-made tools ----------

/// A tool in the catalog. `command` works in any shell; otherwise `sh` and
/// `powershell` are the two spellings.
struct Template {
    name: &'static str,
    title: &'static str,
    category: &'static str,
    description: &'static str,
    command: Option<&'static str>,
    sh: Option<&'static str>,
    powershell: Option<&'static str>,
    params: &'static [(&'static str, &'static str, bool)],
    timeout_seconds: u64,
}

const TEMPLATES: &[Template] = &[
    Template {
        name: "search_text",
        title: "Search the code",
        category: "Files",
        description: "Search the project's files for a word or regular expression and show matching lines with file names and line numbers. Use it to find where something is defined or used.",
        command: None,
        sh: Some(
            "grep -rnI --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=target -e {pattern} . | head -n 200",
        ),
        powershell: Some(
            "Get-ChildItem -Recurse -File | Where-Object { $_.FullName -notmatch '[\\\\/](\\.git|node_modules|target)[\\\\/]' } | Select-String -Pattern {pattern} | Select-Object -First 200 | ForEach-Object { \"$(Resolve-Path -Relative $_.Path):$($_.LineNumber): $($_.Line.Trim())\" }",
        ),
        params: &[("pattern", "Text or regular expression to look for", true)],
        timeout_seconds: 0,
    },
    Template {
        name: "find_files",
        title: "Find files by name",
        category: "Files",
        description: "Find files in the project whose name matches a pattern such as *.py or config*. Use it when you know roughly what a file is called but not where it is.",
        command: None,
        sh: Some(
            "find . -type f -name {pattern} -not -path '*/.git/*' -not -path '*/node_modules/*' | head -n 200",
        ),
        powershell: Some(
            "Get-ChildItem -Recurse -File -Filter {pattern} | Where-Object { $_.FullName -notmatch '[\\\\/](\\.git|node_modules)[\\\\/]' } | Select-Object -First 200 | Resolve-Path -Relative",
        ),
        params: &[("pattern", "File name pattern, e.g. *.ts", true)],
        timeout_seconds: 0,
    },
    Template {
        name: "git_status",
        title: "Git status",
        category: "Git",
        description: "Show the current branch and which files are changed, staged or new. Use it before and after editing to see what changed.",
        command: Some("git status --short --branch"),
        sh: None,
        powershell: None,
        params: &[],
        timeout_seconds: 0,
    },
    Template {
        name: "git_log",
        title: "Recent commits",
        category: "Git",
        description: "Show the latest commits, one per line. Use it when asked what changed recently.",
        command: Some("git log --oneline --decorate -n {count}"),
        sh: None,
        powershell: None,
        params: &[("count", "How many commits to show, e.g. 20", true)],
        timeout_seconds: 0,
    },
    Template {
        name: "git_diff",
        title: "Show changes",
        category: "Git",
        description: "Show the uncommitted changes in the project, or only in one file or folder. Use it to review edits.",
        command: Some("git diff --stat --patch -- {path}"),
        sh: None,
        powershell: None,
        params: &[("path", "A file or folder; leave out for everything", false)],
        timeout_seconds: 0,
    },
    Template {
        name: "git_show",
        title: "Show a commit",
        category: "Git",
        description: "Show what one commit changed: its message, the files and the diff.",
        command: Some("git show --stat --patch {commit}"),
        sh: None,
        powershell: None,
        params: &[("commit", "Commit id, branch or tag, e.g. HEAD~1", true)],
        timeout_seconds: 0,
    },
    Template {
        name: "npm_run",
        title: "Run an npm script",
        category: "Build and test",
        description: "Run a script from package.json, such as test, build or lint, and show its output.",
        command: Some("npm run {script}"),
        sh: None,
        powershell: None,
        params: &[("script", "The script name from package.json", true)],
        timeout_seconds: 600,
    },
    Template {
        name: "cargo_test",
        title: "Run Rust tests",
        category: "Build and test",
        description: "Run the Rust project's tests with cargo, optionally only the tests whose names contain some text.",
        command: Some("cargo test {filter}"),
        sh: None,
        powershell: None,
        params: &[("filter", "Only run tests whose name contains this", false)],
        timeout_seconds: 900,
    },
    Template {
        name: "pytest",
        title: "Run Python tests",
        category: "Build and test",
        description: "Run the Python project's tests with pytest, optionally only one file, folder or test.",
        command: Some("python -m pytest -q {target}"),
        sh: None,
        powershell: None,
        params: &[(
            "target",
            "A test file, folder or test id; leave out for all",
            false,
        )],
        timeout_seconds: 900,
    },
    Template {
        name: "disk_usage",
        title: "Folder sizes",
        category: "System",
        description: "Show how much disk space each file and folder in the project uses, biggest last.",
        command: None,
        sh: Some("du -sh -- * .[!.]* 2>/dev/null | sort -h | tail -n 40"),
        powershell: Some(
            "Get-ChildItem -Force | ForEach-Object { $b = if ($_.PSIsContainer) { (Get-ChildItem -LiteralPath $_.FullName -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum } else { $_.Length }; [pscustomobject]@{ MB = [math]::Round($b / 1MB, 1); Name = $_.Name } } | Sort-Object MB | Format-Table -AutoSize",
        ),
        params: &[],
        timeout_seconds: 0,
    },
    Template {
        name: "listening_ports",
        title: "Listening ports",
        category: "System",
        description: "List the network ports programs on this computer are listening on, with the program. Use it to find a running dev server or a port that's already taken.",
        command: None,
        sh: Some("ss -ltnp 2>/dev/null || netstat -an | grep LISTEN"),
        powershell: Some(
            "Get-NetTCPConnection -State Listen | Sort-Object LocalPort | Select-Object LocalAddress, LocalPort, @{ n = 'Program'; e = { (Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).ProcessName } } | Format-Table -AutoSize",
        ),
        params: &[],
        timeout_seconds: 0,
    },
    Template {
        name: "top_processes",
        title: "Busiest programs",
        category: "System",
        description: "Show the programs using the most CPU and memory right now.",
        command: None,
        sh: Some("ps aux --sort=-%cpu 2>/dev/null | head -n 16 || ps aux | head -n 16"),
        powershell: Some(
            "Get-Process | Sort-Object CPU -Descending | Select-Object -First 15 Name, Id, @{ n = 'CPU s'; e = { [math]::Round($_.CPU, 1) } }, @{ n = 'Memory MB'; e = { [math]::Round($_.WorkingSet64 / 1MB) } } | Format-Table -AutoSize",
        ),
        params: &[],
        timeout_seconds: 0,
    },
];

fn shell_kind(shell: &str) -> &'static str {
    let lower = shell.to_lowercase();
    if lower.contains("powershell") || lower.contains("pwsh") {
        "powershell"
    } else if lower.ends_with("cmd") || lower.ends_with("cmd.exe") {
        "cmd"
    } else {
        "sh"
    }
}

/// The ready-made tools that work in `shell`, as tools ready to add.
pub fn tool_templates(shell: &str) -> Vec<(&'static str, &'static str, CustomTool)> {
    let kind = shell_kind(shell);
    TEMPLATES
        .iter()
        .filter_map(|t| {
            let command = t.command.or(match kind {
                "powershell" => t.powershell,
                "sh" => t.sh,
                _ => None,
            })?;
            Some((
                t.title,
                t.category,
                CustomTool {
                    name: t.name.into(),
                    description: t.description.into(),
                    command: command.into(),
                    parameters: t
                        .params
                        .iter()
                        .map(|(n, d, r)| CustomParam {
                            name: (*n).into(),
                            description: (*d).into(),
                            required: *r,
                        })
                        .collect(),
                    enabled: true,
                    timeout_seconds: t.timeout_seconds,
                },
            ))
        })
        .collect()
}

// ---------- MCP registry ----------

/// Something the user fills in before an MCP server can start.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct McpInput {
    /// `env`, `header` or `arg`.
    pub target: &'static str,
    /// The environment variable or header it sets (empty for `arg`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// What to call it on screen.
    pub name: String,
    pub description: String,
    pub required: bool,
    pub secret: bool,
    pub default: String,
    /// When set, the value replaces `{name}` in this text (e.g.
    /// `Bearer {token}`), and the result is what's saved.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub template: String,
    /// For `arg`: the index in `args` the value goes into.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arg_index: Option<usize>,
    /// For an optional `arg` left empty: also remove this index (the
    /// `--flag` before the value).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_index: Option<usize>,
}

/// One way to run a server from the registry.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpOption {
    /// `npm`, `pypi`, `docker` or `remote`.
    pub kind: String,
    pub label: String,
    /// The package (npm/PyPI name or image) without its version.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub package: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    pub inputs: Vec<McpInput>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpEntry {
    /// The registry's name, e.g. `io.github.owner/server`.
    pub id: String,
    /// A name Relay can save it under.
    pub name: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub repository: String,
    pub website: String,
    pub options: Vec<McpOption>,
}

/// A Relay MCP server name: a lowercase letter, then a-z 0-9 _, at most 32,
/// no double underscore.
pub fn mcp_name(id: &str) -> String {
    let (ns, last) = id.rsplit_once('/').unwrap_or(("", id));
    // `com.notion/mcp`: the publisher is the better name.
    let generic = ["mcp", "server", "mcp-server", "mcp_server", "remote", "api"];
    let last = if generic.contains(&last.to_lowercase().as_str()) && !ns.is_empty() {
        ns.rsplit('.').next().unwrap_or(last)
    } else {
        last
    };
    let mut out = String::new();
    for c in last.to_lowercase().chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '_' };
        if c == '_' && (out.is_empty() || out.ends_with('_')) {
            continue;
        }
        out.push(c);
    }
    for suffix in ["_mcp_server", "_server", "_mcp"] {
        if out.len() > suffix.len() + 2 && out.ends_with(suffix) {
            out.truncate(out.len() - suffix.len());
        }
    }
    if let Some(rest) = out.strip_prefix("mcp_")
        && rest.len() >= 2
    {
        out = rest.to_string();
    }
    let mut out: String = out
        .trim_start_matches(|c: char| !c.is_ascii_lowercase())
        .into();
    out.truncate(32);
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() { "server".into() } else { out }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Inputs for the `{variables}` in a value: one per variable.
fn var_inputs(holder: &Value, template: &str, target: &'static str, key: &str) -> Vec<McpInput> {
    let Some(vars) = holder.get("variables").and_then(Value::as_object) else {
        return Vec::new();
    };
    vars.iter()
        .filter(|(name, _)| template.contains(&format!("{{{name}}}")))
        .map(|(name, v)| McpInput {
            target,
            key: key.to_string(),
            name: name.clone(),
            description: Some(s(v, "description"))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| s(holder, "description")),
            required: b(v, "isRequired") || b(holder, "isRequired"),
            secret: b(v, "isSecret") || b(holder, "isSecret"),
            default: s(v, "default"),
            template: template.to_string(),
            ..Default::default()
        })
        .collect()
}

/// Adds registry `arguments` to `args`. Fixed values go in as they are;
/// values the user must give become inputs. Returns false if one can't be
/// expressed.
fn push_args(list: Option<&Value>, args: &mut Vec<String>, inputs: &mut Vec<McpInput>) -> bool {
    for a in list.and_then(Value::as_array).into_iter().flatten() {
        let value = a
            .get("value")
            .and_then(Value::as_str)
            .or_else(|| a.get("default").and_then(Value::as_str))
            .map(str::to_string);
        let kind = s(a, "type");
        let name = s(a, "name");
        let flag = if kind == "named" {
            if name.is_empty() {
                return false;
            }
            Some(name.clone())
        } else {
            None
        };
        match value {
            Some(v) if v.contains('{') => {
                // A value with variables, e.g. `TOKEN={token}`.
                let vars = var_inputs(a, &v, "arg", "");
                if vars.is_empty() {
                    return false;
                }
                let drop_index = flag.map(|f| {
                    args.push(f);
                    args.len() - 1
                });
                let index = args.len();
                args.push(v);
                inputs.extend(vars.into_iter().map(|mut i| {
                    i.arg_index = Some(index);
                    i.drop_index = drop_index;
                    i
                }));
            }
            Some(v) => {
                if let Some(f) = flag {
                    args.push(f);
                }
                if !v.is_empty() || kind != "named" {
                    args.push(v);
                }
            }
            None if b(a, "isRequired") => {
                if let Some(f) = &flag {
                    args.push(f.clone());
                }
                let hint = s(a, "valueHint");
                inputs.push(McpInput {
                    target: "arg",
                    name: match &flag {
                        Some(f) => f.trim_start_matches('-').into(),
                        None if hint.is_empty() => "argument".into(),
                        None => hint,
                    },
                    description: s(a, "description"),
                    required: true,
                    secret: b(a, "isSecret"),
                    arg_index: Some(args.len()),
                    ..Default::default()
                });
                args.push(String::new());
            }
            None => {}
        }
    }
    true
}

fn env_inputs(pkg: &Value) -> Vec<McpInput> {
    let mut out = Vec::new();
    for e in pkg
        .get("environmentVariables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = s(e, "name");
        if key.is_empty() {
            continue;
        }
        let value = s(e, "value");
        let vars = if value.contains('{') {
            var_inputs(e, &value, "env", &key)
        } else {
            Vec::new()
        };
        if vars.is_empty() {
            out.push(McpInput {
                target: "env",
                name: key.clone(),
                key,
                description: s(e, "description"),
                required: b(e, "isRequired"),
                secret: b(e, "isSecret"),
                default: if value.is_empty() {
                    s(e, "default")
                } else {
                    value
                },
                ..Default::default()
            });
        } else {
            out.extend(vars);
        }
    }
    out
}

/// Inputs for a hosted server's headers.
fn header_inputs(r: &Value) -> Vec<McpInput> {
    let mut out = Vec::new();
    for h in r
        .get("headers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = s(h, "name");
        if key.is_empty() {
            continue;
        }
        let value = s(h, "value");
        let vars = if value.contains('{') {
            var_inputs(h, &value, "header", &key)
        } else {
            Vec::new()
        };
        if vars.is_empty() {
            let auth = key.eq_ignore_ascii_case("authorization");
            out.push(McpInput {
                target: "header",
                name: key.clone(),
                key,
                description: s(h, "description"),
                required: b(h, "isRequired"),
                secret: b(h, "isSecret"),
                default: if value.is_empty() && auth {
                    // Most hosted servers take a bearer token.
                    "Bearer ".into()
                } else {
                    value
                },
                ..Default::default()
            });
        } else {
            out.extend(vars);
        }
    }
    out
}

fn package_option(pkg: &Value) -> McpOption {
    let kind = s(pkg, "registryType");
    let id = s(pkg, "identifier");
    let version = s(pkg, "version");
    let transport = pkg
        .pointer("/transport/type")
        .and_then(Value::as_str)
        .unwrap_or("stdio")
        .to_string();
    let mut opt = McpOption {
        kind: kind.clone(),
        label: String::new(),
        package: id
            .split(['@', ':'])
            .find(|p| !p.is_empty())
            .map(|p| {
                if id.starts_with('@') {
                    format!("@{p}")
                } else {
                    p.to_string()
                }
            })
            .unwrap_or_default(),
        supported: true,
        reason: String::new(),
        command: String::new(),
        args: Vec::new(),
        url: String::new(),
        inputs: env_inputs(pkg),
    };
    let mut args = Vec::new();
    let mut inputs = Vec::new();
    let mut ok = push_args(pkg.get("runtimeArguments"), &mut args, &mut inputs);
    match kind.as_str() {
        "npm" => {
            opt.label = "Node.js package (npx)".into();
            opt.command = "npx".into();
            if !args.iter().any(|a| a == "-y" || a == "--yes") {
                args.insert(0, "-y".into());
                for i in &mut inputs {
                    i.arg_index = i.arg_index.map(|x| x + 1);
                    i.drop_index = i.drop_index.map(|x| x + 1);
                }
            }
            args.push(if version.is_empty() {
                id.clone()
            } else {
                format!("{id}@{version}")
            });
        }
        "pypi" => {
            opt.label = "Python package (uvx)".into();
            opt.command = "uvx".into();
            args.push(if version.is_empty() {
                id.clone()
            } else {
                format!("{id}=={version}")
            });
        }
        "oci" => {
            opt.label = "Docker image".into();
            opt.command = "docker".into();
            let mut pre = vec!["run".to_string(), "-i".into(), "--rm".into()];
            let mut keys: Vec<&str> = Vec::new();
            for e in &opt.inputs {
                if !keys.contains(&e.key.as_str()) {
                    keys.push(&e.key);
                    pre.push("-e".into());
                    pre.push(e.key.clone());
                }
            }
            let shift = pre.len();
            for i in &mut inputs {
                i.arg_index = i.arg_index.map(|x| x + shift);
                i.drop_index = i.drop_index.map(|x| x + shift);
            }
            pre.extend(args);
            args = pre;
            let image = if version.is_empty() || id.contains(':') || id.contains('@') {
                id.clone()
            } else {
                format!("{id}:{version}")
            };
            args.push(image);
        }
        other => {
            opt.label = format!("{other} package");
            opt.supported = false;
            opt.reason = format!("Relay can't run {other} packages yet");
        }
    }
    ok &= push_args(pkg.get("packageArguments"), &mut args, &mut inputs);
    if !ok && opt.supported {
        opt.supported = false;
        opt.reason = "it needs arguments Relay can't fill in".into();
    }
    if transport != "stdio" && opt.supported {
        opt.supported = false;
        opt.reason = format!("the package talks {transport}, and Relay runs packages over stdio");
    }
    opt.args = args;
    opt.inputs.extend(inputs);
    opt
}

fn remote_option(r: &Value) -> McpOption {
    let transport = s(r, "type");
    let url = s(r, "url");
    let mut opt = McpOption {
        kind: "remote".into(),
        label: "Hosted server".into(),
        package: String::new(),
        supported: transport == "streamable-http",
        reason: String::new(),
        command: String::new(),
        args: Vec::new(),
        url: url.clone(),
        inputs: header_inputs(r),
    };
    if !opt.supported {
        opt.reason = if transport == "sse" {
            "it only offers the older SSE connection".into()
        } else {
            format!("Relay can't connect over {transport}")
        };
    }
    if url.contains('{') && opt.supported {
        opt.supported = false;
        opt.reason = "its address needs values filled in".into();
    }
    opt
}

/// Turns one registry result into an entry Relay can show.
pub fn mcp_entry(item: &Value) -> Option<McpEntry> {
    let server = item.get("server").unwrap_or(item);
    let id = s(server, "name");
    if id.is_empty() {
        return None;
    }
    if let Some(status) = item
        .pointer("/_meta/io.modelcontextprotocol.registry~1official/status")
        .and_then(Value::as_str)
        && status != "active"
    {
        return None;
    }
    let mut options: Vec<McpOption> = server
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(package_option)
        .collect();
    options.extend(
        server
            .get("remotes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(remote_option),
    );
    // Supported ways first, then fewer things to fill in.
    options.sort_by_key(|o| (!o.supported, o.inputs.iter().filter(|i| i.required).count()));
    Some(McpEntry {
        name: mcp_name(&id),
        title: s(server, "title"),
        description: s(server, "description"),
        version: s(server, "version"),
        repository: server
            .pointer("/repository/url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        website: s(server, "websiteUrl"),
        options,
        id,
    })
}

/// Searches the official MCP registry (latest versions only).
/// Pages read from the registry per search: it lists matches by name, so
/// the best one (often the service's own) can be a few pages in.
const MCP_PAGES: usize = 3;

/// Searches the official MCP registry (latest versions only), best matches
/// first.
pub async fn mcp_search(
    query: &str,
    cursor: Option<&str>,
) -> Result<(Vec<McpEntry>, Option<String>)> {
    let http = client()?;
    let mut seen = std::collections::HashSet::new();
    let mut entries: Vec<McpEntry> = Vec::new();
    let mut next = cursor.filter(|c| !c.is_empty()).map(str::to_string);
    // Names match as text, so "github" matches every io.github.* server.
    // Ask separately for servers the service itself publishes
    // (`com.notion/...`, `io.github.github/...`).
    let word: String = query
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if next.is_none() && word.len() >= 2 {
        let publisher = format!(".{word}/");
        let resp = http
            .get(format!("{}/v0/servers", mcp_registry()))
            .query(&[
                ("limit", "30"),
                ("version", "latest"),
                ("search", publisher.as_str()),
            ])
            .send()
            .await;
        if let Ok(resp) = resp
            && resp.status().is_success()
            && let Ok(body) = resp.json::<Value>().await
        {
            entries.extend(
                body.get("servers")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(mcp_entry)
                    .filter(|e| seen.insert(e.id.clone())),
            );
        }
    }
    // "home assistant" is spelled home-assistant or homeassistant in names.
    let words: Vec<&str> = query.split_whitespace().collect();
    let terms: Vec<String> = if words.len() > 1 && next.is_none() {
        vec![words.join("-"), words.concat()]
    } else {
        vec![query.trim().to_string()]
    };
    let pages = if terms.len() > 1 { 1 } else { MCP_PAGES };
    for term in &terms {
        let mut page = 0;
        next = cursor.filter(|c| !c.is_empty()).map(str::to_string);
        while page < pages {
            page += 1;
            let mut q: Vec<(&str, &str)> = vec![("limit", "100"), ("version", "latest")];
            if !term.is_empty() {
                q.push(("search", term));
            }
            if let Some(c) = &next {
                q.push(("cursor", c));
            }
            let resp = http
                .get(format!("{}/v0/servers", mcp_registry()))
                .query(&q)
                .send()
                .await
                .context("could not reach the MCP registry")?;
            if !resp.status().is_success() {
                bail!("the MCP registry answered {}", resp.status());
            }
            let body: Value = resp
                .json()
                .await
                .context("reading the MCP registry's answer")?;
            entries.extend(
                body.get("servers")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(mcp_entry)
                    .filter(|e| seen.insert(e.id.clone())),
            );
            next = body
                .pointer("/metadata/nextCursor")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if next.is_none() {
                break;
            }
        }
    }
    if terms.len() > 1 {
        // Paging through two searches at once isn't possible.
        next = None;
    }
    entries.sort_by_key(|e| rank(e, query));
    Ok((entries, next))
}

/// Lower is a better match for `query`: the service's own publisher, then
/// an exact name, then a name containing it. Installable ones first in each.
fn rank(e: &McpEntry, query: &str) -> (u8, bool) {
    let q: String = query
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let squash = |s: &str| -> String {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    };
    let (ns, short) = e.id.split_once('/').unwrap_or(("", &e.id));
    let publisher = ns.rsplit('.').next().unwrap_or(ns);
    let short = squash(short);
    let short_bare = short
        .trim_start_matches("mcp")
        .trim_end_matches("server")
        .trim_end_matches("mcp")
        .to_string();
    let tier = if q.is_empty() {
        3
    } else if squash(publisher) == q || (ns.starts_with("io.github.") && squash(&ns[10..]) == q) {
        0
    } else if short == q || short_bare == q {
        1
    } else if short.contains(&q) || squash(&e.title).contains(&q) {
        2
    } else {
        3
    };
    (tier, !e.options.iter().any(|o| o.supported))
}

/// What `GET /catalog/skills` answers for one collection.
pub fn skills_view(
    source: &str,
    listing: &Listing,
    have: &[Installed],
    local_names: &[String],
) -> Value {
    let skills: Vec<Value> = listing
        .skills
        .iter()
        .map(|k| {
            let mine = have
                .iter()
                .find(|i| i.marker.repo == listing.repo && i.marker.path == k.path);
            json!({
                "path": k.path,
                "name": k.name,
                "description": k.description,
                "files": k.files,
                "bytes": k.bytes,
                "url": k.url,
                "installed": mine.map(|i| json!({
                    "folder": i.folder,
                    "update_available": i.marker.version != k.version,
                    "installed_at": i.marker.installed_at,
                })),
                // A skill with this name is already there, not from here.
                "name_taken": mine.is_none() && local_names.iter().any(|n| n.eq_ignore_ascii_case(&k.name)),
            })
        })
        .collect();
    json!({
        "source": source,
        "repo": listing.repo,
        "commit": listing.commit,
        "skills": skills,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sources() {
        let p = Source::parse("anthropics/skills").unwrap();
        assert_eq!(p.id(), "anthropics/skills");
        let p = Source::parse("https://github.com/anthropics/skills/tree/main/skills/").unwrap();
        assert_eq!(p.repo, "anthropics/skills");
        assert_eq!(p.git_ref.as_deref(), Some("main"));
        assert_eq!(p.path, "skills");
        assert_eq!(p.id(), "anthropics/skills/skills@main");
        assert_eq!(Source::parse(&p.id()).unwrap(), p);
        assert_eq!(Source::parse("github.com/a/b.git").unwrap().id(), "a/b");
        assert!(Source::parse("nope").is_err());
        assert!(Source::parse("a/b/../c").is_err());
        assert!(Source::parse("a/b@..").is_err());
        assert!(validate_sources(&["a/b".into()]).is_ok());
        assert!(validate_sources(&["https://github.com/a/b".into()]).is_err());
    }

    fn tf(path: &str) -> TreeFile {
        TreeFile {
            path: path.into(),
            sha: format!("sha-{path}"),
            size: 10,
        }
    }

    #[test]
    fn finds_shallowest_skill_folders() {
        let files = vec![
            tf("README.md"),
            tf("skills/pdf/SKILL.md"),
            tf("skills/pdf/scripts/fill.py"),
            tf("skills/pdf/examples/inner/SKILL.md"),
            tf("skills/docx/skill.md"),
            tf("template/SKILL.md"),
        ];
        assert_eq!(
            skill_dirs(&files, ""),
            vec!["skills/docx", "skills/pdf", "template"]
        );
        assert_eq!(
            skill_dirs(&files, "skills"),
            vec!["skills/docx", "skills/pdf"]
        );
        let inside = files_in(&files, "skills/pdf");
        assert_eq!(inside.len(), 3);
        let v1 = folder_version(&inside, "skills/pdf");
        let mut changed = files.clone();
        changed[2].sha = "other".into();
        assert_ne!(
            v1,
            folder_version(&files_in(&changed, "skills/pdf"), "skills/pdf")
        );
        assert_eq!(folder_name("skills/pdf", "a/b"), "pdf");
        assert_eq!(folder_name("", "a/my skill"), "my-skill");
    }

    #[test]
    fn templates_fit_the_shell_and_validate() {
        for shell in ["sh", "/bin/bash", "powershell.exe", "pwsh", "cmd.exe"] {
            let list = tool_templates(shell);
            assert!(!list.is_empty());
            for (_, _, t) in list {
                t.validate().unwrap_or_else(|e| panic!("{shell}: {e}"));
            }
        }
        assert!(
            tool_templates("cmd.exe")
                .iter()
                .all(|(_, _, t)| t.name != "disk_usage")
        );
        assert!(
            tool_templates("pwsh")
                .iter()
                .any(|(_, _, t)| t.command.starts_with("Get-ChildItem"))
        );
    }

    #[test]
    fn names_mcp_servers() {
        assert_eq!(mcp_name("io.github.github/github-mcp-server"), "github");
        assert_eq!(
            mcp_name("com.pulsemcp/remote-filesystem"),
            "remote_filesystem"
        );
        assert_eq!(mcp_name("ai.x/mcp-weather"), "weather");
        assert_eq!(mcp_name("a/123-go"), "go");
        assert_eq!(mcp_name("a/---"), "server");
        assert_eq!(mcp_name("com.notion/mcp"), "notion");
        assert!(mcp_name(&format!("a/{}", "x".repeat(80))).len() <= 32);
    }

    #[test]
    fn turns_registry_entries_into_launch_settings() {
        let item = json!({
            "server": {
                "name": "io.github.acme/weather-mcp",
                "title": "Weather",
                "description": "Forecasts",
                "version": "1.2.0",
                "repository": { "url": "https://github.com/acme/weather" },
                "packages": [
                    {
                        "registryType": "npm", "identifier": "@acme/weather", "version": "1.2.0",
                        "transport": { "type": "stdio" },
                        "runtimeArguments": [{ "type": "positional", "value": "-y" }],
                        "packageArguments": [
                            { "type": "named", "name": "--units", "default": "metric" },
                            { "type": "positional", "valueHint": "city", "isRequired": true }
                        ],
                        "environmentVariables": [
                            { "name": "WEATHER_KEY", "isRequired": true, "isSecret": true, "description": "API key" }
                        ]
                    },
                    { "registryType": "nuget", "identifier": "Acme.Weather", "version": "1.2.0" },
                    { "registryType": "pypi", "identifier": "acme-weather", "version": "1.2.0" },
                    { "registryType": "oci", "identifier": "acme/weather", "version": "1.2.0",
                      "environmentVariables": [{ "name": "WEATHER_KEY" }] }
                ],
                "remotes": [
                    { "type": "sse", "url": "https://w.example/sse" },
                    { "type": "streamable-http", "url": "https://w.example/mcp",
                      "headers": [{ "name": "Authorization", "value": "Bearer {key}", "isSecret": true, "isRequired": true }] }
                ]
            },
            "_meta": { "io.modelcontextprotocol.registry/official": { "status": "active" } }
        });
        let e = mcp_entry(&item).unwrap();
        assert_eq!(e.name, "weather");
        assert_eq!(e.repository, "https://github.com/acme/weather");
        let npm = e.options.iter().find(|o| o.kind == "npm").unwrap();
        assert!(npm.supported);
        assert_eq!(npm.command, "npx");
        assert_eq!(npm.package, "@acme/weather");
        assert_eq!(
            npm.args,
            vec!["-y", "@acme/weather@1.2.0", "--units", "metric", ""]
        );
        let city = npm.inputs.iter().find(|i| i.target == "arg").unwrap();
        assert_eq!(city.arg_index, Some(4));
        assert!(
            npm.inputs
                .iter()
                .any(|i| i.target == "env" && i.secret && i.required)
        );
        let py = e.options.iter().find(|o| o.kind == "pypi").unwrap();
        assert_eq!(py.args, vec!["acme-weather==1.2.0"]);
        // Zero required inputs sorts the Python package before npm.
        assert_eq!(e.options[0].kind, "pypi");
        let oci = e.options.iter().find(|o| o.kind == "oci").unwrap();
        assert_eq!(oci.package, "acme/weather");
        assert_eq!(
            oci.args,
            vec![
                "run",
                "-i",
                "--rm",
                "-e",
                "WEATHER_KEY",
                "acme/weather:1.2.0"
            ]
        );
        assert!(
            !e.options
                .iter()
                .find(|o| o.kind == "nuget")
                .unwrap()
                .supported
        );
        let remotes: Vec<_> = e.options.iter().filter(|o| o.kind == "remote").collect();
        assert_eq!(remotes.len(), 2);
        assert!(
            remotes
                .iter()
                .any(|o| o.supported && o.url.ends_with("/mcp"))
        );
        assert!(
            remotes
                .iter()
                .any(|o| !o.supported && o.reason.contains("SSE"))
        );

        let entry = |id: &str| McpEntry {
            id: id.into(),
            name: mcp_name(id),
            title: String::new(),
            description: String::new(),
            version: String::new(),
            repository: String::new(),
            website: String::new(),
            options: Vec::new(),
        };
        let mut list = [
            entry("ai.smithery/smithery-notion"),
            entry("dev.wirecat/notion-cli"),
            entry("io.github.someone/better-things"),
            entry("io.github.ai/notion-mcp-server"),
            entry("com.notion/mcp"),
        ];
        list.sort_by_key(|e| rank(e, "Notion"));
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "com.notion/mcp",
                "io.github.ai/notion-mcp-server",
                "ai.smithery/smithery-notion",
                "dev.wirecat/notion-cli",
                "io.github.someone/better-things"
            ]
        );
        assert_eq!(
            rank(&entry("io.github.github/github-mcp-server"), "github").0,
            0
        );

        // Optional values inside arguments, like the GitHub server's token.
        let gh = json!({ "server": {
            "name": "io.github.github/github-mcp-server",
            "packages": [{
                "registryType": "oci", "identifier": "ghcr.io/github/github-mcp-server:2.3.0",
                "transport": { "type": "stdio" },
                "runtimeArguments": [
                    { "type": "named", "name": "-e", "value": "PORT=8085" },
                    { "type": "named", "name": "-e", "value": "TOKEN={token}",
                      "description": "Optional token",
                      "variables": { "token": { "isSecret": true } } }
                ]
            }],
            "remotes": [{ "type": "streamable-http", "url": "https://api.example/mcp/",
                "headers": [{ "name": "Authorization", "isSecret": true }] }]
        } });
        let e = mcp_entry(&gh).unwrap();
        let oci = e.options.iter().find(|o| o.kind == "oci").unwrap();
        assert_eq!(
            oci.args,
            vec![
                "run",
                "-i",
                "--rm",
                "-e",
                "PORT=8085",
                "-e",
                "TOKEN={token}",
                "ghcr.io/github/github-mcp-server:2.3.0"
            ]
        );
        assert_eq!(oci.package, "ghcr.io/github/github-mcp-server");
        let token = &oci.inputs[0];
        assert_eq!(
            (token.name.as_str(), token.secret, token.required),
            ("token", true, false)
        );
        assert_eq!(token.template, "TOKEN={token}");
        assert_eq!((token.arg_index, token.drop_index), (Some(6), Some(5)));
        let remote = e.options.iter().find(|o| o.kind == "remote").unwrap();
        assert_eq!(remote.inputs[0].key, "Authorization");
        assert_eq!(remote.inputs[0].default, "Bearer ");

        let deleted = json!({ "server": { "name": "a/b" },
            "_meta": { "io.modelcontextprotocol.registry/official": { "status": "deleted" } } });
        assert!(mcp_entry(&deleted).is_none());
    }
}
