//! Serves the Relay web UI (the same bundle the desktop app uses) so any
//! browser — including one on a Proot Linux box — can connect to the brain
//! without a native app. It's served from the same origin as the API (no
//! CORS), using a middleware that intercepts browser navigations and static
//! asset requests and serves them from the bundle, while letting API calls
//! pass through untouched. The page itself needs no token; the app still
//! asks for the brain's token to use the API.
//!
//! How it tells the two apart:
//! - A browser navigation sends `Accept: text/html` — we serve the UI.
//! - The Relay API client (and any non-browser) doesn't — we pass through.
//! - Static assets (`/_expo/...`, `/assets/...`, favicon, manifest) are
//!   served directly by path, since they never collide with API routes.

use std::path::{Path, PathBuf};

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::api::AppState;

/// Where the built web bundle lives. Checked in this order:
/// 1. `RELAY_WEB_ROOT` environment variable
/// 2. `web/` next to the working directory (where the installer puts it)
/// 3. `mobile/dist` (development layout)
pub fn web_root() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = std::env::var("RELAY_WEB_ROOT")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            PathBuf::from("web"),
            PathBuf::from("./web"),
            PathBuf::from("mobile/dist"),
            PathBuf::from("../mobile/dist"),
        ])
        .collect();
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// True for the bundle's static-asset paths (they never collide with the API).
fn is_asset(path: &str) -> bool {
    path.starts_with("/_expo/")
        || path.starts_with("/assets/")
        || path == "/favicon.ico"
        || path == "/metadata.json"
        || path.starts_with("/favicon")
}

/// True when the request is a browser navigation (wants HTML).
fn wants_html(req: &Request) -> bool {
    req.headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/html"))
}

/// Serves the web UI for browser navigations and static assets; passes
/// everything else (the API) through. Applied before the auth middleware.
pub async fn ui_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if state.web_root.is_some() && (wants_html(&req) || is_asset(req.uri().path())) {
        return serve_ui(&state, req.uri().path());
    }
    next.run(req).await
}

/// Resolves a request path to a bundle file, or falls back to index.html
/// (SPA routing). Serves it with the right content type.
fn serve_ui(state: &AppState, path: &str) -> Response {
    let Some(root) = &state.web_root else {
        return (
            StatusCode::NOT_FOUND,
            "the web UI isn't bundled with this Relay build",
        )
            .into_response();
    };

    // Prevent path traversal: drop any ".." and "." components.
    let clean: String = path
        .trim_start_matches('/')
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".." && *c != ".")
        .collect::<Vec<_>>()
        .join("/");
    let file = if clean.is_empty() {
        root.join("index.html")
    } else {
        root.join(&clean)
    };

    // SPA fallback: if the file doesn't exist, serve index.html so the
    // client-side router can handle it (e.g. a refresh on /devices).
    let file = if file.is_file() {
        file
    } else {
        root.join("index.html")
    };

    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
    };

    let is_html = file.extension().is_some_and(|e| e == "html");
    let mut headers = header::HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, mime_for(&file).parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        if is_html {
            "no-cache".parse().unwrap()
        } else {
            "public, max-age=31536000, immutable".parse().unwrap()
        },
    );
    (StatusCode::OK, headers, bytes).into_response()
}

fn mime_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "map" => "application/json",
        "txt" => "text/plain",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
