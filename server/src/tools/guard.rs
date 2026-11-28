//! Best-effort blocklist for obviously destructive shell commands. This is a
//! safety net against model mistakes, not a security boundary: a determined
//! script can always get around string matching.

use super::sandbox::Sandbox;

/// The sandbox-free part of the guard: phrases and programs that are never
/// allowed, on the PC or on a device. Used for commands that run on a
/// paired device, where the delete-target check can't apply (the folder is
/// not on this machine - the device's own sandbox confines it).
pub fn check_command_remote(command: &str) -> Option<String> {
    let lower = command.to_lowercase();
    let squashed: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");

    const BLOCKED_PHRASES: &[(&str, &str)] = &[
        ("format-volume", "formatting volumes"),
        ("clear-disk", "wiping disks"),
        ("initialize-disk", "initializing disks"),
        ("remove-partition", "removing partitions"),
        ("set-partition", "changing partitions"),
        ("stop-computer", "shutting down the PC"),
        ("restart-computer", "restarting the PC"),
        ("vssadmin delete", "deleting shadow copies"),
        ("wmic shadowcopy", "deleting shadow copies"),
        ("cipher /w", "wiping free space"),
        ("reg delete hklm", "deleting machine registry keys"),
        ("remove-item -path hklm:", "deleting machine registry keys"),
        ("remove-item hklm:", "deleting machine registry keys"),
        ("set-executionpolicy", "changing execution policy"),
        ("disable-computerrestore", "disabling system restore"),
        ("rm -rf /", "deleting the filesystem root"),
        ("rm -rf ~", "deleting the home folder"),
        ("rm -rf *", "deleting everything"),
        ("rm -fr /", "deleting the filesystem root"),
        (":(){", "fork bomb"),
        ("dd if=", "raw disk writes"),
        ("net user", "changing user accounts"),
        (
            "invoke-expression (new-object net.webclient)",
            "download-and-execute",
        ),
        ("iex (new-object net.webclient)", "download-and-execute"),
        ("iex(new-object net.webclient)", "download-and-execute"),
    ];
    for (phrase, why) in BLOCKED_PHRASES {
        if squashed.contains(phrase) {
            return Some(format!(
                "blocked: {why} is not allowed (matched '{phrase}')"
            ));
        }
    }

    let tokens = tokenize(&lower);
    const BLOCKED_PROGRAMS: &[&str] = &[
        "diskpart",
        "shutdown",
        "logoff",
        "bcdedit",
        "takeown",
        "icacls",
        "mkfs",
        "format-volume",
    ];
    for t in &tokens {
        let t = t.trim_start_matches(['&', '(', '{', '"', '\'']);
        let t = t.strip_suffix(".exe").unwrap_or(t);
        let base = t.rsplit(['\\', '/']).next().unwrap_or(t);
        if BLOCKED_PROGRAMS.contains(&base) || base.starts_with("mkfs.") {
            return Some(format!("blocked: running '{base}' is not allowed"));
        }
    }
    None
}

