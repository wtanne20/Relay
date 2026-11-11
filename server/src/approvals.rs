//! Smarter approvals: "always allow" rules, so the agent stops asking about
//! things you've already said yes to, and an activity log of every tool call
//! the agent made (and who allowed it).
//!
//! Rules only ever skip a question. They never let a tool run that is turned
//! off, and with "Ask before acting" set to Never nothing is asked anyway.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{self, Project, ToolCall};

/// What a rule covers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// run_command: commands that start with the pattern's words.
    Command,
    /// write_file and edit_file: files inside the pattern's folder.
    Files,
    /// fetch_url: pages on the pattern's site (and its subdomains).
    Web,
}

impl RuleKind {
    fn as_str(self) -> &'static str {
        match self {
            RuleKind::Command => "command",
            RuleKind::Files => "files",
            RuleKind::Web => "web",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "command" => Some(RuleKind::Command),
            "files" => Some(RuleKind::Files),
            "web" => Some(RuleKind::Web),
            _ => None,
        }
    }

    /// The kind of rule that could cover a tool, if any.
    pub fn for_tool(tool: &str) -> Option<Self> {
        match tool {
            "run_command" => Some(RuleKind::Command),
            "write_file" | "edit_file" => Some(RuleKind::Files),
            "fetch_url" => Some(RuleKind::Web),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Rule {
    pub id: i64,
    pub kind: RuleKind,
    pub pattern: String,
    /// None = every project.
    pub project_id: Option<String>,
    /// For the list: the project's name, when the rule has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    pub created_at: String,
    /// How many tool calls this rule has allowed.
    pub uses: i64,
    pub last_used_at: Option<String>,
}

/// A rule before it's saved: what the approval card offers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleDraft {
    pub kind: RuleKind,
    pub pattern: String,
    /// None = every project.
    #[serde(default)]
    pub project_id: Option<String>,
    /// A short plain description, e.g. "Commands starting with git status".
    #[serde(default)]
    pub label: String,
}

pub fn ensure_schema(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS approval_rules (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            pattern TEXT NOT NULL,
            project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
            created_at TEXT NOT NULL,
            uses INTEGER NOT NULL DEFAULT 0,
            last_used_at TEXT
        );
        CREATE TABLE IF NOT EXISTS audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            project_id TEXT NOT NULL,
            project_name TEXT NOT NULL,
            session_id TEXT NOT NULL,
            session_title TEXT NOT NULL,
            tool TEXT NOT NULL,
            detail TEXT NOT NULL,
            decision TEXT NOT NULL,
            rule_id INTEGER,
            is_error INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS audit_session ON audit_log(session_id, id);
        CREATE INDEX IF NOT EXISTS audit_project ON audit_log(project_id, id);",
    )?;
    Ok(())
}

// ---------- matching ----------

/// Characters that chain, pipe, redirect or substitute in sh and PowerShell.
/// A command containing any of them is never covered by a rule: "git status"
/// must not also allow "git status; rm -rf ~".
const SHELL_META: &[char] = &[';', '&', '|', '`', '$', '<', '>', '(', ')', '{', '}', '\n', '\r'];

fn plain_command(cmd: &str) -> bool {
    !cmd.contains(SHELL_META)
}

fn words(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

/// A folder or file path made relative to the project root, as clean parts.
/// None if it leaves the project ("..") or is absolute somewhere else.
fn relative_parts(path: &str, project_root: &str) -> Option<Vec<String>> {
    let norm = |s: &str| s.replace('\\', "/");
    let p = norm(path.trim());
    let root = norm(project_root.trim()).trim_end_matches('/').to_string();
    let is_abs = p.starts_with('/') || p.as_bytes().get(1) == Some(&b':');
    let rel = if is_abs {
        if root.is_empty() {
            return None;
        }
        // Windows paths compare without case.
        let windows = root.as_bytes().get(1) == Some(&b':');
        let (pl, rl) = if windows {
            (p.to_lowercase(), root.to_lowercase())
        } else {
            (p.clone(), root.clone())
        };
        if pl == rl {
            String::new()
        } else if pl.starts_with(&format!("{rl}/")) {
            p[root.len() + 1..].to_string()
        } else {
            return None;
        }
    } else {
        p
    };
    let mut out = Vec::new();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => out.push(other.to_string()),
        }
    }
    Some(out)
}

