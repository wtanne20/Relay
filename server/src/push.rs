//! Expo push notifications. Payloads carry only the project name, session
//! title and status (plus ids for navigation), never code, file contents or
//! prompt text.

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
        self.send_note(project, session_id, session_title, status, None);
    }

    /// A tool call waiting for approval. The notification gets Allow and
    /// Deny buttons; it carries the call's id (to answer it), not the call.
    pub fn notify_approval(&self, project: &Project, session_id: &str, session_title: &str, tool_call_id: &str) {
        self.send_note(project, session_id, session_title, "approval", Some(tool_call_id.to_string()));
    }

    fn send_note(
        &self,
        project: &Project,
        session_id: &str,
        session_title: &str,
        status: &str,
        tool_call_id: Option<String>,
    ) {
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

impl Notification {
    pub fn to_expo(&self, token: &str) -> Value {
        let label = match self.status.as_str() {
            "done" => "finished",
            "error" => "failed",
            "stopped" => "was stopped",
            "approval" => "needs your approval",
            other => other,
        };
        let mut v = json!({
            "to": token,
            "title": self.project_name,
            "body": format!("{} {}", self.session_title, label),
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
        assert_eq!(v["title"], "Relay");
        assert_eq!(v["body"], "Fix build finished");
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
        assert_eq!(v["body"], "Fix build needs your approval");
        assert_eq!(v["categoryId"], "approval");
        assert_eq!(v["data"]["tool_call_id"], "call_1");
    }
}