/// Returns Some(reason) if the command should be refused.
pub fn check_command(command: &str, sandbox: &Sandbox) -> Option<String> {
    if let Some(r) = check_command_remote(command) {
        return Some(r);
    }
    let lower = command.to_lowercase();
    let tokens = tokenize(&lower);

    // `format c:` / `format d: /q` etc. (cmd's format.com)
    for w in tokens.windows(2) {
        if (w[0] == "format" || w[0] == "format.com") && is_drive_spec(&w[1]) {
            return Some("blocked: formatting drives is not allowed".into());
        }
    }

    // Deletions must target paths inside the project.
    const DELETE_VERBS: &[&str] = &[
        "remove-item",
        "rm",
        "ri",
        "del",
        "erase",
        "rd",
        "rmdir",
        "rimraf",
        "remove-itemproperty",
    ];
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].trim_start_matches(['&', '(', '{']);
        if DELETE_VERBS.contains(&t) {
            let mut j = i + 1;
            while j < tokens.len() && !is_separator(&tokens[j]) {
                let arg = tokens[j].trim_matches(['"', '\'', '(', ')', ',']);
                if let Some(reason) = check_delete_target(arg, sandbox) {
                    return Some(reason);
                }
                j += 1;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

fn check_delete_target(arg: &str, sandbox: &Sandbox) -> Option<String> {
    if arg.is_empty() || arg.starts_with('-') {
        return None;
    }
    // cmd-style switches like /s /q (but not a lone "/" = unix root).
    if arg.len() == 2 && arg.starts_with('/') && arg.as_bytes()[1].is_ascii_alphabetic() {
        return None;
    }
    let refers_outside_variable = arg.starts_with('~')
        || arg.starts_with("$env:")
        || arg.starts_with("$home")
        || arg.starts_with("%")
        || arg.starts_with("${env:");
    if refers_outside_variable {
        return Some(format!(
            "blocked: deleting '{arg}' (outside the project folder) is not allowed"
        ));
    }
    let looks_like_path =
        arg.contains(['/', '\\']) || arg.contains("..") || is_drive_spec(arg) || arg == "*";
    if looks_like_path && is_drive_spec(arg) && !cfg!(windows) {
        // A Windows drive path can never be inside a project on other OSes.
        return Some(format!(
            "blocked: deleting '{arg}' outside the project folder is not allowed"
        ));
    }
    if looks_like_path {
        // Wildcards: check the directory part.
        let check = arg
            .trim_end_matches(['*', '.'])
            .trim_end_matches(['/', '\\']);
        let check = if check.is_empty() && arg.starts_with(['/', '\\']) {
            "/"
        } else {
            check
        };
        if check.is_empty() {
            return None; // "*" in the working directory (the project)
        }
        // Treat backslashes as separators everywhere (PowerShell accepts both).
        let check = if cfg!(windows) {
            check.to_string()
        } else {
            check.replace('\\', "/")
        };
        if !sandbox.contains_lexical(&check) {
            return Some(format!(
                "blocked: deleting '{arg}' outside the project folder is not allowed"
            ));
        }
    }
    None
}

fn is_drive_spec(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn is_separator(t: &str) -> bool {
    matches!(t, ";" | "|" | "&&" | "||" | "&")
}

/// Splits on whitespace and statement separators, keeping separators as tokens.
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for ch in s.chars() {
        match ch {
            c if c.is_whitespace() => flush(&mut cur, &mut out),
            ';' | '|' | '\n' => {
                flush(&mut cur, &mut out);
                out.push(";".into());
            }
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb() -> (Sandbox, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("relay-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("proj/build")).unwrap();
        (Sandbox::new(&d.join("proj"), &[]).unwrap(), d)
    }

    #[test]
    fn blocks_destructive() {
        let (s, d) = sb();
        for c in [
            "format c:",
            "Format-Volume -DriveLetter D",
            "rm -rf /",
            "shutdown /s /t 0",
            "Stop-Computer -Force",
            "Remove-Item -Recurse -Force C:\\Windows",
            "Remove-Item ..\\other -Recurse",
            "del /s /q $env:USERPROFILE\\Documents",
            "ls; rm -r ~/stuff",
            "rd /s /q \\",
        ] {
            assert!(check_command(c, &s).is_some(), "should block: {c}");
        }
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn allows_normal() {
        let (s, d) = sb();
        for c in [
            "cargo build --release",
            "Remove-Item -Recurse -Force build",
            "rm -r ./build",
            "del *.tmp",
            "npm run format",
            "git status; git diff",
            "Get-ChildItem -Recurse | Select-String TODO",
            "dotnet format",
            "rg graceful_shutdown src",
        ] {
            assert!(check_command(c, &s).is_none(), "should allow: {c}");
        }
        std::fs::remove_dir_all(d).unwrap();
    }
}
