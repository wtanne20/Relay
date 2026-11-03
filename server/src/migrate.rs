//! `relay migrate`: import projects, skills, system prompt and model choice
//! from an LM Studio (Bionic) setup. Never deletes, moves or modifies the
//! original files. Bionic chat history is not migrated.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::config::LoadedConfig;
use crate::db;
use crate::skills;

#[derive(Debug, Args, Default)]
pub struct MigrateArgs {
    /// Project folders to import (each becomes a project named after the folder).
    pub projects: Vec<PathBuf>,
    /// Parent folder to scan; asks for confirmation for each subfolder.
    #[arg(long)]
    pub scan: Option<PathBuf>,
    /// Extra folders to search for SKILL.md skills (in addition to <lmstudio>/skills).
    #[arg(long = "skills")]
    pub skills: Vec<PathBuf>,
    /// Text file containing your Bionic system prompt; becomes system_prompt.md.
    #[arg(long)]
    pub system_prompt: Option<PathBuf>,
    /// LM Studio folder (default: %USERPROFILE%\.lmstudio).
    #[arg(long)]
    pub lmstudio: Option<PathBuf>,
    /// llama.cpp folder or llama-server.exe path to store in config.toml.
    #[arg(long)]
    pub llama_server: Option<PathBuf>,
    /// Answer yes to every scan confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Skip the interactive model picker.
    #[arg(long)]
    pub skip_model: bool,
}

#[derive(Default)]
struct Summary {
    projects_added: Vec<String>,
    projects_skipped: Vec<String>,
    skills_added: Vec<String>,
    skills_known: usize,
    system_prompt: Option<String>,
    model: Option<String>,
    llama_server: Option<String>,
    warnings: Vec<String>,
}

pub fn run(cfg_path: &Path, args: MigrateArgs) -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    run_with_io(cfg_path, args, &mut input, &mut out)
}