/// The site a URL points at, without "www.". None for anything that isn't a
/// plain http(s) address.
fn host_of(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let h = u.host_str()?.to_ascii_lowercase();
    Some(h.strip_prefix("www.").unwrap_or(&h).to_string())
}

fn arg(call: &ToolCall, key: &str) -> Option<String> {
    serde_json::from_str::<Value>(&call.arguments)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

/// Whether a rule covers this tool call in this project.
pub fn covers(kind: RuleKind, pattern: &str, call: &ToolCall, project: &Project) -> bool {
    if RuleKind::for_tool(&call.name) != Some(kind) {
        return false;
    }
    match kind {
        RuleKind::Command => {
            let Some(cmd) = arg(call, "command") else {
                return false;
            };
            let want = words(pattern);
            let have = words(&cmd);
            plain_command(&cmd) && !want.is_empty() && have.len() >= want.len() && have[..want.len()] == want[..]
        }
        RuleKind::Files => {
            let Some(path) = arg(call, "path") else {
                return false;
            };
            let (Some(folder), Some(file)) = (
                relative_parts(pattern, &project.path),
                relative_parts(&path, &project.path),
            ) else {
                return false;
            };
            // The folder itself isn't a file; a file must be inside it.
            file.len() > folder.len() && file[..folder.len()] == folder[..]
        }
        RuleKind::Web => {
            let Some(host) = arg(call, "url").and_then(|u| host_of(&u)) else {
                return false;
            };
            let site = pattern.trim().to_ascii_lowercase();
            let site = site.strip_prefix("www.").unwrap_or(&site);
            !site.is_empty() && (host == site || host.ends_with(&format!(".{site}")))
        }
    }
}

/// Words that start a command and are commonly followed by a subcommand
/// (`git status`, `cargo test`). For anything else the whole command is the
/// pattern, so a yes to `python build.py` doesn't become a yes to any Python.
fn has_subcommands(program: &str) -> bool {
    let p = program.to_ascii_lowercase();
    let p = p.strip_suffix(".exe").unwrap_or(&p);
    matches!(
        p,
        "git" | "cargo" | "npm" | "pnpm" | "yarn" | "bun" | "npx" | "go" | "dotnet" | "docker" | "kubectl"
            | "make" | "gradle" | "gradlew" | "./gradlew" | "mvn" | "pip" | "pip3" | "uv" | "poetry" | "rustup"
            | "winget" | "brew" | "apt" | "flutter" | "dart" | "deno" | "swift" | "gh" | "terraform"
    )
}

/// The rule offered under "Always allow" for a call that's waiting, if the
/// call can be covered by one. Scoped to the project by default.
pub fn suggest(call: &ToolCall, project: &Project) -> Option<RuleDraft> {
    let kind = RuleKind::for_tool(&call.name)?;
    let project_id = Some(project.id.clone());
    match kind {
        RuleKind::Command => {
            let cmd = arg(call, "command")?;
            if !plain_command(&cmd) {
                return None;
            }
            let w = words(&cmd);
            let first = *w.first()?;
            let pattern = match w.get(1) {
                Some(sub)
                    if has_subcommands(first)
                        && !sub.starts_with('-')
                        && sub.chars().all(|c| c.is_alphanumeric() || c == '-' || c == ':' || c == '_') =>
                {
                    format!("{first} {sub}")
                }
                _ => w.join(" "),
            };
            let pattern: String = pattern.chars().take(200).collect();
            Some(RuleDraft {
                label: format!("Commands starting with {pattern}"),
                kind,
                pattern,
                project_id,
            })
        }
        RuleKind::Files => {
            if project.path.trim().is_empty() && project.device_id.is_none() {
                return None;
            }
            let parts = relative_parts(&arg(call, "path")?, &project.path)?;
            let folder = parts[..parts.len().saturating_sub(1)].join("/");
            Some(RuleDraft {
                label: if folder.is_empty() {
                    "Changes to any file in this project".into()
                } else {
                    format!("Changes to files in {folder}/")
                },
                pattern: if folder.is_empty() { ".".into() } else { folder },
                kind,
                project_id,
            })
        }
        RuleKind::Web => {
            let host = host_of(&arg(call, "url")?)?;
            Some(RuleDraft {
                label: format!("Reading pages on {host}"),
                pattern: host,
                kind,
                project_id,
            })
        }
    }
}

/// Checks and tidies a rule someone wants to save.
pub fn clean(draft: &RuleDraft) -> Result<RuleDraft> {
    let pattern = draft.pattern.trim().to_string();
    if pattern.is_empty() {
        bail!("say what the rule allows");
    }
    if pattern.chars().count() > 500 {
        bail!("that rule is too long");
    }
    match draft.kind {
        RuleKind::Command => {
            if !plain_command(&pattern) {
                bail!("a command rule can't contain ; & | $ ` < > ( ) or braces");
            }
        }
        RuleKind::Files => {
            if draft.project_id.is_none() {
                bail!("a folder rule belongs to one project");
            }
            if pattern.replace('\\', "/").split('/').any(|p| p == "..") {
                bail!("the folder can't go outside the project");
            }
        }
        RuleKind::Web => {
            let host = if pattern.contains("://") {
                host_of(&pattern)
            } else {
                host_of(&format!("https://{pattern}"))
            };
            let Some(host) = host else {
                bail!("that isn't a website address");
            };
            return Ok(RuleDraft {
                pattern: host,
                ..draft.clone()
            });
        }
    }
    Ok(RuleDraft {
        pattern,
        ..draft.clone()
    })
}

// ---------- rules in the database ----------

const RULE_COLS: &str = "r.id, r.kind, r.pattern, r.project_id, p.name, r.created_at, r.uses, r.last_used_at";

fn rule_row(r: &Row) -> rusqlite::Result<Rule> {
    let kind: String = r.get(1)?;
    Ok(Rule {
        id: r.get(0)?,
        kind: RuleKind::parse(&kind).unwrap_or(RuleKind::Command),
        pattern: r.get(2)?,
        project_id: r.get(3)?,
        project_name: r.get(4)?,
        created_at: r.get(5)?,
        uses: r.get(6)?,
        last_used_at: r.get(7)?,
    })
}

pub fn list_rules(c: &Connection) -> Result<Vec<Rule>> {
    let mut st = c.prepare(&format!(
        "SELECT {RULE_COLS} FROM approval_rules r LEFT JOIN projects p ON p.id = r.project_id ORDER BY r.id DESC"
    ))?;
    let rows = st.query_map([], rule_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn add_rule(c: &Connection, d: &RuleDraft) -> Result<Rule> {
    // The same rule twice is one rule.
    let existing: Option<i64> = c
        .query_row(
            "SELECT id FROM approval_rules WHERE kind = ?1 AND pattern = ?2 AND project_id IS ?3",
            params![d.kind.as_str(), d.pattern, d.project_id],
            |r| r.get(0),
        )
        .optional()?;
    let id = match existing {
        Some(id) => id,
        None => {
            c.execute(
                "INSERT INTO approval_rules (kind, pattern, project_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![d.kind.as_str(), d.pattern, d.project_id, db::now()],
            )?;
            c.last_insert_rowid()
        }
    };
    Ok(c.query_row(
        &format!("SELECT {RULE_COLS} FROM approval_rules r LEFT JOIN projects p ON p.id = r.project_id WHERE r.id = ?1"),
        [id],
        rule_row,
    )?)
}

pub fn delete_rule(c: &Connection, id: i64) -> Result<bool> {
    Ok(c.execute("DELETE FROM approval_rules WHERE id = ?1", [id])? > 0)
}

/// The first rule that covers this call, counted as used.
pub fn matching_rule(c: &Connection, call: &ToolCall, project: &Project) -> Result<Option<i64>> {
    let Some(kind) = RuleKind::for_tool(&call.name) else {
        return Ok(None);
    };
    let mut st = c.prepare(
        "SELECT id, pattern FROM approval_rules WHERE kind = ?1 AND (project_id IS NULL OR project_id = ?2) ORDER BY id",
    )?;
    let rules = st
        .query_map(params![kind.as_str(), project.id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let hit = rules
        .into_iter()
        .find(|(_, pattern)| covers(kind, pattern, call, project))
        .map(|(id, _)| id);
    if let Some(id) = hit {
        c.execute(
            "UPDATE approval_rules SET uses = uses + 1, last_used_at = ?2 WHERE id = ?1",
            params![id, db::now()],
        )?;
    }
    Ok(hit)
}

// ---------- activity log ----------

/// Entries kept; older ones are dropped as new ones come in.
pub const AUDIT_KEEP: i64 = 20_000;

/// How a tool call got to run (or didn't).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecision {
    /// Nothing needed asking.
    Auto,
    /// Covered by an "always allow" rule.
    Rule,
    /// You allowed it.
    Allowed,
    /// You allowed everything for the rest of the task.
    AllowedAll,
    /// You denied it.
    Denied,
    /// The task was stopped before it ran.
    Stopped,
    /// The tool is turned off in settings.
    Off,
}

impl AuditDecision {
    fn as_str(self) -> &'static str {
        match self {
            AuditDecision::Auto => "auto",
            AuditDecision::Rule => "rule",
            AuditDecision::Allowed => "allowed",
            AuditDecision::AllowedAll => "allowed_all",
            AuditDecision::Denied => "denied",
            AuditDecision::Stopped => "stopped",
            AuditDecision::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub at: String,
    pub project_id: String,
    pub project_name: String,
    pub session_id: String,
    pub session_title: String,
    pub tool: String,
    /// The command, file, address or search: the gist, not the content.
    pub detail: String,
    pub decision: String,
    pub rule_id: Option<i64>,
    pub is_error: bool,
    /// False once the chat has been deleted (the entry stays).
    pub session_exists: bool,
}

pub struct NewAudit<'a> {
    pub project: &'a Project,
    pub session_id: &'a str,
    pub session_title: &'a str,
    pub call: &'a ToolCall,
    pub decision: AuditDecision,
    pub rule_id: Option<i64>,
    pub is_error: bool,
}

/// The gist of a call for the log: the command, path, address or search.
pub fn detail(call: &ToolCall) -> String {
    let key = match call.name.as_str() {
        "run_command" => "command",
        "fetch_url" => "url",
        "web_search" => "query",
        _ => "path",
    };
    let d = arg(call, key).unwrap_or_default();
    d.chars().take(1000).collect()
}

pub fn record(c: &Connection, a: &NewAudit) -> Result<()> {
    c.execute(
        "INSERT INTO audit_log (at, project_id, project_name, session_id, session_title, tool, detail, decision, rule_id, is_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            db::now(),
            a.project.id,
            a.project.name,
            a.session_id,
            a.session_title,
            a.call.name,
            detail(a.call),
            a.decision.as_str(),
            a.rule_id,
            a.is_error,
        ],
    )?;
    let id = c.last_insert_rowid();
    if id % 500 == 0 {
        c.execute("DELETE FROM audit_log WHERE id <= ?1", [id - AUDIT_KEEP])?;
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
pub struct AuditQuery {
    pub before: Option<i64>,
    pub limit: Option<usize>,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    /// "commands", "files", "web", "asked" (you answered) or "denied".
    pub filter: Option<String>,
}

/// Newest first, a page at a time; true if there are older entries.
pub fn list_audit(c: &Connection, q: &AuditQuery) -> Result<(Vec<AuditEntry>, bool)> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut sql = String::from(
        "SELECT a.id, a.at, a.project_id, COALESCE(p.name, a.project_name), a.session_id,
                COALESCE(s.title, a.session_title), a.tool, a.detail, a.decision, a.rule_id, a.is_error,
                s.id IS NOT NULL
         FROM audit_log a
         LEFT JOIN sessions s ON s.id = a.session_id
         LEFT JOIN projects p ON p.id = a.project_id
         WHERE a.id < ?1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(q.before.unwrap_or(i64::MAX))];
    if let Some(p) = q.project_id.as_ref().filter(|s| !s.is_empty()) {
        args.push(Box::new(p.clone()));
        sql.push_str(&format!(" AND a.project_id = ?{}", args.len()));
    }
    if let Some(s) = q.session_id.as_ref().filter(|s| !s.is_empty()) {
        args.push(Box::new(s.clone()));
        sql.push_str(&format!(" AND a.session_id = ?{}", args.len()));
    }
    match q.filter.as_deref().unwrap_or("") {
        "commands" => sql.push_str(" AND a.tool = 'run_command'"),
        "files" => sql.push_str(" AND a.tool IN ('write_file', 'edit_file')"),
        "web" => sql.push_str(" AND a.tool IN ('fetch_url', 'web_search')"),
        "asked" => sql.push_str(" AND a.decision IN ('allowed', 'allowed_all', 'denied')"),
        "denied" => sql.push_str(" AND a.decision = 'denied'"),
        _ => {}
    }
    args.push(Box::new((limit + 1) as i64));
    sql.push_str(&format!(" ORDER BY a.id DESC LIMIT ?{}", args.len()));
    let mut st = c.prepare(&sql)?;
    let mut rows = st
        .query_map(rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                at: r.get(1)?,
                project_id: r.get(2)?,
                project_name: r.get(3)?,
                session_id: r.get(4)?,
                session_title: r.get(5)?,
                tool: r.get(6)?,
                detail: r.get(7)?,
                decision: r.get(8)?,
                rule_id: r.get(9)?,
                is_error: r.get(10)?,
                session_exists: r.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok((rows, more))
}

pub fn clear_audit(c: &Connection) -> Result<usize> {
    Ok(c.execute("DELETE FROM audit_log", [])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(path: &str) -> Project {
        Project {
            id: "p".into(),
            name: "P".into(),
            path: path.into(),
            instructions: String::new(),
            created_at: String::new(),
            model: None,
            device_id: None,
            archived: false,
        }
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments: args.to_string(),
        }
    }

    #[test]
    fn command_rules_match_whole_words_and_refuse_chains() {
        let p = project("C:\\code\\app");
        let k = RuleKind::Command;
        assert!(covers(k, "git status", &call("run_command", json!({"command": "git status"})), &p));
        assert!(covers(k, "git status", &call("run_command", json!({"command": "git  status -s"})), &p));
        assert!(!covers(k, "git status", &call("run_command", json!({"command": "git statusx"})), &p));
        assert!(!covers(k, "git status", &call("run_command", json!({"command": "git push"})), &p));
        for bad in ["git status; rm -rf ~", "git status && del x", "git status | sh", "git status $(whoami)", "git status > f", "git status\nrm x"] {
            assert!(!covers(k, "git status", &call("run_command", json!({"command": bad})), &p), "{bad}");
        }
        // A command rule never covers another tool.
        assert!(!covers(k, "git", &call("write_file", json!({"path": "git"})), &p));
    }

    #[test]
    fn folder_rules_stay_inside_the_folder() {
        let p = project("C:\\code\\app");
        let k = RuleKind::Files;
        let w = |path: &str| call("write_file", json!({"path": path, "content": ""}));
        assert!(covers(k, "src", &w("src/main.rs"), &p));
        assert!(covers(k, "src", &w("src\\deep\\x.rs"), &p));
        assert!(covers(k, "src", &w("./src/x.rs"), &p));
        assert!(covers(k, "src", &w("C:\\Code\\App\\src\\x.rs"), &p));
        assert!(!covers(k, "src", &w("src"), &p));
        assert!(!covers(k, "src", &w("srcx/a.rs"), &p));
        assert!(!covers(k, "src", &w("src/../secrets.txt"), &p));
        assert!(!covers(k, "src", &w("C:\\other\\src\\x.rs"), &p));
        assert!(covers(k, ".", &w("README.md"), &p));
        assert!(covers(k, "src", &call("edit_file", json!({"path": "src/a.rs"})), &p));
        assert!(!covers(k, "src", &call("read_file", json!({"path": "src/a.rs"})), &p));
    }

    #[test]
    fn web_rules_cover_the_site_and_subdomains() {
        let p = project("");
        let k = RuleKind::Web;
        let f = |u: &str| call("fetch_url", json!({"url": u}));
        assert!(covers(k, "docs.rs", &f("https://docs.rs/serde"), &p));
        assert!(covers(k, "rust-lang.org", &f("https://www.rust-lang.org/learn"), &p));
        assert!(covers(k, "rust-lang.org", &f("https://doc.rust-lang.org/std"), &p));
        assert!(!covers(k, "rust-lang.org", &f("https://evilrust-lang.org"), &p));
        assert!(!covers(k, "rust-lang.org", &f("https://rust-lang.org.evil.com"), &p));
        assert!(!covers(k, "rust-lang.org", &f("file:///etc/passwd"), &p));
    }

    #[test]
    fn suggestions() {
        let p = project("/home/me/app");
        let s = |c: ToolCall| suggest(&c, &p).map(|d| d.pattern);
        assert_eq!(s(call("run_command", json!({"command": "git status -s"}))).as_deref(), Some("git status"));
        assert_eq!(s(call("run_command", json!({"command": "cargo test --all"}))).as_deref(), Some("cargo test"));
        assert_eq!(s(call("run_command", json!({"command": "python build.py"}))).as_deref(), Some("python build.py"));
        assert_eq!(s(call("run_command", json!({"command": "ls | wc"}))), None);
        assert_eq!(s(call("write_file", json!({"path": "src/ui/a.tsx"}))).as_deref(), Some("src/ui"));
        assert_eq!(s(call("write_file", json!({"path": "a.txt"}))).as_deref(), Some("."));
        assert_eq!(s(call("fetch_url", json!({"url": "https://www.example.com/x"}))).as_deref(), Some("example.com"));
        assert_eq!(s(call("read_file", json!({"path": "a"}))), None);
    }

    #[test]
    fn rules_and_log_in_the_database() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT); INSERT INTO projects VALUES ('p', 'P');
             CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT);",
        )
        .unwrap();
        ensure_schema(&c).unwrap();
        let p = project("/app");
        let d = RuleDraft {
            kind: RuleKind::Command,
            pattern: "npm test".into(),
            project_id: Some("p".into()),
            label: String::new(),
        };
        let r = add_rule(&c, &d).unwrap();
        assert_eq!(add_rule(&c, &d).unwrap().id, r.id);
        assert_eq!(r.project_name.as_deref(), Some("P"));
        let hit = call("run_command", json!({"command": "npm test -- --watch=false"}));
        assert_eq!(matching_rule(&c, &hit, &p).unwrap(), Some(r.id));
        let mut other = p.clone();
        other.id = "q".into();
        assert_eq!(matching_rule(&c, &hit, &other).unwrap(), None);
        assert_eq!(list_rules(&c).unwrap()[0].uses, 1);

        record(
            &c,
            &NewAudit {
                project: &p,
                session_id: "gone",
                session_title: "Old chat",
                call: &hit,
                decision: AuditDecision::Rule,
                rule_id: Some(r.id),
                is_error: false,
            },
        )
        .unwrap();
        let (rows, more) = list_audit(&c, &AuditQuery::default()).unwrap();
        assert!(!more);
        assert_eq!(rows[0].detail, "npm test -- --watch=false");
        assert_eq!(rows[0].decision, "rule");
        assert_eq!(rows[0].session_title, "Old chat");
        assert!(!rows[0].session_exists);
        let (rows, _) = list_audit(&c, &AuditQuery { filter: Some("files".into()), ..Default::default() }).unwrap();
        assert!(rows.is_empty());
        assert!(delete_rule(&c, r.id).unwrap());
    }

    #[test]
    fn cleaning_rules() {
        let web = RuleDraft { kind: RuleKind::Web, pattern: "https://www.Example.com/a".into(), project_id: None, label: String::new() };
        assert_eq!(clean(&web).unwrap().pattern, "example.com");
        let cmd = RuleDraft { kind: RuleKind::Command, pattern: "git status; rm".into(), project_id: None, label: String::new() };
        assert!(clean(&cmd).is_err());
        let files = RuleDraft { kind: RuleKind::Files, pattern: "../x".into(), project_id: Some("p".into()), label: String::new() };
        assert!(clean(&files).is_err());
        let files = RuleDraft { kind: RuleKind::Files, pattern: "src".into(), project_id: None, label: String::new() };
        assert!(clean(&files).is_err());
    }
}
