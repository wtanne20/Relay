//! Files other than pictures attached to prompts: code, text, PDFs, Word
//! documents and anything else. Each is stored in `<data>/attachments/`
//! under a name Relay makes. The text Relay could read from it is kept next
//! to it as `<name>.txt` and given to the model with the message, so any
//! model can use it (no vision needed).

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

/// Biggest file accepted.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;
/// Files per prompt.
pub const MAX_PER_PROMPT: usize = 8;
/// Most text given to the model from one file.
pub const MAX_TEXT: usize = 100_000;

/// A stored attachment, as kept with the message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FileRef {
    /// The name it was sent with (cleaned up).
    pub name: String,
    /// The stored file in the attachments folder.
    pub stored: String,
    pub mime: String,
    pub size: u64,
    /// How the model gets it: "text", "pdf", "docx" (its text is given to
    /// the model) or "other" (only described).
    pub kind: String,
    /// Characters of text read from it; 0 when none could be read.
    #[serde(default)]
    pub chars: usize,
}

/// An attachment as the app sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct NewFile {
    pub name: String,
    /// A data URL, or bare base64.
    pub data: String,
}

/// A safe display name: no folders, no control characters, not too long.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let s: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    if s.is_empty() { "file".into() } else { s }
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".into())
}

/// Extensions read as plain text even if the app didn't say text/*.
const TEXT_EXTS: &[&str] = &[
    "txt",
    "md",
    "markdown",
    "rst",
    "csv",
    "tsv",
    "json",
    "jsonl",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "env",
    "log",
    "xml",
    "html",
    "htm",
    "css",
    "scss",
    "less",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "py",
    "rb",
    "rs",
    "go",
    "java",
    "kt",
    "kts",
    "swift",
    "c",
    "h",
    "cc",
    "cpp",
    "hpp",
    "cs",
    "php",
    "pl",
    "lua",
    "r",
    "sql",
    "sh",
    "bash",
    "zsh",
    "ps1",
    "bat",
    "cmd",
    "gradle",
    "dart",
    "vue",
    "svelte",
    "tex",
    "srt",
    "vtt",
    "gitignore",
    "dockerfile",
    "makefile",
    "lock",
    "properties",
    "proto",
    "graphql",
    "gql",
    "ipynb",
    "svg",
];

fn mime_for(ext: &str) -> &'static str {
    match ext {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "json" => "application/json",
        "csv" => "text/csv",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        _ if TEXT_EXTS.contains(&ext) => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Text from bytes that look like text: valid UTF-8 (or close) with no NULs.
fn as_text(bytes: &[u8]) -> Option<String> {
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.contains(&0) {
        return None;
    }
    let s = String::from_utf8_lossy(bytes);
    let bad = s.chars().take(8192).filter(|&c| c == '\u{FFFD}').count();
    (bad * 50 < s.chars().take(8192).count().max(1)).then(|| s.into_owned())
}

fn pdf_text(bytes: &[u8]) -> Option<String> {
    // The PDF reader can panic on unusual files; that only means no text.
    let bytes = bytes.to_vec();
    std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&bytes).ok())
        .ok()
        .flatten()
}

fn docx_text(bytes: &[u8]) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .ok()?
        .take(64 * 1024 * 1024)
        .read_to_string(&mut xml)
        .ok()?;
    Some(xml_text(&xml))
}

/// The text of a Word document's XML: paragraphs become lines.
fn xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        if tag.starts_with("/w:p") && !tag.starts_with("/w:pPr") || tag.starts_with("w:br") {
            out.push('\n');
        } else if tag.starts_with("w:tab") && !tag.starts_with("w:tabs") {
            out.push('\t');
        }
        rest = &rest[open + close + 1..];
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn decode(data: &str) -> Result<Vec<u8>> {
    let b64 = match data.split_once(',') {
        Some((head, rest)) if head.starts_with("data:") => {
            if !head.ends_with(";base64") {
                bail!("the file isn't base64-encoded");
            }
            rest
        }
        _ => data,
    };
    let b64: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    if b64.len() / 4 * 3 > MAX_BYTES + 3 {
        bail!("the file is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    STANDARD
        .decode(b64.as_bytes())
        .map_err(|_| anyhow!("the file data is damaged"))
}

/// Stores a file and the text Relay can read from it.
pub fn save(dir: &Path, f: &NewFile) -> Result<FileRef> {
    let name = clean_name(&f.name);
    let bytes = decode(&f.data).with_context(|| name.clone())?;
    if bytes.len() > MAX_BYTES {
        bail!("{name} is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    let ext = extension(&name);
    let (kind, text) = match ext.as_str() {
        "pdf" => ("pdf", pdf_text(&bytes)),
        "docx" => ("docx", docx_text(&bytes)),
        _ => match as_text(&bytes) {
            Some(t) => ("text", Some(t)),
            None => ("other", None),
        },
    };
    let text = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let stored = format!("{}.{ext}", uuid::Uuid::new_v4().simple());
    std::fs::write(dir.join(&stored), &bytes).context("saving the file")?;
    let chars = text.as_ref().map(|t| t.chars().count()).unwrap_or(0);
    if let Some(t) = &text {
        std::fs::write(dir.join(format!("{stored}.txt")), t).context("saving the file's text")?;
    }
    Ok(FileRef {
        name,
        stored,
        mime: mime_for(&ext).to_string(),
        size: bytes.len() as u64,
        kind: kind.to_string(),
        chars,
    })
}

/// Only names Relay made: 32 hex digits and a short extension.
pub fn valid_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else {
        return false;
    };
    stem.len() == 32
        && stem.bytes().all(|b| b.is_ascii_hexdigit())
        && !ext.is_empty()
        && ext.len() <= 8
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub fn mime(name: &str) -> &'static str {
    mime_for(&extension(name))
}

/// The stored text of a file, if any.
pub fn text(dir: &Path, f: &FileRef) -> Option<String> {
    if !valid_name(&f.stored) || f.chars == 0 {
        return None;
    }
    std::fs::read_to_string(dir.join(format!("{}.txt", f.stored))).ok()
}

fn human_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)
    } else if n >= 1024 {
        format!("{} KB", n.div_ceil(1024))
    } else {
        format!("{n} bytes")
    }
}