pub fn run_with_io(
    cfg_path: &Path,
    args: MigrateArgs,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<()> {
    let mut loaded = LoadedConfig::load(cfg_path)?;
    let conn = db::open_connection(&loaded.db_path())?;
    let mut summary = Summary::default();
    let lmstudio = args.lmstudio.clone().or_else(default_lmstudio_dir);

    writeln!(out, "Relay migration (config: {})", loaded.path.display())?;
    writeln!(
        out,
        "Original files are never deleted, moved or modified. Chat history is not migrated.\n"
    )?;

    // 1. Projects
    let mut candidates: Vec<PathBuf> = args.projects.clone();
    if let Some(parent) = &args.scan {
        let parent = dunce::canonicalize(parent)
            .with_context(|| format!("scan folder {}", parent.display()))?;
        let mut subdirs: Vec<PathBuf> = std::fs::read_dir(&parent)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter(|p| {
                !p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            })
            .collect();
        subdirs.sort();
        writeln!(
            out,
            "Scanning {} ({} folders)",
            parent.display(),
            subdirs.len()
        )?;
        for d in subdirs {
            if args.yes || confirm(input, out, &format!("  Import {} ?", d.display()))? {
                candidates.push(d);
            }
        }
    }
    for p in candidates {
        match import_project(&conn, &p) {
            Ok(Some(name)) => summary.projects_added.push(name),
            Ok(None) => summary
                .projects_skipped
                .push(format!("{} (already a project)", p.display())),
            Err(e) => summary
                .warnings
                .push(format!("project {}: {e:#}", p.display())),
        }
    }

    // 2. Skills (referenced in place, never copied)
    let mut skill_search: Vec<PathBuf> = Vec::new();
    if let Some(lm) = &lmstudio {
        let d = lm.join("skills");
        if d.is_dir() {
            skill_search.push(d);
        }
    }
    skill_search.extend(args.skills.iter().cloned());
    let existing: Vec<PathBuf> = loaded
        .skill_roots()
        .iter()
        .map(|p| dunce::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();
    for root in &skill_search {
        if !root.exists() {
            summary
                .warnings
                .push(format!("skills folder not found: {}", root.display()));
            continue;
        }
        for dir in skills::find_skill_dirs(root, 3) {
            let canon = dunce::canonicalize(&dir).unwrap_or(dir);
            let covered = existing.iter().any(|e| canon.starts_with(e))
                || summary.skills_added.iter().any(|s| Path::new(s) == canon);
            if covered {
                summary.skills_known += 1;
                continue;
            }
            summary
                .skills_added
                .push(canon.to_string_lossy().into_owned());
        }
    }
    loaded
        .config
        .skills_paths
        .extend(summary.skills_added.iter().cloned());

    // 3. System prompt
    if let Some(src) = &args.system_prompt {
        let text =
            std::fs::read_to_string(src).with_context(|| format!("reading {}", src.display()))?;
        if text.trim().is_empty() {
            summary.warnings.push(format!(
                "{} is empty; system prompt unchanged",
                src.display()
            ));
        } else {
            let dest = loaded.system_prompt_path();
            if dest.exists() {
                let backup = dest.with_extension("md.bak");
                std::fs::copy(&dest, &backup)?;
                writeln!(
                    out,
                    "Backed up existing system prompt to {}",
                    backup.display()
                )?;
            }
            std::fs::write(&dest, text)?;
            summary.system_prompt = Some(format!("{} -> {}", src.display(), dest.display()));
        }
    }

    // 4. llama-server and model
    if let Some(ls) = &args.llama_server {
        let canon = dunce::canonicalize(ls)
            .with_context(|| format!("llama-server path {}", ls.display()))?;
        loaded.config.llama.llama_server_path = canon.to_string_lossy().into_owned();
        let exe = loaded.config.llama.executable();
        if !exe.is_file() {
            summary.warnings.push(format!(
                "llama-server executable not found at {}",
                exe.display()
            ));
        }
        summary.llama_server = Some(loaded.config.llama.llama_server_path.clone());
    }
    if !args.skip_model {
        match &lmstudio {
            Some(lm) if lm.join("models").is_dir() => {
                let mut models = Vec::new();
                find_ggufs(&lm.join("models"), 6, &mut models);
                models.retain(|(p, _)| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("mmproj"))
                });
                models.sort();
                if models.is_empty() {
                    summary.warnings.push(format!(
                        "no .gguf files under {}",
                        lm.join("models").display()
                    ));
                } else {
                    writeln!(out, "\nGGUF models under {}:", lm.join("models").display())?;
                    for (i, (p, size)) in models.iter().enumerate() {
                        writeln!(
                            out,
                            "  [{}] {}  ({})",
                            i + 1,
                            p.display(),
                            human_size(*size)
                        )?;
                    }
                    let current = &loaded.config.llama.model_path;
                    let prompt = if current.is_empty() {
                        "Pick a model number (Enter to skip): ".to_string()
                    } else {
                        format!("Pick a model number (Enter to keep {current}): ")
                    };
                    if let Some(i) = pick(input, out, &prompt, models.len())? {
                        loaded.config.llama.model_path = models[i].0.to_string_lossy().into_owned();
                        summary.model = Some(loaded.config.llama.model_path.clone());
                    }
                }
            }
            _ => summary
                .warnings
                .push("LM Studio models folder not found; model_path unchanged".into()),
        }
    }

    loaded.save()?;

    // 5. Summary
    writeln!(out, "\n=== Migration summary ===")?;
    writeln!(out, "Projects added: {}", summary.projects_added.len())?;
    for p in &summary.projects_added {
        writeln!(out, "  + {p}")?;
    }
    for p in &summary.projects_skipped {
        writeln!(out, "  = {p}")?;
    }
    writeln!(
        out,
        "Skill folders added to config.toml: {}",
        summary.skills_added.len()
    )?;
    for s in &summary.skills_added {
        writeln!(out, "  + {s}")?;
    }
    if summary.skills_known > 0 {
        writeln!(out, "  ({} already configured)", summary.skills_known)?;
    }
    writeln!(
        out,
        "System prompt: {}",
        summary.system_prompt.as_deref().unwrap_or("unchanged")
    )?;
    writeln!(
        out,
        "llama-server: {}",
        summary
            .llama_server
            .as_deref()
            .unwrap_or(&display_or_unset(&loaded.config.llama.llama_server_path))
    )?;
    writeln!(
        out,
        "Model: {}",
        summary
            .model
            .as_deref()
            .unwrap_or(&display_or_unset(&loaded.config.llama.model_path))
    )?;
    for w in &summary.warnings {
        writeln!(out, "Warning: {w}")?;
    }
    writeln!(out, "Config saved to {}", loaded.path.display())?;
    Ok(())
}

fn display_or_unset(s: &str) -> String {
    if s.is_empty() {
        "(not set)".into()
    } else {
        format!("{s} (unchanged)")
    }
}

fn import_project(conn: &rusqlite::Connection, p: &Path) -> Result<Option<String>> {
    if !p.is_dir() {
        bail!("not a folder");
    }
    let canon = dunce::canonicalize(p)?;
    let path = canon.to_string_lossy().into_owned();
    if db::find_project_by_path(conn, &path)?.is_some() {
        return Ok(None);
    }
    let name = canon
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    db::create_project(conn, &name, &path)?;
    Ok(Some(format!("{name} ({path})")))
}

fn default_lmstudio_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".lmstudio"))
}

