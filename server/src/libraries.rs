//! Model libraries: folders on any disk where Relay keeps models, like game
//! libraries in Steam. The built-in one is `models` in the data folder; the
//! user can add more (e.g. on a bigger disk), choose which one new downloads
//! go to, and move models between them.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::setup::ModelFile;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Disk {
    /// Drive or mount the library is on ("C:\" on Windows).
    pub root: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// "local", "network", "removable" or "unknown".
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub path: String,
    /// Relay's own folder in its data directory; can't be removed.
    pub builtin: bool,
    /// New downloads and copies go here.
    pub default: bool,
    pub exists: bool,
    pub disk: Option<Disk>,
    pub models: Vec<ModelFile>,
    /// Total size of the models in it (all parts of split models).
    pub size_bytes: u64,
}

/// Folders of other apps Relay must never take over as a library, since
/// moving models out of one would remove that app's files.
pub fn is_other_apps_folder(path: &Path) -> bool {
    let s = path.to_string_lossy().to_lowercase().replace('\\', "/");
    [
        "/.lmstudio",
        "/.ollama",
        "/lm studio/",
        "/.cache/huggingface",
        "/.cache/lm-studio",
    ]
    .iter()
    .any(|m| s.contains(m) || s.ends_with(m.trim_end_matches('/')))
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let c = dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = c.to_string_lossy().replace('\\', "/");
        let s = s.trim_end_matches('/').to_string();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    norm(a) == norm(b)
}

/// True when `file` is inside `dir`.
pub fn contains(dir: &Path, file: &Path) -> bool {
    let d = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let f = dunce::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    if cfg!(windows) {
        let ds = d.to_string_lossy().to_lowercase().replace('\\', "/");
        let fs = f.to_string_lossy().to_lowercase().replace('\\', "/");
        fs.starts_with(&format!("{}/", ds.trim_end_matches('/')))
    } else {
        f.starts_with(&d) && f != d
    }
}

/// Every file of a model: just the file, or all parts of a split model
/// ("-00001-of-00003.gguf" and its siblings).
pub fn model_parts(first: &Path) -> Vec<PathBuf> {
    let Some(name) = first.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return vec![first.to_path_buf()];
    };
    let lower = name.to_lowercase();
    let Some(of) = lower.find("-of-") else {
        return vec![first.to_path_buf()];
    };
    // "<stem>-00001-of-00003.gguf": the part number is the 5 digits before "-of-".
    if of < 5 || !lower[..of].ends_with("00001") {
        return vec![first.to_path_buf()];
    }
    let stem = &name[..of - 5];
    let suffix = &name[of..];
    let Some(dir) = first.parent() else {
        return vec![first.to_path_buf()];
    };
    let mut parts = vec![first.to_path_buf()];
    for n in 2..=999u32 {
        let p = dir.join(format!("{stem}{n:05}{suffix}"));
        if !p.is_file() {
            break;
        }
        parts.push(p);
    }
    parts
}

pub fn model_size(first: &Path) -> u64 {
    model_parts(first)
        .iter()
        .map(|p| p.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}

/// Lists the libraries with their disks and models (blocking: walks folders).
pub fn list(builtin: &Path, default: &Path, extra: &[PathBuf]) -> Vec<Library> {
    let mut paths: Vec<PathBuf> = vec![builtin.to_path_buf()];
    for p in std::iter::once(default.to_path_buf()).chain(extra.iter().cloned()) {
        if !paths.iter().any(|q| same_path(q, &p)) {
            paths.push(p);
        }
    }
    paths
        .into_iter()
        .map(|p| {
            let mut models = crate::setup::discover_models(std::slice::from_ref(&p));
            for m in &mut models {
                m.size_bytes = model_size(Path::new(&m.path));
            }
            Library {
                path: p.to_string_lossy().into_owned(),
                builtin: same_path(&p, builtin),
                default: same_path(&p, default),
                exists: p.is_dir(),
                disk: disk(&p),
                size_bytes: models.iter().map(|m| m.size_bytes).sum(),
                models,
            }
        })
        .collect()
}

/// The nearest existing folder at or above `path` (a new library may not
/// exist yet).
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut p = Some(path);
    while let Some(q) = p {
        if q.exists() {
            return Some(q.to_path_buf());
        }
        p = q.parent();
    }
    None
}

#[cfg(windows)]
pub fn disk(path: &Path) -> Option<Disk> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumePathNameW,
    };
    let at = existing_ancestor(path)?;
    let wide: Vec<u16> = at.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = vec![0u16; 1024];
    // SAFETY: both buffers are valid, NUL-terminated and sized as passed.
    let ok = unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) };
    if ok == 0 {
        return None;
    }
    let (mut free, mut total, mut _all_free) = (0u64, 0u64, 0u64);
    // SAFETY: `root` is a NUL-terminated volume path; out-params are valid.
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, &mut _all_free) };
    if ok == 0 {
        return None;
    }
    // SAFETY: `root` is a NUL-terminated volume path.
    let kind = match unsafe { GetDriveTypeW(root.as_ptr()) } {
        2 => "removable",
        3 => "local",
        4 => "network",
        _ => "unknown",
    };
    let end = root.iter().position(|&c| c == 0).unwrap_or(root.len());
    Some(Disk {
        root: String::from_utf16_lossy(&root[..end]),
        total_bytes: total,
        free_bytes: free,
        kind,
    })
}

#[cfg(unix)]
pub fn disk(path: &Path) -> Option<Disk> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let at = existing_ancestor(path)?;
    let c = CString::new(at.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is plain old data; `c` is a valid NUL-terminated path.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    // The mount point: climb while the device stays the same.
    let dev = at.metadata().ok()?.dev();
    let mut root = at.clone();
    while let Some(parent) = root.parent() {
        match parent.metadata() {
            Ok(m) if m.dev() == dev => root = parent.to_path_buf(),
            _ => break,
        }
    }
    let frsize = st.f_frsize as u64;
    Some(Disk {
        root: root.to_string_lossy().into_owned(),
        total_bytes: st.f_blocks as u64 * frsize,
        free_bytes: st.f_bavail as u64 * frsize,
        kind: "local",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_split_parts() {
        let dir = std::env::temp_dir().join(format!("relay-lib-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        for n in 1..=3 {
            std::fs::write(dir.join(format!("M-Q8_0-{n:05}-of-00003.gguf")), [0u8; 10])
                .expect("write");
        }
        std::fs::write(dir.join("single.gguf"), [0u8; 4]).expect("write");
        let first = dir.join("M-Q8_0-00001-of-00003.gguf");
        assert_eq!(model_parts(&first).len(), 3);
        assert_eq!(model_size(&first), 30);
        assert_eq!(model_parts(&dir.join("single.gguf")).len(), 1);
        assert!(contains(&dir, &first));
        assert!(!contains(&dir, &dir));
        assert!(disk(&dir.join("not-yet")).is_some_and(|d| d.total_bytes > 0));
        let libs = list(&dir, &dir, &[]);
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].models.len(), 2);
        assert_eq!(libs[0].size_bytes, 34);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_other_apps() {
        assert!(is_other_apps_folder(Path::new(
            r"C:\Users\w\.lmstudio\models"
        )));
        assert!(is_other_apps_folder(Path::new("/home/w/.ollama")));
        assert!(!is_other_apps_folder(Path::new(r"W:\Relay\models")));
    }
}
