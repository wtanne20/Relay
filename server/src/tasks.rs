//! Background tasks on the brain.
//!
//! - **Queue**: prompts that run one after another, each in its own chat (or
//!   as the next message of a chat that is busy), while nobody watches. One
//!   queued task runs at a time so the GPU isn't shared between them.
//! - **Schedules**: a prompt that is added to the queue at set times (once,
//!   on chosen days at a time of day, or every N minutes).
//! - **Notices**: a short log of finished tasks and approval requests (names
//!   and status only), which a phone without a push service checks now and
//!   then to notify about work that started while the app was closed.
//!
//! Times of day are the brain's local time; stored times are UTC.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::agent::{AgentManager, StartError};
use crate::api::{ApiError, AppState, JsonBody};
use crate::db::{self, Db};
use crate::push::Pusher;

type ApiResult<T> = Result<T, ApiError>;

/// How often the runner looks for due schedules and finished tasks when
/// nothing pokes it.
const TICK: Duration = Duration::from_secs(5);
/// Finished queue items and notices kept for the lists.
const KEEP_FINISHED: i64 = 100;
const KEEP_NOTICES: i64 = 200;
pub const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_NAME_CHARS: usize = 120;
/// The shortest repeat interval.
pub const MIN_EVERY_MINUTES: u32 = 5;

// ---------- schema ----------

pub fn ensure_schema(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS schedules (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            prompt TEXT NOT NULL,
            repeat TEXT NOT NULL,
            same_chat INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            next_run TEXT,
            last_run TEXT,
            last_session_id TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS queue (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            session_id TEXT,
            schedule_id TEXT,
            name TEXT NOT NULL,
            prompt TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'queued',
            error TEXT,
            position INTEGER NOT NULL DEFAULT 0,
            follow_up INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            started_at TEXT,
            finished_at TEXT
        );
        CREATE INDEX IF NOT EXISTS queue_status ON queue(status, position);
        CREATE TABLE IF NOT EXISTS notices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL,
            status TEXT NOT NULL,
            project_id TEXT NOT NULL,
            project_name TEXT NOT NULL,
            session_id TEXT NOT NULL,
            title TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// Queue items left running by a restart are marked stopped.
pub fn reset_stale(c: &Connection) -> Result<usize> {
    Ok(c.execute(
        "UPDATE queue SET status = 'stopped', error = 'Relay restarted while this task was running', finished_at = ?1 WHERE status = 'running'",
        [db::now()],
    )?)
}

// ---------- repeat rules ----------

/// When a schedule runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repeat {
    /// One time, at a local date and time ("2026-11-12T08:00").
    Once { at: String },
    /// At a local time of day ("08:00") on the given weekdays (1 = Monday
    /// .. 7 = Sunday; empty means every day).
    Daily {
        time: String,
        #[serde(default)]
        days: Vec<u8>,
    },
    /// Every so many minutes.
    Every { minutes: u32 },
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

fn parse_local(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M"))
        .ok()
}

/// A local wall-clock time as an instant. In a daylight-saving gap the
/// time doesn't exist, so the hour after is used.
fn local_instant<Tz: TimeZone>(tz: &Tz, t: NaiveDateTime) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&t)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(t + chrono::Duration::hours(1)))
                .earliest()
        })
        .map(|d| d.with_timezone(&Utc))
}

impl Repeat {
    pub fn validate(&self) -> Result<()> {
        match self {
            Repeat::Once { at } => {
                if parse_local(at).is_none() {
                    bail!("use a date and time like 2026-11-12T08:00");
                }
            }
            Repeat::Daily { time, days } => {
                if parse_time(time).is_none() {
                    bail!("use a time of day like 08:00");
                }
                if days.iter().any(|d| !(1..=7).contains(d)) {
                    bail!("days are 1 (Monday) to 7 (Sunday)");
                }
            }
            Repeat::Every { minutes } => {
                if *minutes < MIN_EVERY_MINUTES {
                    bail!("repeat at most every {MIN_EVERY_MINUTES} minutes");
                }
                if *minutes > 60 * 24 * 31 {
                    bail!("repeat at least once a month");
                }
            }
        }
        Ok(())
    }

