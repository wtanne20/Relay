//! SQLite storage. All access goes through `Db::call`, which runs the closure
//! on tokio's blocking pool so database work never blocks the async runtime.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON string of arguments, exactly as produced by the model.
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub session_id: String,
    /// "user", "assistant" or "tool"
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub is_error: bool,
    pub created_at: String,
}

#[derive(Debug, Default, Clone)]
pub struct NewMessage {
    pub role: String,
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub is_error: bool,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'idle',
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_project ON sessions(project_id, updated_at);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    reasoning TEXT,
    tool_calls TEXT,
    tool_call_id TEXT,
    tool_name TEXT,
    is_error INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id, id);
CREATE TABLE IF NOT EXISTS push_tokens (
    token TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);
"#;

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = open_connection(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run `f` with the connection on the blocking thread pool.
    pub async fn call<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn
                .lock()
                .map_err(|_| anyhow::anyhow!("database mutex poisoned"))?;
            f(&mut guard)
        })
        .await
        .context("database task panicked")?
    }
}

pub fn open_connection(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

// ---------- projects ----------

fn project_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        created_at: r.get(3)?,
    })
}

pub fn list_projects(c: &Connection) -> Result<Vec<Project>> {
    let mut st =
        c.prepare("SELECT id, name, path, created_at FROM projects ORDER BY name COLLATE NOCASE")?;
    let rows = st
        .query_map([], project_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_project(c: &Connection, id: &str) -> Result<Option<Project>> {
    Ok(c.query_row(
        "SELECT id, name, path, created_at FROM projects WHERE id = ?1",
        [id],
        project_row,
    )
    .optional()?)
}

pub fn find_project_by_path(c: &Connection, path: &str) -> Result<Option<Project>> {
    Ok(c.query_row(
        "SELECT id, name, path, created_at FROM projects WHERE path = ?1 COLLATE NOCASE",
        [path],
        project_row,
    )
    .optional()?)
}

pub fn create_project(c: &Connection, name: &str, path: &str) -> Result<Project> {
    let p = Project {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        path: path.to_string(),
        created_at: now(),
    };
    c.execute(
        "INSERT INTO projects (id, name, path, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![p.id, p.name, p.path, p.created_at],
    )?;
    Ok(p)
}

pub fn update_project(
    c: &Connection,
    id: &str,
    name: Option<&str>,
    path: Option<&str>,
) -> Result<Option<Project>> {
    if let Some(n) = name {
        c.execute(
            "UPDATE projects SET name = ?1 WHERE id = ?2",
            params![n, id],
        )?;
    }
    if let Some(p) = path {
        c.execute(
            "UPDATE projects SET path = ?1 WHERE id = ?2",
            params![p, id],
        )?;
    }
    get_project(c, id)
}

/// Removes the project (and its sessions/messages) from Relay's database.
/// Never touches the folder on disk.
pub fn delete_project(c: &Connection, id: &str) -> Result<bool> {
    Ok(c.execute("DELETE FROM projects WHERE id = ?1", [id])? > 0)
}

// ---------- sessions ----------

const SESSION_COLS: &str = "id, project_id, title, status, error, created_at, updated_at";

fn session_row(r: &Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        project_id: r.get(1)?,
        title: r.get(2)?,
        status: r.get(3)?,
        error: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

pub fn list_sessions(c: &Connection, project_id: &str) -> Result<Vec<Session>> {
    let mut st = c.prepare(&format!(
        "SELECT {SESSION_COLS} FROM sessions WHERE project_id = ?1 ORDER BY updated_at DESC"
    ))?;
    let rows = st
        .query_map([project_id], session_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_session_ids_for_project(c: &Connection, project_id: &str) -> Result<Vec<String>> {
    let mut st = c.prepare("SELECT id FROM sessions WHERE project_id = ?1")?;
    let rows = st
        .query_map([project_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

pub fn get_session(c: &Connection, id: &str) -> Result<Option<Session>> {
    Ok(c.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [id],
        session_row,
    )
    .optional()?)
}

pub fn create_session(c: &Connection, project_id: &str, title: &str) -> Result<Session> {
    let t = now();
    let s = Session {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.to_string(),
        title: title.to_string(),
        status: "idle".into(),
        error: None,
        created_at: t.clone(),
        updated_at: t,
    };
    c.execute(
        "INSERT INTO sessions (id, project_id, title, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![s.id, s.project_id, s.title, s.status, s.created_at, s.updated_at],
    )?;
    Ok(s)
}

pub fn rename_session(c: &Connection, id: &str, title: &str) -> Result<Option<Session>> {
    c.execute(
        "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, now(), id],
    )?;
    get_session(c, id)
}

pub fn set_session_status(
    c: &Connection,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    c.execute(
        "UPDATE sessions SET status = ?1, error = ?2, updated_at = ?3 WHERE id = ?4",
        params![status, error, now(), id],
    )?;
    Ok(())
}

pub fn delete_session(c: &Connection, id: &str) -> Result<bool> {
    Ok(c.execute("DELETE FROM sessions WHERE id = ?1", [id])? > 0)
}

/// Sessions left "running" by a crash or forced exit are marked stopped.
pub fn reset_stale_running(c: &Connection) -> Result<usize> {
    Ok(c.execute(
        "UPDATE sessions SET status = 'stopped', error = 'Relay restarted while this task was running' WHERE status = 'running'",
        [],
    )?)
}

// ---------- messages ----------

const MESSAGE_COLS: &str = "id, session_id, role, content, reasoning, tool_calls, tool_call_id, tool_name, is_error, created_at";

fn message_row(r: &Row) -> rusqlite::Result<Message> {
    let tool_calls: Option<String> = r.get(5)?;
    Ok(Message {
        id: r.get(0)?,
        session_id: r.get(1)?,
        role: r.get(2)?,
        content: r.get(3)?,
        reasoning: r.get(4)?,
        tool_calls: tool_calls
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        tool_call_id: r.get(6)?,
        tool_name: r.get(7)?,
        is_error: r.get::<_, i64>(8)? != 0,
        created_at: r.get(9)?,
    })
}

pub fn insert_message(c: &Connection, session_id: &str, m: &NewMessage) -> Result<Message> {
    let created_at = now();
    let tool_calls = if m.tool_calls.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&m.tool_calls)?)
    };
    c.execute(
        "INSERT INTO messages (session_id, role, content, reasoning, tool_calls, tool_call_id, tool_name, is_error, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            session_id,
            m.role,
            m.content,
            m.reasoning,
            tool_calls,
            m.tool_call_id,
            m.tool_name,
            m.is_error as i64,
            created_at
        ],
    )?;
    let id = c.last_insert_rowid();
    c.execute(
        "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
        params![created_at, session_id],
    )?;
    Ok(Message {
        id,
        session_id: session_id.to_string(),
        role: m.role.clone(),
        content: m.content.clone(),
        reasoning: m.reasoning.clone(),
        tool_calls: m.tool_calls.clone(),
        tool_call_id: m.tool_call_id.clone(),
        tool_name: m.tool_name.clone(),
        is_error: m.is_error,
        created_at,
    })
}

/// All messages for a session in order (used to build the model context).
pub fn all_messages(c: &Connection, session_id: &str) -> Result<Vec<Message>> {
    let mut st = c.prepare(&format!(
        "SELECT {MESSAGE_COLS} FROM messages WHERE session_id = ?1 ORDER BY id"
    ))?;
    let rows = st
        .query_map([session_id], message_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// A page of messages in ascending id order.
/// - `before`: newest `limit` messages with id < before (scrolling back)
/// - `after`: oldest `limit` messages with id > after (catching up)
/// - neither: the newest `limit` messages
///
/// Returns (messages, has_more) where has_more refers to the paging direction.
pub fn page_messages(
    c: &Connection,
    session_id: &str,
    before: Option<i64>,
    after: Option<i64>,
    limit: usize,
) -> Result<(Vec<Message>, bool)> {
    let fetch = (limit + 1) as i64;
    let (sql, bound): (String, i64) = match (before, after) {
        (_, Some(a)) => (
            format!(
                "SELECT {MESSAGE_COLS} FROM messages WHERE session_id = ?1 AND id > ?2 ORDER BY id ASC LIMIT ?3"
            ),
            a,
        ),
        (Some(b), None) => (
            format!(
                "SELECT {MESSAGE_COLS} FROM messages WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
            ),
            b,
        ),
        (None, None) => (
            format!(
                "SELECT {MESSAGE_COLS} FROM messages WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
            ),
            i64::MAX,
        ),
    };
    let mut st = c.prepare(&sql)?;
    let mut rows = st
        .query_map(params![session_id, bound, fetch], message_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    if after.is_none() {
        rows.reverse();
    }
    Ok((rows, has_more))
}

// ---------- push tokens ----------

pub fn add_push_token(c: &Connection, token: &str) -> Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO push_tokens (token, created_at) VALUES (?1, ?2)",
        params![token, now()],
    )?;
    Ok(())
}

pub fn remove_push_token(c: &Connection, token: &str) -> Result<bool> {
    Ok(c.execute("DELETE FROM push_tokens WHERE token = ?1", [token])? > 0)
}

pub fn list_push_tokens(c: &Connection) -> Result<Vec<String>> {
    let mut st = c.prepare("SELECT token FROM push_tokens")?;
    let rows = st
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        c
    }

    #[test]
    fn paging() {
        let c = mem();
        let p = create_project(&c, "p", "C:\\x").unwrap();
        let s = create_session(&c, &p.id, "s").unwrap();
        for i in 0..10 {
            insert_message(
                &c,
                &s.id,
                &NewMessage {
                    role: "user".into(),
                    content: i.to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let (latest, more) = page_messages(&c, &s.id, None, None, 4).unwrap();
        assert!(more);
        assert_eq!(
            latest
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            ["6", "7", "8", "9"]
        );
        let (older, more) = page_messages(&c, &s.id, Some(latest[0].id), None, 4).unwrap();
        assert!(more);
        assert_eq!(older[0].content, "2");
        let (newer, more) = page_messages(&c, &s.id, None, Some(latest[1].id), 10).unwrap();
        assert!(!more);
        assert_eq!(newer.len(), 2);
    }

    #[test]
    fn cascade_delete_keeps_nothing() {
        let c = mem();
        let p = create_project(&c, "p", "C:\\x").unwrap();
        let s = create_session(&c, &p.id, "s").unwrap();
        insert_message(
            &c,
            &s.id,
            &NewMessage {
                role: "user".into(),
                content: "hi".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(delete_project(&c, &p.id).unwrap());
        assert!(get_session(&c, &s.id).unwrap().is_none());
        assert!(all_messages(&c, &s.id).unwrap().is_empty());
    }

    #[test]
    fn tool_calls_roundtrip() {
        let c = mem();
        let p = create_project(&c, "p", "C:\\x").unwrap();
        let s = create_session(&c, &p.id, "s").unwrap();
        let tc = ToolCall {
            id: "a".into(),
            name: "read_file".into(),
            arguments: "{\"path\":\"x\"}".into(),
        };
        insert_message(
            &c,
            &s.id,
            &NewMessage {
                role: "assistant".into(),
                tool_calls: vec![tc.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        let all = all_messages(&c, &s.id).unwrap();
        assert_eq!(all[0].tool_calls, vec![tc]);
    }
}
