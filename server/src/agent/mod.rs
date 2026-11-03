//! The agent: background tasks that run the model/tool loop for a session,
//! independent of any connected client, with per-session event broadcast.

pub mod llm;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::{DEFAULT_SYSTEM_PROMPT, LoadedConfig};
use crate::db::{self, Db, Message, NewMessage, Project, Session};
use crate::model_manager::{ModelManager, ModelState};
use crate::push::Pusher;
use crate::skills;
use crate::tools::{self, Sandbox, ToolContext};

/// Tool results in WebSocket events are cut to this length; the full result
/// is stored and available via the messages endpoint.
pub const EVENT_RESULT_CHARS: usize = 2000;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status {
        status: String,
        error: Option<String>,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        tool_call_id: String,
        name: String,
        content: String,
        is_error: bool,
        truncated: bool,
    },
    Message {
        message: Message,
    },
    Final {
        content: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Draft {
    pub content: String,
    pub reasoning: String,
}

struct RunningTask {
    cancel: CancellationToken,
    draft: Arc<Mutex<Draft>>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("session not found")]
    NotFound,
    #[error("a task is already running in this session")]
    AlreadyRunning,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub struct AgentManager {
    db: Db,
    model: ModelManager,
    cfg: Arc<LoadedConfig>,
    http: reqwest::Client,
    pusher: Pusher,
    channels: Mutex<HashMap<String, broadcast::Sender<Event>>>,
    running: Mutex<HashMap<String, RunningTask>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

enum LoopEnd {
    Done,
    Stopped,
}

impl AgentManager {
    pub fn new(db: Db, model: ModelManager, cfg: Arc<LoadedConfig>, pusher: Pusher) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Arc::new(Self {
            db,
            model,
            cfg,
            http,
            pusher,
            channels: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
        })
    }

    fn sender(&self, session_id: &str) -> broadcast::Sender<Event> {
        lock(&self.channels)
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(1024).0)
            .clone()
    }

    fn emit(&self, session_id: &str, ev: Event) {
        // No receivers is fine: events are also persisted where it matters.
        let _ = self.sender(session_id).send(ev);
    }

    /// Subscribe to a session's events. Also returns the in-progress draft if
    /// a task is running, so a reconnecting client can show partial output.
    pub fn subscribe(&self, session_id: &str) -> (broadcast::Receiver<Event>, Option<Draft>) {
        let rx = self.sender(session_id).subscribe();
        let draft = lock(&self.running)
            .get(session_id)
            .map(|t| lock(&t.draft).clone());
        (rx, draft)
    }

    pub fn is_running(&self, session_id: &str) -> bool {
        lock(&self.running).contains_key(session_id)
    }

    pub fn forget(&self, session_id: &str) {
        lock(&self.channels).remove(session_id);
    }

    /// Stores the prompt and starts a background task. Returns the stored
    /// user message.
    pub async fn start(
        self: &Arc<Self>,
        session_id: &str,
        prompt: String,
    ) -> Result<Message, StartError> {
        if prompt.trim().is_empty() {
            return Err(StartError::Invalid("prompt is empty".into()));
        }
        let sid = session_id.to_string();
        let found = self
            .db
            .call(move |c| {
                let Some(s) = db::get_session(c, &sid)? else {
                    return Ok(None);
                };
                let p = db::get_project(c, &s.project_id)?;
                Ok(p.map(|p| (s, p)))
            })
            .await?;
        let Some((session, project)) = found else {
            return Err(StartError::NotFound);
        };

        let cancel = CancellationToken::new();
        let draft = Arc::new(Mutex::new(Draft::default()));
        {
            let mut running = lock(&self.running);
            if running.contains_key(session_id) {
                return Err(StartError::AlreadyRunning);
            }
            running.insert(
                session_id.to_string(),
                RunningTask {
                    cancel: cancel.clone(),
                    draft: draft.clone(),
                },
            );
        }

        let sid = session_id.to_string();
        let stored = self
            .db
            .call(move |c| {
                let m = db::insert_message(
                    c,
                    &sid,
                    &NewMessage {
                        role: "user".into(),
                        content: prompt,
                        ..Default::default()
                    },
                )?;
                db::set_session_status(c, &sid, "running", None)?;
                Ok(m)
            })
            .await;
        let user_msg = match stored {
            Ok(m) => m,
            Err(e) => {
                lock(&self.running).remove(session_id);
                return Err(StartError::Internal(e));
            }
        };
        self.emit(
            session_id,
            Event::Message {
                message: user_msg.clone(),
            },
        );
        self.emit(
            session_id,
            Event::Status {
                status: "running".into(),
                error: None,
            },
        );

        let this = self.clone();
        tokio::spawn(async move { this.run_task(session, project, cancel, draft).await });
        Ok(user_msg)
    }

    /// Requests cancellation. Returns false if nothing was running.
    pub fn stop(&self, session_id: &str) -> bool {
        match lock(&self.running).get(session_id) {
            Some(t) => {
                t.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Cancels every running task and waits (bounded) for them to finish.
    pub async fn stop_all(&self) {
        let ids: Vec<String> = lock(&self.running).keys().cloned().collect();
        for id in &ids {
            self.stop(id);
        }
        for _ in 0..100 {
            if lock(&self.running).is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn run_task(
        self: Arc<Self>,
        session: Session,
        project: Project,
        cancel: CancellationToken,
        draft: Arc<Mutex<Draft>>,
    ) {
        tracing::info!(session = %session.id, project = %project.name, "task started");
        let result = self.agent_loop(&session, &project, &cancel, &draft).await;
        let (status, error) = match result {
            Ok(LoopEnd::Done) => ("done", None),
            Ok(LoopEnd::Stopped) => ("stopped", None),
            Err(_) if cancel.is_cancelled() => ("stopped", None),
            Err(e) => ("error", Some(format!("{e:#}"))),
        };
        tracing::info!(session = %session.id, status, error = error.as_deref().unwrap_or(""), "task finished");

        let sid = session.id.clone();
        let err = error.clone();
        let title = self
            .db
            .call(move |c| {
                db::set_session_status(c, &sid, status, err.as_deref())?;
                Ok(db::get_session(c, &sid)?.map(|s| s.title))
            })
            .await;
        if let Err(e) = &title {
            tracing::error!("failed to store final status: {e:#}");
        }
        lock(&self.running).remove(&session.id);
        if let Some(e) = &error {
            self.emit(&session.id, Event::Error { message: e.clone() });
        }
        self.emit(
            &session.id,
            Event::Status {
                status: status.into(),
                error,
            },
        );

        // Only names and status leave the machine.
        let title = title.ok().flatten().unwrap_or(session.title);
        self.pusher.notify(&project, &session.id, &title, status);
    }

    async fn agent_loop(
        &self,
        session: &Session,
        project: &Project,
        cancel: &CancellationToken,
        draft: &Arc<Mutex<Draft>>,
    ) -> Result<LoopEnd> {
        let found_skills = skills::discover(&self.cfg.skill_roots());
        let skill_dirs: Vec<PathBuf> = found_skills.iter().map(|s| s.dir.clone()).collect();
        let sandbox = Sandbox::new(Path::new(&project.path), &skill_dirs)?;
        let ctx = ToolContext {
            sandbox,
            shell: self.cfg.config.shell.clone(),
        };
        let system = build_system_prompt(
            &self.cfg.system_prompt_path(),
            project,
            &ctx.sandbox.root,
            &found_skills,
        )
        .await;
        let url = format!("{}/v1/chat/completions", self.model.base_url());
        let tool_defs = tools::definitions();

        for _step in 0..self.cfg.config.max_steps {
            if cancel.is_cancelled() {
                return Ok(LoopEnd::Stopped);
            }
            if !self.wait_for_model(cancel).await? {
                return Ok(LoopEnd::Stopped);
            }

            let sid = session.id.clone();
            let history = self.db.call(move |c| db::all_messages(c, &sid)).await?;
            let body = json!({
                "model": self.cfg.config.llama.model_name,
                "messages": to_openai_messages(&system, &history),
                "tools": tool_defs,
                "stream": true,
            });

            *lock(draft) = Draft::default();
            let sid = session.id.clone();
            let result = llm::stream_chat(&self.http, &url, &body, cancel, |d| match d {
                llm::Delta::Content(t) => {
                    lock(draft).content.push_str(&t);
                    self.emit(&sid, Event::TextDelta { text: t });
                }
                llm::Delta::Reasoning(t) => {
                    lock(draft).reasoning.push_str(&t);
                    self.emit(&sid, Event::ReasoningDelta { text: t });
                }
            })
            .await?;

            let (turn, stopped) = match result {
                llm::StreamResult::Complete(t) => (t, false),
                llm::StreamResult::Cancelled(t) => (t, true),
            };
            if stopped && turn.content.is_empty() && turn.reasoning.is_empty() {
                return Ok(LoopEnd::Stopped);
            }

            let assistant = NewMessage {
                role: "assistant".into(),
                content: turn.content.clone(),
                reasoning: (!turn.reasoning.is_empty()).then(|| turn.reasoning.clone()),
                tool_calls: if stopped {
                    Vec::new()
                } else {
                    turn.tool_calls.clone()
                },
                ..Default::default()
            };
            let saved = self.save(&session.id, assistant).await?;
            *lock(draft) = Draft::default();
            self.emit(&session.id, Event::Message { message: saved });

            if stopped {
                return Ok(LoopEnd::Stopped);
            }
            if turn.tool_calls.is_empty() {
                self.emit(
                    &session.id,
                    Event::Final {
                        content: turn.content,
                    },
                );
                return Ok(LoopEnd::Done);
            }

            for call in &turn.tool_calls {
                let output = if cancel.is_cancelled() {
                    tools::ToolOutput {
                        content: "cancelled by user before running".into(),
                        is_error: true,
                    }
                } else {
                    self.emit(
                        &session.id,
                        Event::ToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        },
                    );
                    tools::execute(&ctx, &call.name, &call.arguments, cancel).await
                };
                let truncated = output.content.chars().count() > EVENT_RESULT_CHARS;
                self.emit(
                    &session.id,
                    Event::ToolResult {
                        tool_call_id: call.id.clone(),
                        name: call.name.clone(),
                        content: tools::truncate_head(
                            &output.content,
                            EVENT_RESULT_CHARS,
                            "full result in message history",
                        ),
                        is_error: output.is_error,
                        truncated,
                    },
                );
                let saved = self
                    .save(
                        &session.id,
                        NewMessage {
                            role: "tool".into(),
                            content: output.content,
                            tool_call_id: Some(call.id.clone()),
                            tool_name: Some(call.name.clone()),
                            is_error: output.is_error,
                            ..Default::default()
                        },
                    )
                    .await?;
                self.emit(&session.id, Event::Message { message: saved });
            }
        }
        bail!(
            "stopped after reaching the limit of {} steps",
            self.cfg.config.max_steps
        )
    }

    async fn save(&self, session_id: &str, m: NewMessage) -> Result<Message> {
        let sid = session_id.to_string();
        self.db.call(move |c| db::insert_message(c, &sid, &m)).await
    }

    /// Waits while the model is starting. Ok(false) if cancelled.
    async fn wait_for_model(&self, cancel: &CancellationToken) -> Result<bool> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30 * 60);
        loop {
            let st = self.model.status();
            match st.state {
                ModelState::Ready => return Ok(true),
                ModelState::Error => {
                    return Err(anyhow!(
                        "model is not available: {}",
                        st.message.unwrap_or_else(|| "error".into())
                    ));
                }
                ModelState::Starting => {}
            }
            if tokio::time::Instant::now() > deadline {
                bail!("timed out waiting for the model to finish loading");
            }
            tokio::select! {
                _ = cancel.cancelled() => return Ok(false),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }
}

pub async fn build_system_prompt(
    prompt_path: &Path,
    project: &Project,
    root: &Path,
    found: &[skills::Skill],
) -> String {
    let base = tokio::fs::read_to_string(prompt_path)
        .await
        .unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.to_string());
    let mut s = base.trim_end().to_string();
    let os = if cfg!(windows) {
        "Windows"
    } else {
        std::env::consts::OS
    };
    s.push_str(&format!(
        "\n\n# Environment\n- Project: {}\n- Project folder: {}\n- OS: {os}; run_command uses PowerShell\n- Today: {}\n",
        project.name,
        root.display(),
        chrono::Local::now().format("%Y-%m-%d"),
    ));
    for name in ["AGENTS.md", "agents.md"] {
        if let Ok(text) = tokio::fs::read_to_string(root.join(name)).await {
            s.push_str("\n# Project instructions (AGENTS.md)\n");
            s.push_str(text.trim());
            s.push('\n');
            break;
        }
    }
    if let Some(section) = skills::prompt_section(found) {
        s.push('\n');
        s.push_str(&section);
    }
    s
}

/// Converts stored history to OpenAI chat messages. Tool calls without a
/// stored result (e.g. after a crash) get a synthetic result so the request
/// stays well-formed.
pub fn to_openai_messages(system: &str, history: &[Message]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];
    let mut pending: Vec<String> = Vec::new();
    let flush = |pending: &mut Vec<String>, out: &mut Vec<Value>| {
        for id in pending.drain(..) {
            out.push(json!({"role": "tool", "tool_call_id": id, "content": "(no result: the task was interrupted)"}));
        }
    };
    let mut answered: HashSet<String> = HashSet::new();
    for m in history {
        match m.role.as_str() {
            "tool" => {
                let id = m.tool_call_id.clone().unwrap_or_default();
                if !pending.contains(&id) || answered.contains(&id) {
                    continue; // orphaned result
                }
                pending.retain(|p| p != &id);
                answered.insert(id.clone());
                out.push(json!({"role": "tool", "tool_call_id": id, "content": m.content}));
            }
            "assistant" => {
                flush(&mut pending, &mut out);
                if m.content.is_empty() && m.tool_calls.is_empty() {
                    continue;
                }
                let mut v = json!({"role": "assistant", "content": m.content});
                if !m.tool_calls.is_empty() {
                    v["tool_calls"] = Value::Array(
                        m.tool_calls
                            .iter()
                            .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}}))
                            .collect(),
                    );
                    pending.extend(m.tool_calls.iter().map(|c| c.id.clone()));
                }
                out.push(v);
            }
            _ => {
                flush(&mut pending, &mut out);
                out.push(json!({"role": "user", "content": m.content}));
            }
        }
    }
    flush(&mut pending, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ToolCall;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            id: 0,
            session_id: "s".into(),
            role: role.into(),
            content: content.into(),
            reasoning: None,
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            is_error: false,
            created_at: String::new(),
        }
    }

    #[test]
    fn patches_missing_tool_results() {
        let mut a = msg("assistant", "");
        a.tool_calls = vec![
            ToolCall {
                id: "1".into(),
                name: "list_dir".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "2".into(),
                name: "list_dir".into(),
                arguments: "{}".into(),
            },
        ];
        let mut t = msg("tool", "ok");
        t.tool_call_id = Some("1".into());
        let h = vec![msg("user", "hi"), a, t, msg("user", "again")];
        let out = to_openai_messages("sys", &h);
        let roles: Vec<&str> = out
            .iter()
            .map(|m| m["role"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            roles,
            ["system", "user", "assistant", "tool", "tool", "user"]
        );
        assert_eq!(out[4]["tool_call_id"], "2");
    }
}