    /// The first run strictly after `after`, in time zone `tz`. None when a
    /// one-time schedule has passed.
    pub fn next_after<Tz: TimeZone>(&self, tz: &Tz, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Repeat::Once { at } => {
                let t = local_instant(tz, parse_local(at)?)?;
                (t > after).then_some(t)
            }
            Repeat::Daily { time, days } => {
                let time = parse_time(time)?;
                let start: NaiveDate = after.with_timezone(tz).date_naive();
                (0..9).find_map(|d| {
                    let date = start + chrono::Duration::days(d);
                    let wd = date.weekday().number_from_monday() as u8;
                    if !days.is_empty() && !days.contains(&wd) {
                        return None;
                    }
                    local_instant(tz, date.and_time(time)).filter(|t| *t > after)
                })
            }
            Repeat::Every { minutes } => {
                Some(after + chrono::Duration::minutes(i64::from(*minutes)))
            }
        }
    }

    fn next_local(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.next_after(&Local, after)
    }
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
fn parse_stamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

// ---------- rows ----------

#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub name: String,
    pub prompt: String,
    pub repeat: Repeat,
    /// Every run continues one chat instead of starting a new one.
    pub same_chat: bool,
    pub enabled: bool,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_session_id: Option<String>,
    /// The latest run's chat status ("done", "error", "running", ...).
    pub last_status: Option<String>,
    pub created_at: String,
}

const SCHEDULE_SELECT: &str = "SELECT s.id, s.project_id, p.name, s.name, s.prompt, s.repeat, s.same_chat, s.enabled, s.next_run, s.last_run, s.last_session_id, x.status, s.created_at
    FROM schedules s JOIN projects p ON p.id = s.project_id LEFT JOIN sessions x ON x.id = s.last_session_id";

fn schedule_row(r: &Row) -> rusqlite::Result<Schedule> {
    let repeat: String = r.get(5)?;
    Ok(Schedule {
        id: r.get(0)?,
        project_id: r.get(1)?,
        project_name: r.get(2)?,
        name: r.get(3)?,
        prompt: r.get(4)?,
        repeat: serde_json::from_str(&repeat).unwrap_or(Repeat::Every { minutes: 60 * 24 }),
        same_chat: r.get::<_, i64>(6)? != 0,
        enabled: r.get::<_, i64>(7)? != 0,
        next_run: r.get(8)?,
        last_run: r.get(9)?,
        last_session_id: r.get(10)?,
        last_status: r.get(11)?,
        created_at: r.get(12)?,
    })
}

