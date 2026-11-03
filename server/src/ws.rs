//! Per-session WebSocket: forwards agent events to the client.
//!
//! Protocol: on connect the server sends a `hello` event with the current
//! status and any in-progress draft, then streams events. Clients reconcile
//! history by fetching `GET /sessions/{id}/messages?after=<last id>` and
//! upsert `message` events by id. A `resync` event means events were dropped
//! (slow client) and history should be refetched.

use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, Utf8Bytes, WebSocket};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::api::AppState;
use crate::db;

pub async fn handle(mut socket: WebSocket, state: AppState, session_id: String) {
    // Subscribe before reading state so no event falls in the gap.
    let (mut rx, draft) = state.agent.subscribe(&session_id);
    let sid = session_id.clone();
    let snapshot = state
        .db
        .call(move |c| {
            let s = db::get_session(c, &sid)?;
            let (last, _) = db::page_messages(c, &sid, None, None, 1)?;
            Ok(s.map(|s| (s, last.first().map(|m| m.id))))
        })
        .await;
    let (session, last_id) = match snapshot {
        Ok(Some(v)) => v,
        Ok(None) => {
            let _ = send_json(
                &mut socket,
                &json!({"type": "error", "message": "session not found"}),
            )
            .await;
            return;
        }
        Err(e) => {
            let _ = send_json(
                &mut socket,
                &json!({"type": "error", "message": format!("{e:#}")}),
            )
            .await;
            return;
        }
    };
    let running = state.agent.is_running(&session_id);
    let hello = json!({
        "type": "hello",
        "session_id": session_id,
        "status": if running { "running" } else { session.status.as_str() },
        "error": session.error,
        "running": running,
        "last_message_id": last_id,
        "draft": draft,
    });
    if send_json(&mut socket, &hello).await.is_err() {
        return;
    }

    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let Ok(v) = serde_json::to_value(&ev) else { continue };
                    if send_json(&mut socket, &v).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(session = %session_id, "websocket client lagged by {n} events");
                    if send_json(&mut socket, &json!({"type": "resync", "dropped": n})).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Closed) => {
                    let _ = socket.send(WsMessage::Close(None)).await;
                    break;
                }
            },
            msg = socket.recv() => match msg {
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(WsMessage::Text(t))) if t.as_str() == "ping" => {
                    if socket.send(WsMessage::Text(Utf8Bytes::from_static("pong"))).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if socket.send(WsMessage::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
            _ = state.shutdown.cancelled() => {
                let _ = socket.send(WsMessage::Close(None)).await;
                break;
            }
        }
    }
}

async fn send_json(socket: &mut WebSocket, v: &serde_json::Value) -> Result<(), axum::Error> {
    socket.send(WsMessage::Text(v.to_string().into())).await
}
