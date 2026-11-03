//! UI mockups: the model writes a self-contained HTML or SVG block and the
//! phone shows it as a picture. Relay renders the picture with the PC's own
//! Edge or Chrome in headless mode (a throwaway profile), under a content
//! policy that blocks scripts and every network request, so a mockup can't
//! run code or reach anything.
//!
//! The desktop app doesn't need this: it shows the HTML live in a sandboxed
//! frame.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// Largest mockup accepted, in bytes.
pub const MAX_HTML: usize = 512 * 1024;
const DEFAULT_SIZE: (u32, u32) = (1280, 800);
const RENDER_TIMEOUT: Duration = Duration::from_secs(45);

/// Blocks all network access and scripts; only inline styles and data: images.
pub const CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:";

/// The page Relay renders: the model's HTML (or SVG) with the content policy
/// first in <head>, so it applies before anything else loads.
pub fn prepare(source: &str, svg: bool) -> String {
    let meta = format!(
        r#"<meta http-equiv="Content-Security-Policy" content="{CSP}"><meta charset="utf-8">"#
    );
    if svg {
        return format!(
            "<!doctype html><html><head>{meta}<style>html,body{{margin:0;background:#fff}}svg{{display:block;max-width:100%;height:auto}}</style></head><body>{source}</body></html>"
        );
    }
    let lower = source.to_ascii_lowercase();
    if let Some(at) = after_tag(&lower, "head") {
        return format!("{}{meta}{}", &source[..at], &source[at..]);
    }
    if let Some(at) = after_tag(&lower, "html") {
        return format!("{}<head>{meta}</head>{}", &source[..at], &source[at..]);
    }
    format!("<!doctype html><html><head>{meta}</head><body>{source}</body></html>")
}

/// The byte just after the first `<name>` or `<name ...>` opening tag
/// (not `<header>` when looking for `<head>`).
fn after_tag(lower: &str, name: &str) -> Option<usize> {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(i) = lower[from..].find(&open) {
        let start = from + i;
        let next = lower[start + open.len()..].chars().next();
        if matches!(next, Some(c) if c == '>' || c.is_ascii_whitespace()) {
            return lower[start..].find('>').map(|e| start + e + 1);
        }
        from = start + open.len();
    }
    None
}

/// The picture size: `<meta name="relay-size" content="390x844">` if the
/// mockup asks for one (a phone screen, say), otherwise a desktop window.
pub fn size_hint(source: &str) -> (u32, u32) {
    let lower = source.to_ascii_lowercase();
    let Some(i) = lower.find("relay-size") else {
        return DEFAULT_SIZE;
    };
    let rest = &lower[i..lower.len().min(i + 120)];
    let Some(c) = rest.find("content=") else {
        return DEFAULT_SIZE;
    };
    let value: String = rest[c + 8..]
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == 'x')
        .collect();
    let mut parts = value.split('x').map(|p| p.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(w), Some(h)) => (w.clamp(240, 2560), h.clamp(240, 4000)),
        _ => DEFAULT_SIZE,
    }
}

/// A stable name for a prepared page, so the same mockup renders once.
pub fn mockup_id(page: &str, size: (u32, u32)) -> String {
    let mut h = std::hash::DefaultHasher::new();
    page.hash(&mut h);
    size.hash(&mut h);
    format!("{:016x}{:08x}", h.finish(), page.len() as u32)
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Edge or Chrome on this PC. `RELAY_BROWSER` overrides the search.
pub fn find_browser() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RELAY_BROWSER").map(PathBuf::from)
        && p.is_file()
    {
        return Some(p);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
                candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
                candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            }
        }
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path) {
            for name in [
                "microsoft-edge",
                "google-chrome",
                "chromium",
                "chromium-browser",
            ] {
                candidates.push(dir.join(name));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Renders `page` to `<dir>/<id>.png` unless that picture already exists.
pub async fn render(dir: &Path, id: &str, page: &str, size: (u32, u32)) -> Result<PathBuf> {
    let png = dir.join(format!("{id}.png"));
    if png.is_file() {
        return Ok(png);
    }
    let Some(browser) = find_browser() else {
        bail!("Relay needs Microsoft Edge or Google Chrome on the PC to draw mockups");
    };
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    let html = dir.join(format!("{id}.html"));
    tokio::fs::write(&html, page).await?;
    let url = format!(
        "file:///{}",
        html.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let mut cmd = tokio::process::Command::new(&browser);
    cmd.arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .arg("--hide-scrollbars")
        .arg(format!("--window-size={},{}", size.0, size.1))
        .arg(format!("--screenshot={}", png.display()))
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(not(windows))]
    {
        // Linux containers often run as root, where Chrome's sandbox won't start.
        cmd.arg("--no-sandbox");
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", browser.display()))?;
    match tokio::time::timeout(RENDER_TIMEOUT, child.wait()).await {
        Ok(status) => {
            status?;
        }
        Err(_) => {
            let _ = child.kill().await;
            bail!("drawing the mockup took too long");
        }
    }
    let _ = tokio::fs::remove_file(&html).await;
    if !png.is_file() {
        bail!("the browser didn't produce a picture of the mockup");
    }
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puts_the_policy_first() {
        let p = prepare(
            "<html><head><title>x</title></head><body>hi</body></html>",
            false,
        );
        assert!(p.starts_with("<html><head><meta http-equiv=\"Content-Security-Policy\""));
        let p = prepare("<div>hi</div>", false);
        assert!(p.contains(CSP) && p.contains("<body><div>hi</div>"));
        let p = prepare("<HTML lang=en><body>x</body></HTML>", false);
        assert!(p.starts_with("<HTML lang=en><head><meta"));
        assert!(prepare("<svg></svg>", true).contains("<body><svg></svg>"));
        let p = prepare("<header>top</header>", false);
        assert!(p.contains("<body><header>top</header>"), "{p}");
    }

    #[test]
    fn reads_size_hints() {
        assert_eq!(size_hint("<p>x</p>"), DEFAULT_SIZE);
        assert_eq!(
            size_hint(r#"<meta name="relay-size" content="390x844">"#),
            (390, 844)
        );
        assert_eq!(
            size_hint(r#"<meta name="relay-size" content="10x99999">"#),
            (240, 4000)
        );
        assert_eq!(
            size_hint(r#"<meta name="relay-size" content="big">"#),
            DEFAULT_SIZE
        );
    }

    #[test]
    fn ids_are_stable_and_checked() {
        let a = mockup_id("<p>x</p>", (1, 2));
        assert_eq!(a, mockup_id("<p>x</p>", (1, 2)));
        assert_ne!(a, mockup_id("<p>y</p>", (1, 2)));
        assert!(valid_id(&a));
        assert!(!valid_id("../../etc/passwd"));
    }
}