pub fn list_schedules(c: &Connection) -> Result<Vec<Schedule>> {
    let mut st = c.prepare(&format!(
        "{SCHEDULE_SELECT} ORDER BY s.enabled DESC, s.next_run IS NULL, s.next_run, s.created_at"
    ))?;
    let rows = st
        .query_map([], schedule_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_schedule(c: &Connection, id: &str) -> Result<Option<Schedule>> {
    Ok(c.query_row(
        &format!("{SCHEDULE_SELECT} WHERE s.id = ?1"),
        [id],
        schedule_row,
    )
    .optional()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueItem {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    /// The chat it runs in (set once it starts, or from the start when it
    /// continues an existing chat).
    pub session_id: Option<String>,
    pub session_title: Option<String>,
    pub schedule_id: Option<String>,
    /// A message for a chat that was busy: it is sent as soon as that chat
    /// is free, without waiting for other queued tasks.
    pub follow_up: bool,
    pub name: String,
    pub prompt: String,
    /// queued, running, done, error or stopped.
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// While running: waiting for the user to allow a tool call.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub needs_approval: bool,
}

const QUEUE_SELECT: &str = "SELECT q.id, q.project_id, p.name, q.session_id, x.title, q.schedule_id, q.name, q.prompt, q.status, q.error, q.created_at, q.started_at, q.finished_at, q.follow_up
    FROM queue q JOIN projects p ON p.id = q.project_id LEFT JOIN sessions x ON x.id = q.session_id";

fn queue_row(r: &Row) -> rusqlite::Result<QueueItem> {
    Ok(QueueItem {
        id: r.get(0)?,
        project_id: r.get(1)?,
        project_name: r.get(2)?,
        session_id: r.get(3)?,
        session_title: r.get(4)?,
        schedule_id: r.get(5)?,
        name: r.get(6)?,
        prompt: r.get(7)?,
        status: r.get(8)?,
        error: r.get(9)?,
        created_at: r.get(10)?,
        started_at: r.get(11)?,
        finished_at: r.get(12)?,
        follow_up: r.get::<_, i64>(13)? != 0,
        needs_approval: false,
    })
}

/// Running first, then queued in order, then the latest finished.
pub fn list_queue(c: &Connection) -> Result<Vec<QueueItem>> {
    let mut st = c.prepare(&format!(
        "{QUEUE_SELECT} ORDER BY CASE q.status WHEN 'running' THEN 0 WHEN 'queued' THEN 1 ELSE 2 END,
            CASE WHEN q.status IN ('running', 'queued') THEN q.position END,
            q.finished_at DESC LIMIT 200"
    ))?;
    let rows = st
        .query_map([], queue_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_queue_item(c: &Connection, id: &str) -> Result<Option<QueueItem>> {
    Ok(
        c.query_row(&format!("{QUEUE_SELECT} WHERE q.id = ?1"), [id], queue_row)
            .optional()?,
    )
}

fn queued_in_order(c: &Connection) -> Result<Vec<QueueItem>> {
    let mut st = c.prepare(&format!(
        "{QUEUE_SELECT} WHERE q.status = 'queued' ORDER BY q.position, q.created_at"
    ))?;
    let rows = st
        .query_map([], queue_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn running_items(c: &Connection) -> Result<Vec<QueueItem>> {
    let mut st = c.prepare(&format!("{QUEUE_SELECT} WHERE q.status = 'running'"))?;
    let rows = st
        .query_map([], queue_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub struct NewItem {
    pub project_id: String,
    pub session_id: Option<String>,
    pub schedule_id: Option<String>,
    pub follow_up: bool,
    pub name: String,
    pub prompt: String,
}

pub fn enqueue(c: &Connection, n: &NewItem) -> Result<QueueItem> {
    let id = uuid::Uuid::new_v4().to_string();
    let position: i64 = c.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM queue",
        [],
        |r| r.get(0),
    )?;
    c.execute(
        "INSERT INTO queue (id, project_id, session_id, schedule_id, name, prompt, status, position, created_at, follow_up) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?8, ?9)",
        params![id, n.project_id, n.session_id, n.schedule_id, n.name, n.prompt, position, db::now(), n.follow_up as i64],
    )?;
    get_queue_item(c, &id)?.ok_or_else(|| anyhow::anyhow!("queued item vanished"))
}

/// Moves a queued item to the front of the line.
pub fn move_first(c: &Connection, id: &str) -> Result<bool> {
    let front: i64 = c.query_row(
        "SELECT COALESCE(MIN(position), 1) - 1 FROM queue WHERE status = 'queued'",
        [],
        |r| r.get(0),
    )?;
    Ok(c.execute(
        "UPDATE queue SET position = ?1 WHERE id = ?2 AND status = 'queued'",
        params![front, id],
    )? > 0)
}

fn finish_item(c: &Connection, id: &str, status: &str, error: Option<&str>) -> Result<()> {
    c.execute(
        "UPDATE queue SET status = ?1, error = ?2, finished_at = ?3 WHERE id = ?4",
        params![status, error, db::now(), id],
    )?;
    // Keep the list short.
    c.execute(
        "DELETE FROM queue WHERE status NOT IN ('queued', 'running') AND id NOT IN (
            SELECT id FROM queue WHERE status NOT IN ('queued', 'running') ORDER BY finished_at DESC LIMIT ?1)",
        [KEEP_FINISHED],
    )?;
    Ok(())
}

/// Removes finished items from the list.
pub fn clear_finished(c: &Connection) -> Result<usize> {
    Ok(c.execute(
        "DELETE FROM queue WHERE status NOT IN ('queued', 'running')",
        [],
    )?)
}

// ---------- notices ----------

#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub id: i64,
    pub at: String,
    /// done, error, stopped or approval.
    pub status: String,
    pub project_id: String,
    pub project_name: String,
    pub session_id: String,
    pub title: String,
}

pub fn add_notice(
    c: &Connection,
    status: &str,
    project_id: &str,
    project_name: &str,
    session_id: &str,
    title: &str,
) -> Result<()> {
    c.execute(
        "INSERT INTO notices (at, status, project_id, project_name, session_id, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![db::now(), status, project_id, project_name, session_id, title],
    )?;
    c.execute(
        "DELETE FROM notices WHERE id <= (SELECT MAX(id) FROM notices) - ?1",
        [KEEP_NOTICES],
    )?;
    Ok(())
}

/// Notices newer than `after`, oldest first, and the newest id.
pub fn notices_after(c: &Connection, after: i64, limit: usize) -> Result<(Vec<Notice>, i64)> {
    let last: i64 = c.query_row("SELECT COALESCE(MAX(id), 0) FROM notices", [], |r| r.get(0))?;
    let mut st = c.prepare(
        "SELECT id, at, status, project_id, project_name, session_id, title FROM notices WHERE id > ?1 ORDER BY id LIMIT ?2",
    )?;
    let rows = st
        .query_map(params![after, limit as i64], |r| {
            Ok(Notice {
                id: r.get(0)?,
                at: r.get(1)?,
                status: r.get(2)?,
                project_id: r.get(3)?,
                project_name: r.get(4)?,
                session_id: r.get(5)?,
                title: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows, last))
}

// ---------- the runner ----------

/// Starts queued tasks one at a time and adds scheduled runs to the queue.
pub struct Tasks {
    db: Db,
    agent: Arc<AgentManager>,
    pusher: Pusher,
    poke: Notify,
}

impl Tasks {
    pub fn new(db: Db, agent: Arc<AgentManager>, pusher: Pusher) -> Arc<Self> {
        Arc::new(Self {
            db,
            agent,
            pusher,
            poke: Notify::new(),
        })
    }

    /// Look at the queue now instead of at the next tick.
    pub fn poke(&self) {
        self.poke.notify_one();
    }

    /// Runs until `shutdown` is cancelled.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = this.tick(Utc::now()).await {
                    tracing::warn!("task runner: {e:#}");
                }
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = this.poke.notified() => {}
                    _ = this.agent.finished.notified() => {}
                    _ = tokio::time::sleep(TICK) => {}
                }
            }
        });
    }

    /// One pass: queue due schedules, record finished tasks, start the next.
    pub async fn tick(&self, now: DateTime<Utc>) -> Result<()> {
        self.queue_due(now).await?;
        let running = self.db.call(|c| running_items(c)).await?;
        let mut busy = false;
        for item in running {
            let Some(sid) = item.session_id.clone() else {
                let id = item.id.clone();
                self.db
                    .call(move |c| finish_item(c, &id, "error", Some("no chat")))
                    .await?;
                continue;
            };
            if self.agent.is_running(&sid) {
                // Follow-ups are part of a conversation, not background work.
                busy |= !item.follow_up;
                continue;
            }
            let id = item.id.clone();
            self.db
                .call(move |c| {
                    let (status, error) = match db::get_session(c, &sid)? {
                        Some(s) if s.status == "running" => return Ok(()),
                        Some(s) => (
                            if s.status == "idle" {
                                "done".into()
                            } else {
                                s.status
                            },
                            s.error,
                        ),
                        None => ("stopped".into(), Some("the chat was deleted".into())),
                    };
                    finish_item(c, &id, &status, error.as_deref())
                })
                .await?;
        }
        self.start_next(busy).await
    }

    async fn queue_due(&self, now: DateTime<Utc>) -> Result<()> {
        let now_s = stamp(now);
        self.db
            .call(move |c| {
                let due: Vec<Schedule> = list_schedules(c)?
                    .into_iter()
                    .filter(|s| s.enabled && s.next_run.as_deref().is_some_and(|n| n <= now_s.as_str()))
                    .collect();
                for s in due {
                    let next = s.repeat.next_local(now);
                    // A run still waiting in line isn't added twice (the PC
                    // was busy, or asleep through several runs).
                    let waiting: bool = c
                        .prepare("SELECT 1 FROM queue WHERE schedule_id = ?1 AND status = 'queued'")?
                        .exists([&s.id])?;
                    if !waiting {
                        let session_id = if s.same_chat {
                            s.last_session_id.clone().filter(|id| db::get_session(c, id).ok().flatten().is_some())
                        } else {
                            None
                        };
                        enqueue(
                            c,
                            &NewItem {
                                project_id: s.project_id.clone(),
                                session_id,
                                schedule_id: Some(s.id.clone()),
                                follow_up: false,
                                name: s.name.clone(),
                                prompt: s.prompt.clone(),
                            },
                        )?;
                        tracing::info!(schedule = %s.name, "scheduled task queued");
                    }
                    c.execute(
                        "UPDATE schedules SET last_run = ?1, next_run = ?2, enabled = ?3 WHERE id = ?4",
                        params![now_s, next.map(stamp), next.is_some() as i64, s.id],
                    )?;
                }
                Ok(())
            })
            .await
    }

    /// Starts every follow-up whose chat is free, and the next background
    /// task unless one is already running (`busy`).
    async fn start_next(&self, mut busy: bool) -> Result<()> {
        let queued = self.db.call(|c| queued_in_order(c)).await?;
        for item in queued {
            if busy && !item.follow_up {
                continue;
            }
            // A follow-up for a chat that's busy waits for it to finish.
            if let Some(sid) = &item.session_id
                && self.agent.is_running(sid)
            {
                continue;
            }
            let sid = match item.session_id.clone() {
                Some(s) => s,
                None => {
                    let (pid, name, id) =
                        (item.project_id.clone(), item.name.clone(), item.id.clone());
                    self.db
                        .call(move |c| {
                            let title = if name.trim().is_empty() {
                                "Queued task"
                            } else {
                                name.trim()
                            };
                            let s = db::create_session(c, &pid, title, name.trim().is_empty())?;
                            c.execute(
                                "UPDATE queue SET session_id = ?1 WHERE id = ?2",
                                params![s.id, id],
                            )?;
                            Ok(s.id)
                        })
                        .await?
                }
            };
            match self
                .agent
                .start(&sid, item.prompt.clone(), Vec::new(), Vec::new(), false)
                .await
            {
                Ok(_) => {
                    let (id, sid2, sched) =
                        (item.id.clone(), sid.clone(), item.schedule_id.clone());
                    self.db
                        .call(move |c| {
                            c.execute(
                                "UPDATE queue SET status = 'running', started_at = ?1, session_id = ?2 WHERE id = ?3",
                                params![db::now(), sid2, id],
                            )?;
                            db::set_session_archived(c, &sid2, false)?;
                            if let Some(s) = sched {
                                c.execute(
                                    "UPDATE schedules SET last_session_id = ?1 WHERE id = ?2",
                                    params![sid2, s],
                                )?;
                            }
                            Ok(())
                        })
                        .await?;
                    tracing::info!(task = %item.name, session = %sid, "queued task started");
                    busy |= !item.follow_up;
                }
                // The model is switching for another chat; try again later.
                Err(StartError::Busy(_)) => busy = true,
                // The user started something in that chat meanwhile.
                Err(StartError::AlreadyRunning) => continue,
                Err(e) => {
                    let msg = e.to_string();
                    let (id, m) = (item.id.clone(), msg.clone());
                    self.db
                        .call(move |c| finish_item(c, &id, "error", Some(&m)))
                        .await?;
                    tracing::warn!(task = %item.name, "queued task could not start: {msg}");
                    let pid = item.project_id.clone();
                    if let Ok(Some(p)) = self.db.call(move |c| db::get_project(c, &pid)).await {
                        let title = if item.name.is_empty() {
                            "A queued task".to_string()
                        } else {
                            item.name.clone()
                        };
                        self.pusher.notify(&p, &sid, &title, "error");
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------- HTTP ----------

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/queue", get(get_queue).post(add_to_queue))
        .route("/queue/clear", post(clear_queue))
        .route("/queue/{id}", axum::routing::delete(remove_from_queue))
        .route("/queue/{id}/first", post(queue_first))
        .route("/schedules", get(get_schedules).post(create_schedule))
        .route(
            "/schedules/{id}",
            get(one_schedule)
                .patch(update_schedule)
                .delete(delete_schedule),
        )
        .route("/schedules/{id}/run", post(run_schedule))
        .route("/notices", get(get_notices))
}

fn clean_name(name: Option<String>) -> ApiResult<String> {
    let n = name.unwrap_or_default().trim().to_string();
    if n.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest(format!(
            "keep the name under {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(n)
}

fn clean_prompt(p: &str) -> ApiResult<String> {
    let p = p.trim();
    if p.is_empty() {
        return Err(ApiError::BadRequest("say what the task should do".into()));
    }
    if p.chars().count() > MAX_PROMPT_CHARS {
        return Err(ApiError::BadRequest(format!(
            "keep the prompt under {MAX_PROMPT_CHARS} characters"
        )));
    }
    Ok(p.to_string())
}

/// A name from the prompt's first line when none is given.
fn name_from_prompt(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        out = out.trim_end().to_string() + "…";
    }
    out
}

async fn get_queue(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let mut items = s.db.call(|c| list_queue(c)).await?;
    for it in &mut items {
        if it.status == "running"
            && let Some(sid) = &it.session_id
        {
            it.needs_approval = s.agent.pending_approval(sid).is_some();
        }
    }
    let queued = items.iter().filter(|i| i.status == "queued").count();
    let running = items.iter().filter(|i| i.status == "running").count();
    Ok(Json(
        json!({ "items": items, "queued": queued, "running": running }),
    ))
}

#[derive(Deserialize)]
struct AddToQueue {
    #[serde(default)]
    project_id: Option<String>,
    /// Run as the next message of this chat instead of in a new chat.
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    prompt: String,
}

async fn add_to_queue(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<AddToQueue>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let prompt = clean_prompt(&body.prompt)?;
    let given = clean_name(body.name)?;
    let session_id = body.session_id.filter(|x| !x.trim().is_empty());
    let project_id = body.project_id.filter(|x| !x.trim().is_empty());
    let item =
        s.db.call(move |c| {
            let (project_id, session_id, name) = match session_id {
                Some(sid) => {
                    let Some(sess) = db::get_session(c, &sid)? else {
                        return Ok(Err(ApiError::NotFound("session")));
                    };
                    let name = if given.is_empty() {
                        sess.title.clone()
                    } else {
                        given
                    };
                    (sess.project_id, Some(sid), name)
                }
                None => {
                    let pid = project_id.unwrap_or_else(|| db::CHATS_PROJECT_ID.to_string());
                    if db::get_project(c, &pid)?.is_none() {
                        return Ok(Err(ApiError::NotFound("project")));
                    }
                    let name = if given.is_empty() {
                        name_from_prompt(&prompt)
                    } else {
                        given
                    };
                    (pid, None, name)
                }
            };
            let follow_up = session_id.is_some();
            Ok(Ok(enqueue(
                c,
                &NewItem {
                    project_id,
                    session_id,
                    schedule_id: None,
                    follow_up,
                    name,
                    prompt,
                },
            )?))
        })
        .await??;
    s.tasks.poke();
    Ok((StatusCode::CREATED, Json(json!({ "item": item }))))
}

/// Takes a queued task out of the line, stops a running one, or removes a
/// finished one from the list.
async fn remove_from_queue(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let qid = id.clone();
    let item =
        s.db.call(move |c| get_queue_item(c, &qid))
            .await?
            .ok_or(ApiError::NotFound("queued task"))?;
    if item.status == "running" {
        let stopping = item
            .session_id
            .as_deref()
            .is_some_and(|sid| s.agent.stop(sid));
        s.tasks.poke();
        return Ok(Json(json!({ "ok": true, "stopping": stopping })));
    }
    s.db.call(move |c| Ok(c.execute("DELETE FROM queue WHERE id = ?1", [id])?))
        .await?;
    Ok(Json(json!({ "ok": true })))
}

async fn queue_first(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    if !s.db.call(move |c| move_first(c, &id)).await? {
        return Err(ApiError::Conflict(
            "only a task that's waiting can move".into(),
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

async fn clear_queue(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let removed = s.db.call(|c| clear_finished(c)).await?;
    Ok(Json(json!({ "removed": removed })))
}

async fn get_schedules(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let schedules = s.db.call(|c| list_schedules(c)).await?;
    Ok(Json(
        json!({ "schedules": schedules, "timezone": Local::now().format("%:z").to_string() }),
    ))
}

async fn one_schedule(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let schedule =
        s.db.call(move |c| get_schedule(c, &id))
            .await?
            .ok_or(ApiError::NotFound("schedule"))?;
    Ok(Json(json!({ "schedule": schedule })))
}

#[derive(Deserialize)]
struct ScheduleBody {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    repeat: Option<Repeat>,
    #[serde(default)]
    same_chat: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
}

fn next_for(repeat: &Repeat) -> ApiResult<String> {
    repeat
        .next_local(Utc::now())
        .map(stamp)
        .ok_or_else(|| ApiError::BadRequest("that time has already passed".into()))
}

async fn create_schedule(
    State(s): State<AppState>,
    JsonBody(body): JsonBody<ScheduleBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let prompt = clean_prompt(body.prompt.as_deref().unwrap_or(""))?;
    let name = clean_name(body.name)?;
    let name = if name.is_empty() {
        name_from_prompt(&prompt)
    } else {
        name
    };
    let repeat = body
        .repeat
        .ok_or_else(|| ApiError::BadRequest("say when the task runs".into()))?;
    repeat
        .validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let enabled = body.enabled.unwrap_or(true);
    let next = if enabled {
        Some(next_for(&repeat)?)
    } else {
        None
    };
    let pid = body
        .project_id
        .filter(|x| !x.trim().is_empty())
        .unwrap_or_else(|| db::CHATS_PROJECT_ID.to_string());
    let same_chat = body.same_chat.unwrap_or(false);
    let repeat_json =
        serde_json::to_string(&repeat).map_err(|e| ApiError::Internal(e.to_string()))?;
    let schedule = s
        .db
        .call(move |c| {
            if db::get_project(c, &pid)?.is_none() {
                return Ok(None);
            }
            let id = uuid::Uuid::new_v4().to_string();
            c.execute(
                "INSERT INTO schedules (id, project_id, name, prompt, repeat, same_chat, enabled, next_run, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, pid, name, prompt, repeat_json, same_chat as i64, enabled as i64, next, db::now()],
            )?;
            get_schedule(c, &id)
        })
        .await?
        .ok_or(ApiError::NotFound("project"))?;
    Ok((StatusCode::CREATED, Json(json!({ "schedule": schedule }))))
}

async fn update_schedule(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
    JsonBody(body): JsonBody<ScheduleBody>,
) -> ApiResult<Json<Value>> {
    let sid = id.clone();
    let current =
        s.db.call(move |c| get_schedule(c, &sid))
            .await?
            .ok_or(ApiError::NotFound("schedule"))?;
    let prompt = match &body.prompt {
        Some(p) => clean_prompt(p)?,
        None => current.prompt.clone(),
    };
    let name = match body.name {
        Some(n) => {
            let n = clean_name(Some(n))?;
            if n.is_empty() {
                name_from_prompt(&prompt)
            } else {
                n
            }
        }
        None => current.name.clone(),
    };
    let repeat = body.repeat.unwrap_or(current.repeat.clone());
    repeat
        .validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let enabled = body.enabled.unwrap_or(current.enabled);
    // The next run moves when the timing changes or the schedule is turned on.
    let next = if !enabled {
        None
    } else if repeat != current.repeat || !current.enabled || current.next_run.is_none() {
        Some(next_for(&repeat)?)
    } else {
        current.next_run.clone()
    };
    let pid = body
        .project_id
        .filter(|x| !x.trim().is_empty())
        .unwrap_or(current.project_id.clone());
    let same_chat = body.same_chat.unwrap_or(current.same_chat);
    let repeat_json =
        serde_json::to_string(&repeat).map_err(|e| ApiError::Internal(e.to_string()))?;
    let schedule = s
        .db
        .call(move |c| {
            if db::get_project(c, &pid)?.is_none() {
                return Ok(Err(ApiError::NotFound("project")));
            }
            c.execute(
                "UPDATE schedules SET project_id = ?1, name = ?2, prompt = ?3, repeat = ?4, same_chat = ?5, enabled = ?6, next_run = ?7 WHERE id = ?8",
                params![pid, name, prompt, repeat_json, same_chat as i64, enabled as i64, next, id],
            )?;
            Ok(get_schedule(c, &id)?.ok_or(ApiError::NotFound("schedule")))
        })
        .await??;
    Ok(Json(json!({ "schedule": schedule })))
}

async fn delete_schedule(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<Json<Value>> {
    let removed =
        s.db.call(move |c| {
            // Runs still waiting go too; running and finished ones stay.
            c.execute(
                "DELETE FROM queue WHERE schedule_id = ?1 AND status = 'queued'",
                [&id],
            )?;
            Ok(c.execute("DELETE FROM schedules WHERE id = ?1", [&id])? > 0)
        })
        .await?;
    if !removed {
        return Err(ApiError::NotFound("schedule"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// Adds a run to the queue now, without changing the schedule's timing.
async fn run_schedule(
    State(s): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let item =
        s.db.call(move |c| {
            let Some(sch) = get_schedule(c, &id)? else {
                return Ok(None);
            };
            let session_id = if sch.same_chat {
                sch.last_session_id
                    .filter(|x| db::get_session(c, x).ok().flatten().is_some())
            } else {
                None
            };
            Ok(Some(enqueue(
                c,
                &NewItem {
                    project_id: sch.project_id,
                    session_id,
                    schedule_id: Some(sch.id),
                    follow_up: false,
                    name: sch.name,
                    prompt: sch.prompt,
                },
            )?))
        })
        .await?
        .ok_or(ApiError::NotFound("schedule"))?;
    s.tasks.poke();
    Ok((StatusCode::CREATED, Json(json!({ "item": item }))))
}

#[derive(Deserialize)]
struct NoticeQuery {
    after: Option<i64>,
    limit: Option<usize>,
}

/// What finished or asked for approval since `after` (an id from an
/// earlier answer). Without `after`, only the newest id, so a phone starts
/// from now instead of notifying about old news.
async fn get_notices(
    State(s): State<AppState>,
    Query(q): Query<NoticeQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let (notices, last) = match q.after {
        Some(after) => s.db.call(move |c| notices_after(c, after, limit)).await?,
        None => {
            let (_, last) = s.db.call(|c| notices_after(c, i64::MAX, 1)).await?;
            (Vec::new(), last)
        }
    };
    Ok(Json(json!({ "notices": notices, "last_id": last })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<Utc> {
        parse_stamp(s).expect("time")
    }

    #[test]
    fn daily_picks_the_next_matching_day() {
        let tz = FixedOffset::east_opt(-5 * 3600).expect("tz");
        // Wednesday 2026-11-11 10:00 local (15:00 UTC).
        let now = at("2026-11-11T15:00:00Z");
        let every = Repeat::Daily {
            time: "08:00".into(),
            days: vec![],
        };
        assert_eq!(every.next_after(&tz, now), Some(at("2026-11-12T13:00:00Z")));
        let later_today = Repeat::Daily {
            time: "18:30".into(),
            days: vec![],
        };
        assert_eq!(
            later_today.next_after(&tz, now),
            Some(at("2026-11-11T23:30:00Z"))
        );
        let weekdays = Repeat::Daily {
            time: "08:00".into(),
            days: vec![1, 2, 3, 4, 5],
        };
        // Friday 2026-11-13 after 08:00 -> Monday 2026-11-16.
        assert_eq!(
            weekdays.next_after(&tz, at("2026-11-13T14:00:00Z")),
            Some(at("2026-11-16T13:00:00Z"))
        );
        let sundays = Repeat::Daily {
            time: "09:00".into(),
            days: vec![7],
        };
        assert_eq!(
            sundays.next_after(&tz, now),
            Some(at("2026-11-15T14:00:00Z"))
        );
    }

    #[test]
    fn once_and_every() {
        let tz = FixedOffset::east_opt(0).expect("tz");
        let now = at("2026-11-11T15:00:00Z");
        let once = Repeat::Once {
            at: "2026-11-12T08:00".into(),
        };
        assert_eq!(once.next_after(&tz, now), Some(at("2026-11-12T08:00:00Z")));
        assert_eq!(once.next_after(&tz, at("2026-11-12T08:00:00Z")), None);
        let every = Repeat::Every { minutes: 90 };
        assert_eq!(every.next_after(&tz, now), Some(at("2026-11-11T16:30:00Z")));
    }

    #[test]
    fn validation() {
        assert!(
            Repeat::Daily {
                time: "25:00".into(),
                days: vec![]
            }
            .validate()
            .is_err()
        );
        assert!(
            Repeat::Daily {
                time: "07:30".into(),
                days: vec![8]
            }
            .validate()
            .is_err()
        );
        assert!(Repeat::Every { minutes: 1 }.validate().is_err());
        assert!(
            Repeat::Once {
                at: "tomorrow".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Repeat::Once {
                at: "2026-11-12T08:00".into()
            }
            .validate()
            .is_ok()
        );
        let parsed: Repeat =
            serde_json::from_value(json!({"kind": "daily", "time": "08:00"})).expect("json");
        assert_eq!(
            parsed,
            Repeat::Daily {
                time: "08:00".into(),
                days: vec![]
            }
        );
    }

    #[test]
    fn queue_order_and_notices() {
        let c = Connection::open_in_memory().expect("db");
        c.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE sessions (id TEXT PRIMARY KEY, project_id TEXT, title TEXT, status TEXT, error TEXT);
             INSERT INTO projects VALUES ('p', 'Relay', '', '');",
        )
        .expect("schema");
        ensure_schema(&c).expect("tasks schema");
        let add = |name: &str| {
            enqueue(
                &c,
                &NewItem {
                    project_id: "p".into(),
                    session_id: None,
                    schedule_id: None,
                    follow_up: false,
                    name: name.into(),
                    prompt: "do it".into(),
                },
            )
            .expect("enqueue")
        };
        let a = add("a");
        let _b = add("b");
        let c3 = add("c");
        assert!(move_first(&c, &c3.id).expect("move"));
        let order: Vec<String> = queued_in_order(&c)
            .expect("list")
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(order, ["c", "a", "b"]);
        finish_item(&c, &a.id, "done", None).expect("finish");
        assert_eq!(
            list_queue(&c)
                .expect("list")
                .last()
                .map(|i| i.status.clone()),
            Some("done".into())
        );
        assert_eq!(clear_finished(&c).expect("clear"), 1);

        add_notice(&c, "done", "p", "Relay", "s1", "Morning summary").expect("notice");
        add_notice(&c, "approval", "p", "Relay", "s2", "Fix build").expect("notice");
        let (all, last) = notices_after(&c, 0, 10).expect("notices");
        assert_eq!(all.len(), 2);
        let (newer, _) = notices_after(&c, all[0].id, 10).expect("notices");
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].status, "approval");
        assert_eq!(last, all[1].id);
    }

    #[test]
    fn names_from_prompts() {
        assert_eq!(
            name_from_prompt("Summarize my repo\nand more"),
            "Summarize my repo"
        );
        let long = "a".repeat(60);
        assert!(name_from_prompt(&long).ends_with('…'));
    }
}
