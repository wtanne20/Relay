//! Task notifications.
//!
//! Each finished task gets a notice kept on the PC: the task's name, its
//! status and a one- or two-line summary of what happened, sized for a
//! collapsed notification. The Android app checks these over Tailscale and
//! shows them itself, so the summary never leaves your own devices.
//!
//! Expo push (only for builds with an Expo project id) goes through Expo's
//! servers, so its payloads carry only the names and status (plus ids for
//! navigation), never the summary, code, file contents or prompt text.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::config::SharedConfig;
use crate::db::{self, Db, Project};

const EXPO_PUSH_URL: &str = "https://exp.host/--/api/v2/push/send";

#[derive(Clone)]
pub struct Pusher {
    db: Db,
    http: reqwest::Client,
    cfg: Arc<SharedConfig>,
}

impl Pusher {
    pub fn new(db: Db, cfg: Arc<SharedConfig>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { db, http, cfg }
    }

    /// The `[push] enabled` setting (changeable from the app).
    pub fn enabled(&self) -> bool {
        self.cfg.read(|l| l.config.push.enabled)
    }

    /// Fire-and-forget notification for a finished task or a pending
    /// approval.
    pub fn notify(&self, project: &Project, session_id: &str, session_title: &str, status: &str) {
        self.send_note(project, session_id, session_title, status, None, None);
    }

    /// A finished run, with a short summary of what happened for the
    /// notification's body. `name` is the task's name (or the chat's title).
    pub fn notify_finished(
        &self,
        project: &Project,
        session_id: &str,
        name: &str,
        status: &str,
        summary: Option<String>,
    ) {
        self.send_note(project, session_id, name, status, None, summary);
    }

    /// A tool call waiting for approval. The notification gets Allow and
    /// Deny buttons; it carries the call's id (to answer it), not the call.
    pub fn notify_approval(&self, project: &Project, session_id: &str, session_title: &str, tool_call_id: &str) {
        self.send_note(project, session_id, session_title, "approval", Some(tool_call_id.to_string()), None);
    }

    fn send_note(
        &self,
        project: &Project,
        session_id: &str,
        session_title: &str,
        status: &str,
        tool_call_id: Option<String>,
        summary: Option<String>,
    ) {
        // Kept on the PC for phones that check in instead of getting pushes.
        let (db, pid, pname, sid, title, st, sum) = (
            self.db.clone(),
            project.id.clone(),
            project.name.clone(),
            session_id.to_string(),
            session_title.to_string(),
            status.to_string(),
            summary,
        );
        tokio::spawn(async move {
            let saved = db
                .call(move |c| {
                    crate::tasks::add_notice(c, &st, &pid, &pname, &sid, &title, sum.as_deref())
                })
                .await;
            if let Err(e) = saved {
                tracing::warn!("could not save notice: {e:#}");
            }
        });
        if !self.enabled() {
            return;
        }
        let this = self.clone();
        let payload = Notification {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            session_id: session_id.to_string(),
            session_title: session_title.to_string(),
            status: status.to_string(),
            tool_call_id,
        };
        tokio::spawn(async move {
            if let Err(e) = this.send(payload).await {
                tracing::warn!("push notification failed: {e:#}");
            }
        });
    }

    async fn send(&self, n: Notification) -> anyhow::Result<()> {
        let tokens = self.db.call(|c| db::list_push_tokens(c)).await?;
        if tokens.is_empty() {
            return Ok(());
        }
        let messages: Vec<Value> = tokens.iter().map(|t| n.to_expo(t)).collect();
        let resp = self.http.post(EXPO_PUSH_URL).json(&messages).send().await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            anyhow::bail!("Expo push API returned {status}: {body}");
        }
        // Drop tokens Expo reports as no longer registered.
        if let Some(results) = body.get("data").and_then(Value::as_array) {
            for (token, r) in tokens.iter().zip(results) {
                if r.pointer("/details/error").and_then(Value::as_str)
                    == Some("DeviceNotRegistered")
                {
                    let t = token.clone();
                    let _ = self.db.call(move |c| db::remove_push_token(c, &t)).await;
                    tracing::info!("removed unregistered push token");
                }
            }
        }
        Ok(())
    }
}

pub struct Notification {
    pub project_id: String,
    pub project_name: String,
    pub session_id: String,
    pub session_title: String,
    pub status: String,
    /// For an approval: the call to answer from the notification's buttons.
    pub tool_call_id: Option<String>,
}

/// The status word for a notification's title: "Morning summary · Done".
pub fn status_label(status: &str) -> &str {
    match status {
        "done" => "Done",
        "error" => "Failed",
        "stopped" => "Stopped",
        "approval" => "Needs approval",
        other => other,
    }
}

/// A notification's title: the task's name and how it ended.
pub fn notice_title(name: &str, status: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { "A chat" } else { name };
    format!("{name} · {}", status_label(status))
}

/// About two lines of a collapsed notification on a phone.
pub const SUMMARY_CHARS: usize = 110;