fn find_ggufs(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                find_ggufs(&p, depth - 1, out);
            }
        } else if p
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("gguf"))
        {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((p, size));
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / MB)
    }
}

fn read_line(input: &mut dyn BufRead) -> Result<Option<String>> {
    let mut s = String::new();
    if input.read_line(&mut s)? == 0 {
        return Ok(None);
    }
    Ok(Some(s.trim().to_string()))
}

fn confirm(input: &mut dyn BufRead, out: &mut dyn Write, q: &str) -> Result<bool> {
    write!(out, "{q} [y/N] ")?;
    out.flush()?;
    Ok(matches!(
        read_line(input)?.as_deref(),
        Some("y" | "Y" | "yes" | "Yes")
    ))
}

fn pick(input: &mut dyn BufRead, out: &mut dyn Write, q: &str, n: usize) -> Result<Option<usize>> {
    loop {
        write!(out, "{q}")?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            return Ok(None);
        };
        if line.is_empty() {
            return Ok(None);
        }
        match line.parse::<usize>() {
            Ok(i) if (1..=n).contains(&i) => return Ok(Some(i - 1)),
            _ => writeln!(out, "Enter a number from 1 to {n}, or press Enter to skip.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_sample_layout() {
        let base = std::env::temp_dir().join(format!("relay-migrate-{}", uuid::Uuid::new_v4()));
        let code = base.join("code");
        let lm = base.join(".lmstudio");
        let relay = base.join("relay");
        for d in [
            code.join("alpha"),
            code.join("beta"),
            code.join("gamma"),
            lm.join("skills/docx"),
            lm.join("skills/group/pdf"),
            lm.join("models/lmstudio-community/Qwen-GGUF"),
            relay.clone(),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            lm.join("skills/docx/SKILL.md"),
            "---\nname: docx\ndescription: Word\n---\n",
        )
        .unwrap();
        std::fs::write(
            lm.join("skills/group/pdf/SKILL.md"),
            "---\nname: pdf\ndescription: PDF\n---\n",
        )
        .unwrap();
        std::fs::write(
            lm.join("models/lmstudio-community/Qwen-GGUF/model-Q6_K.gguf"),
            vec![0u8; 2048],
        )
        .unwrap();
        std::fs::write(
            lm.join("models/lmstudio-community/Qwen-GGUF/mmproj-model.gguf"),
            b"x",
        )
        .unwrap();
        let prompt = base.join("bionic.txt");
        std::fs::write(&prompt, "You are Bionic.").unwrap();

        let cfg = relay.join("config.toml");
        let args = MigrateArgs {
            projects: vec![code.join("gamma")],
            scan: Some(code.clone()),
            system_prompt: Some(prompt.clone()),
            lmstudio: Some(lm.clone()),
            ..Default::default()
        };
        // Scan answers: alpha yes, beta no, gamma yes (duplicate); then model 1.
        let mut input = std::io::Cursor::new(b"y\nn\ny\n1\n".to_vec());
        let mut out = Vec::new();
        run_with_io(&cfg, args, &mut input, &mut out).unwrap();
        let text = String::from_utf8_lossy(&out);

        let loaded = LoadedConfig::load(&cfg).unwrap();
        assert_eq!(loaded.config.skills_paths.len(), 2, "{text}");
        assert!(
            loaded.config.llama.model_path.ends_with("model-Q6_K.gguf"),
            "{text}"
        );
        assert_eq!(
            std::fs::read_to_string(relay.join("system_prompt.md")).unwrap(),
            "You are Bionic."
        );
        let conn = db::open_connection(&loaded.db_path()).unwrap();
        let names: Vec<String> = db::list_projects(&conn)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["alpha", "gamma"], "{text}");
        assert!(text.contains("already a project"), "{text}");
        // Originals untouched.
        assert!(lm.join("skills/docx/SKILL.md").exists());
        assert_eq!(std::fs::read_to_string(&prompt).unwrap(), "You are Bionic.");
        assert!(!relay.join("skills/docx").exists());

        // Running again adds nothing new.
        let mut input = std::io::Cursor::new(b"\n".to_vec());
        let mut out = Vec::new();
        let args = MigrateArgs {
            projects: vec![code.join("alpha")],
            lmstudio: Some(lm.clone()),
            ..Default::default()
        };
        run_with_io(&cfg, args, &mut input, &mut out).unwrap();
        let loaded = LoadedConfig::load(&cfg).unwrap();
        assert_eq!(loaded.config.skills_paths.len(), 2);
        assert!(loaded.config.llama.model_path.ends_with("model-Q6_K.gguf"));
        std::fs::remove_dir_all(base).unwrap();
    }
}
