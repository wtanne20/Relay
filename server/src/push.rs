//! Expo push notifications. Payloads carry only the project name, session
//! title and status (plus ids for navigation), never code, file contents or
//! prompt text.

use std::time::Duration;

use serde_json::{Value, json};

use crate::db::{self, Db, Project};

const EXPO_PUSH_URL: &str = "https://exp.host/--/api/v2/push/send";

#[derive(Clone)]
pub struct Pusher {
    db: Db,
    http: reqwest::Client,
    enabled: bool,
}

impl Pusher {
    pub fn new(db: Db, enabled: bool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { db, http, enabled }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Fire-and-forget notification for a finished task.
    pub fn notify(&self, project: &Project, session_id: &str, session_title: &str, status: &str) {
        if !self.enabled {
            return;
        }
        let this = self.clone();
        let payload = Notification {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            session_id: session_id.to_string(),
            session_title: session_title.to_string(),
            status: status.to_string(),
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
}

impl Notification {
    pub fn to_expo(&self, token: &str) -> Value {
        let label = match self.status.as_str() {
            "done" => "finished",
            "error" => "failed",
            "stopped" => "was stopped",
            other => other,
        };
        json!({
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
        })
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
}