/// Turns a reply or an error into one or two plain lines for a
/// notification: markdown and code dropped, whitespace folded, the first
/// sentences that fit in [`SUMMARY_CHARS`], cut at a word with "…".
/// None when nothing readable is left.
pub fn short_summary(text: &str) -> Option<String> {
    let text = text.rsplit("</think>").next().unwrap_or(text);
    let mut lines = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.is_empty() || t.starts_with('|') || t.chars().all(|c| "-=*_#>".contains(c)) {
            continue;
        }
        // Headings, bullets, numbering and quotes.
        let t = t.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
        let t = match t.split_once(". ") {
            Some((n, rest)) if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) => rest,
            _ => t,
        };
        let t: String = t.chars().filter(|c| !matches!(c, '*' | '`' | '_')).collect();
        let t = t.trim().trim_matches(|c: char| matches!(c, '"' | '“' | '”')).trim();
        if !t.is_empty() {
            lines.push(t.to_string());
        }
        if lines.iter().map(|l| l.len()).sum::<usize>() > SUMMARY_CHARS * 2 {
            break;
        }
    }
    let joined = lines
        .iter()
        .map(|l| {
            // A line without its own ending reads as part of a list.
            if l.ends_with(['.', '!', '?', ':', ';']) { l.clone() } else { format!("{l}.") }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let flat = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.trim_end_matches(':').trim();
    if flat.is_empty() {
        return None;
    }
    if flat.chars().count() <= SUMMARY_CHARS {
        return Some(flat.to_string());
    }
    // Whole sentences when they fit, otherwise cut at a word.
    let head: String = flat.chars().take(SUMMARY_CHARS).collect();
    if let Some(end) = head.rfind(['.', '!', '?'])
        && end >= SUMMARY_CHARS / 3
    {
        return Some(head[..=end].to_string());
    }
    let cut: String = flat.chars().take(SUMMARY_CHARS - 1).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > SUMMARY_CHARS / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    Some(format!("{}…", cut.trim_end_matches([',', ';', ':', ' ', '.'])))
}

impl Notification {
    pub fn to_expo(&self, token: &str) -> Value {
        // Names and status only: these pass through Expo's servers.
        let mut v = json!({
            "to": token,
            "title": notice_title(&self.session_title, &self.status),
            "body": self.project_name,
            "sound": "default",
            "priority": "high",
            "channelId": "tasks",
            "data": {
                "project_id": self.project_id,
                "session_id": self.session_id,
                "status": self.status,
            }
        });
        if let Some(id) = &self.tool_call_id {
            // The app registers this category with Allow and Deny buttons.
            v["categoryId"] = json!("approval");
            v["data"]["tool_call_id"] = json!(id);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_has_only_names_and_status() {
        let n = Notification {
            project_id: "p".into(),
            project_name: "Relay".into(),
            session_id: "s".into(),
            session_title: "Fix build".into(),
            status: "done".into(),
            tool_call_id: None,
        };
        let v = n.to_expo("ExponentPushToken[x]");
        assert_eq!(v["title"], "Fix build · Done");
        assert_eq!(v["body"], "Relay");
        let keys: Vec<&String> = v["data"]
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn approval_label() {
        let n = Notification {
            project_id: "p".into(),
            project_name: "Relay".into(),
            session_id: "s".into(),
            session_title: "Fix build".into(),
            status: "approval".into(),
            tool_call_id: Some("call_1".into()),
        };
        let v = n.to_expo("t");
        assert_eq!(v["title"], "Fix build · Needs approval");
        assert_eq!(v["categoryId"], "approval");
        assert_eq!(v["data"]["tool_call_id"], "call_1");
    }

    #[test]
    fn titles() {
        assert_eq!(notice_title("Morning summary", "done"), "Morning summary · Done");
        assert_eq!(notice_title("Backup", "error"), "Backup · Failed");
        assert_eq!(notice_title("  ", "stopped"), "A chat · Stopped");
    }

    #[test]
    fn summaries_fit_a_notification() {
        assert_eq!(short_summary("All 42 tests pass."), Some("All 42 tests pass.".into()));
        assert_eq!(
            short_summary("## Morning summary\n\n- **3** new emails, none urgent\n- Rain after 4 PM"),
            Some("Morning summary. 3 new emails, none urgent. Rain after 4 PM.".into())
        );
        assert_eq!(short_summary("```\nls -la\n```\n"), None);
        assert_eq!(short_summary("\"Backed up 12 files.\""), Some("Backed up 12 files.".into()));
        assert_eq!(short_summary("<think>hmm</think>Done: backed up 12 files."), Some("Done: backed up 12 files.".into()));
        let long = "I checked every folder in the project and found that the build fails because the lockfile is out of date. I updated it and reran the build, which now passes.";
        let s = short_summary(long).expect("summary");
        assert!(s.chars().count() <= SUMMARY_CHARS, "{s}");
        assert!(s.ends_with('…') || s.ends_with('.'), "{s}");
        let words = "word ".repeat(60);
        let s = short_summary(&words).expect("summary");
        assert!(s.chars().count() <= SUMMARY_CHARS && s.ends_with("word…"), "{s}");
    }
}
