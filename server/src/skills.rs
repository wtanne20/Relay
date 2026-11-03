//! SKILL.md discovery. A skill is a folder containing SKILL.md whose YAML
//! frontmatter has `name` and `description`. Only the name, description and
//! path go into the system prompt; the model reads the full file on demand.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Absolute path to SKILL.md.
    pub path: PathBuf,
    /// Folder containing SKILL.md (granted read-only access).
    pub dir: PathBuf,
}

/// Each root may itself be a skill folder, or contain skill folders
/// (searched up to three levels deep).
pub fn discover(roots: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for root in roots {
        let root = dunce::canonicalize(root).unwrap_or_else(|_| root.clone());
        for dir in find_skill_dirs(&root, 3) {
            if let Some(skill) = load(&dir)
                && !out.iter().any(|s| s.dir == skill.dir)
            {
                out.push(skill);
            }
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Returns folders under `root` (including root) that contain SKILL.md.
pub fn find_skill_dirs(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, max_depth, &mut found);
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    if skill_file(dir).is_some() {
        found.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();
    for d in subdirs {
        walk(&d, depth - 1, found);
    }
}

fn skill_file(dir: &Path) -> Option<PathBuf> {
    ["SKILL.md", "skill.md", "Skill.md"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

fn load(dir: &Path) -> Option<Skill> {
    let path = skill_file(dir)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let (name, description) = parse_frontmatter(&text);
    let fallback = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "skill".into());
    Some(Skill {
        name: name.unwrap_or(fallback),
        description: description.unwrap_or_default(),
        path: dunce::canonicalize(&path).unwrap_or(path),
        dir: dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()),
    })
}

/// Minimal frontmatter parser for `name:` and `description:` keys, including
/// quoted values and YAML block scalars (`>` / `|`).
pub fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let body: Vec<&str> = lines.take_while(|l| l.trim() != "---").collect();
    let mut name = None;
    let mut description = None;
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        if line.starts_with(' ') || line.starts_with('\t') {
            i += 1;
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let mut value = value.trim().to_string();
            if value == ">" || value == "|" || value == ">-" || value == "|-" {
                let mut parts = Vec::new();
                i += 1;
                while i < body.len()
                    && (body[i].starts_with(' ')
                        || body[i].starts_with('\t')
                        || body[i].trim().is_empty())
                {
                    parts.push(body[i].trim());
                    i += 1;
                }
                value = parts.join(" ").trim().to_string();
            } else {
                i += 1;
            }
            let value = unquote(&value);
            match key {
                "name" => name = Some(value),
                "description" => description = Some(value),
                _ => {}
            }
        } else {
            i += 1;
        }
    }
    (
        name.filter(|s| !s.is_empty()),
        description.filter(|s| !s.is_empty()),
    )
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// The section appended to the system prompt.
pub fn prompt_section(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut s = String::from(
        "# Skills\nThese skills are available. When one is relevant, read its SKILL.md with read_file (using the exact path shown) before starting, and follow it. Skill folders are read-only.\n",
    );
    for sk in skills {
        s.push_str(&format!(
            "- {}: {} (path: {})\n",
            sk.name,
            sk.description,
            sk.path.display()
        ));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter() {
        let (n, d) =
            parse_frontmatter("---\nname: pdf\ndescription: \"Work with PDFs\"\n---\nbody");
        assert_eq!(n.as_deref(), Some("pdf"));
        assert_eq!(d.as_deref(), Some("Work with PDFs"));
        let (n, d) =
            parse_frontmatter("---\nname: x\ndescription: >\n  multi\n  line\nlicense: MIT\n---\n");
        assert_eq!(n.as_deref(), Some("x"));
        assert_eq!(d.as_deref(), Some("multi line"));
        assert_eq!(parse_frontmatter("no frontmatter"), (None, None));
    }

    #[test]
    fn discovers_nested() {
        let base = std::env::temp_dir().join(format!("relay-skills-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("a")).unwrap();
        std::fs::create_dir_all(base.join("group/b")).unwrap();
        std::fs::write(
            base.join("a/SKILL.md"),
            "---\nname: alpha\ndescription: A\n---\n",
        )
        .unwrap();
        std::fs::write(base.join("group/b/SKILL.md"), "---\ndescription: B\n---\n").unwrap();
        let skills = discover(&[base.clone(), base.join("a")]);
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].name, "b");
        std::fs::remove_dir_all(base).unwrap();
    }
}
