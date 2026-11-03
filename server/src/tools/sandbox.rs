//! Path confinement. Every path the model supplies is resolved against the
//! project folder, canonicalized with dunce (which resolves symlinks and
//! junctions) and rejected unless it stays inside an allowed root.

use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone)]
pub struct Sandbox {
    /// Canonical project folder (read/write).
    pub root: PathBuf,
    /// Canonical skill folders (read-only).
    pub read_only: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Access {
    Read,
    Write,
}

impl Sandbox {
    pub fn new(root: &Path, read_only: &[PathBuf]) -> Result<Self> {
        let root = dunce::canonicalize(root)
            .map_err(|e| anyhow!("project folder {}: {e}", root.display()))?;
        let read_only = read_only
            .iter()
            .filter_map(|p| dunce::canonicalize(p).ok())
            .collect();
        Ok(Self { root, read_only })
    }

    fn allowed(&self, p: &Path, access: Access) -> bool {
        if p.starts_with(&self.root) {
            return true;
        }
        access == Access::Read && self.read_only.iter().any(|r| p.starts_with(r))
    }

    /// Resolves a path that must already exist.
    pub fn resolve_existing(&self, input: &str, access: Access) -> Result<PathBuf> {
        let joined = self.join(input)?;
        let canon = dunce::canonicalize(&joined).map_err(|e| anyhow!("{input}: {e}"))?;
        if !self.allowed(&canon, access) {
            bail!("access denied: {input} is outside the project folder");
        }
        Ok(canon)
    }

    /// Resolves a path that may not exist yet (for writes). The deepest
    /// existing ancestor is canonicalized (following any symlinks) and must be
    /// inside the project; the remaining components may not contain `..`.
    pub fn resolve_for_write(&self, input: &str) -> Result<PathBuf> {
        let joined = self.join(input)?;
        if joined.exists() {
            let canon = dunce::canonicalize(&joined).map_err(|e| anyhow!("{input}: {e}"))?;
            if !self.allowed(&canon, Access::Write) {
                bail!("access denied: {input} is outside the project folder");
            }
            if canon.is_dir() {
                bail!("{input} is a directory");
            }
            return Ok(canon);
        }
        let mut existing = joined.as_path();
        let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
        loop {
            if existing.exists() {
                break;
            }
            match (existing.file_name(), existing.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name);
                    existing = parent;
                }
                _ => bail!("invalid path: {input}"),
            }
        }
        let mut canon = dunce::canonicalize(existing).map_err(|e| anyhow!("{input}: {e}"))?;
        if !self.allowed(&canon, Access::Write) {
            bail!("access denied: {input} is outside the project folder");
        }
        for part in rest.iter().rev() {
            canon.push(part);
        }
        if !canon.starts_with(&self.root) {
            bail!("access denied: {input} is outside the project folder");
        }
        Ok(canon)
    }

    fn join(&self, input: &str) -> Result<PathBuf> {
        let input = input.trim();
        if input.is_empty() || input == "." {
            return Ok(self.root.clone());
        }
        let p = Path::new(input);
        // Reject Windows device paths and alternate data streams outright.
        if input.starts_with(r"\\?\") || input.starts_with(r"\\.\") {
            bail!("device paths are not allowed");
        }
        let without_drive = if input.len() >= 2 && input.as_bytes()[1] == b':' {
            &input[2..]
        } else {
            input
        };
        if without_drive.contains(':') {
            bail!("invalid path: {input}");
        }
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };
        // Lexically normalize so that "a/../../x" can't sneak past a missing
        // intermediate directory; canonicalize then handles symlinks.
        let mut out = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::ParentDir => {
                    if !out.pop() {
                        bail!("invalid path: {input}");
                    }
                }
                Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        Ok(out)
    }

    /// Path shown back to the model: relative to the project when possible.
    pub fn display(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => p.to_string_lossy().into_owned(),
        }
    }

    /// True if `p` (lexically normalized, absolute) is inside the project.
    pub fn contains_lexical(&self, input: &str) -> bool {
        match self.join(input) {
            Ok(p) => {
                // Use canonical form when the path exists (follows symlinks).
                let p = dunce::canonicalize(&p).unwrap_or(p);
                p.starts_with(&self.root)
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("relay-test-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn confines_paths() {
        let base = tmp("sb");
        let proj = base.join("proj");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::write(proj.join("src/a.txt"), "x").unwrap();
        std::fs::write(base.join("secret.txt"), "s").unwrap();
        let sb = Sandbox::new(&proj, &[]).unwrap();

        assert!(sb.resolve_existing("src/a.txt", Access::Read).is_ok());
        assert!(
            sb.resolve_existing("src/../src/a.txt", Access::Read)
                .is_ok()
        );
        assert!(sb.resolve_existing("../secret.txt", Access::Read).is_err());
        assert!(
            sb.resolve_existing(base.join("secret.txt").to_str().unwrap(), Access::Read)
                .is_err()
        );
        assert!(sb.resolve_for_write("new/dir/file.txt").is_ok());
        assert!(sb.resolve_for_write("new/../../escape.txt").is_err());
        assert!(sb.resolve_for_write("../escape.txt").is_err());
        assert!(sb.contains_lexical("src"));
        assert!(!sb.contains_lexical(".."));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let base = tmp("sl");
        let proj = base.join("proj");
        let outside = base.join("outside");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("s.txt"), "s").unwrap();
        std::os::unix::fs::symlink(&outside, proj.join("link")).unwrap();
        let sb = Sandbox::new(&proj, &[]).unwrap();
        assert!(sb.resolve_existing("link/s.txt", Access::Read).is_err());
        assert!(sb.resolve_for_write("link/new.txt").is_err());
        assert!(sb.resolve_for_write("link/s.txt").is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn read_only_roots() {
        let base = tmp("ro");
        let proj = base.join("proj");
        let skills = base.join("skills");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(skills.join("SKILL.md"), "x").unwrap();
        let sb = Sandbox::new(&proj, std::slice::from_ref(&skills)).unwrap();
        let p = skills.join("SKILL.md");
        assert!(
            sb.resolve_existing(p.to_str().unwrap(), Access::Read)
                .is_ok()
        );
        assert!(
            sb.resolve_existing(p.to_str().unwrap(), Access::Write)
                .is_err()
        );
        assert!(sb.resolve_for_write(p.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