/// What the model is told about a file: its text in a fenced block, or a
/// note saying what it is when there's no text to give.
pub fn for_model(f: &FileRef, text: Option<&str>) -> String {
    let head = format!(
        "Attached file: {} ({}, {})",
        f.name,
        f.mime,
        human_size(f.size)
    );
    let Some(t) = text else {
        let why = match f.kind.as_str() {
            "pdf" => "Relay couldn't read any text from this PDF (it may be scanned pages).",
            "docx" => "Relay couldn't read any text from this document.",
            _ => "It isn't a text file, so its contents can't be shown to you.",
        };
        return format!("{head}\n{why}");
    };
    let (body, cut) = if t.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        (&t[..end], true)
    } else {
        (t, false)
    };
    let fence = if body.contains("```") { "~~~~" } else { "```" };
    let lang = match extension(&f.name).as_str() {
        "pdf" | "docx" | "txt" | "bin" => String::new(),
        e => e.to_string(),
    };
    let mut s = format!("{head}\n{fence}{lang}\n{body}\n{fence}");
    if cut {
        s.push_str(&format!(
            "\n[The file is longer: only the first {} KB of its text is shown.]",
            MAX_TEXT / 1024
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(bytes: &[u8]) -> String {
        format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(bytes)
        )
    }

    #[test]
    fn stores_text_and_describes_binaries() {
        let dir =
            std::env::temp_dir().join(format!("relay-files-{}", uuid::Uuid::new_v4().simple()));
        let code = save(
            &dir,
            &NewFile {
                name: "../src/main.rs".into(),
                data: data(b"fn main() {}\n"),
            },
        )
        .unwrap();
        assert_eq!(
            (code.name.as_str(), code.kind.as_str()),
            ("main.rs", "text")
        );
        assert!(valid_name(&code.stored) && code.stored.ends_with(".rs"));
        let t = text(&dir, &code).unwrap();
        assert_eq!(t, "fn main() {}");
        let shown = for_model(&code, Some(&t));
        assert!(
            shown.starts_with(
                "Attached file: main.rs (text/plain, 13 bytes)\n```rs\nfn main() {}\n```"
            ),
            "{shown}"
        );

        let bin = save(
            &dir,
            &NewFile {
                name: "blob.dat".into(),
                data: data(&[0, 1, 2, 255]),
            },
        )
        .unwrap();
        assert_eq!((bin.kind.as_str(), bin.chars), ("other", 0));
        assert!(text(&dir, &bin).is_none());
        assert!(for_model(&bin, None).contains("isn't a text file"));

        let bad = save(
            &dir,
            &NewFile {
                name: "a.pdf".into(),
                data: data(b"not a pdf"),
            },
        )
        .unwrap();
        assert_eq!((bad.kind.as_str(), bad.chars), ("pdf", 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_word_xml() {
        let xml = r#"<w:document><w:body><w:p><w:pPr/><w:r><w:t>Hello &amp; welcome</w:t></w:r></w:p><w:p><w:r><w:t>Line</w:t><w:tab/><w:t>two</w:t></w:r></w:p></w:body></w:document>"#;
        assert_eq!(xml_text(xml), "Hello & welcome\nLine\ttwo\n");
    }

    #[test]
    fn long_text_is_cut() {
        let f = FileRef {
            name: "big.txt".into(),
            mime: "text/plain".into(),
            size: 1,
            kind: "text".into(),
            ..Default::default()
        };
        let t = "a".repeat(MAX_TEXT + 10);
        let s = for_model(&f, Some(&t));
        assert!(s.contains("only the first"));
        assert!(s.len() < MAX_TEXT + 300);
        assert!(!valid_name("../x.txt") && !valid_name("abc.png"));
    }
}
