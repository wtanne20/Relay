//! Relay Client — a small desktop app that connects this machine to a Relay
//! brain. You enter the brain's address + token once; it then starts this
//! machine's host (exposing its files + shell), pairs it with the brain, and
//! you can open the UI. The brain does all the model processing; this
//! machine is the "hands". No terminal needed.

use std::path::PathBuf;
use std::process::Stdio;

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Where the client keeps its state (config, host token, logs).
fn state_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".relay-client")
}

/// Find the relay binary. It's bundled with the client (as a resource) so the
/// .deb is self-contained; fall back to common install locations / PATH.
fn find_relay(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    // Bundled with the app (Tauri resource dir).
    if let Ok(rd) = app.path().resource_dir() {
        candidates.push(rd.join("relay"));
        candidates.push(rd.join("relay/relay"));
    }
    candidates.extend([
        PathBuf::from("/usr/local/bin/relay"),
        PathBuf::from("/usr/bin/relay"),
        state_dir().join("relay"),
    ]);
    for c in candidates {
        if c.is_file() {
            return Ok(c);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let c = PathBuf::from(dir).join("relay");
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    Err("could not find the relay binary (it should be bundled with the Relay Client app)".into())
}

/// Normalize a brain address: add http:// and a default port if missing.
fn normalize_brain(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let with_scheme = if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("http://{s}")
    };
    let rest = with_scheme.split_once("//").map(|(_, r)| r).unwrap_or(&with_scheme);
    let host = rest.split('/').next().unwrap_or(rest);
    if host.contains(':') {
        with_scheme
    } else {
        format!("{with_scheme}:8765")
    }
}

#[derive(Clone, Serialize)]
struct ConnectResult {
    ok: bool,
    message: String,
    brain_url: String,
}

/// Test the brain, start this machine's host, pair it, and save the creds.
#[tauri::command]
fn relay_connect(app: AppHandle, brain: String, token: String) -> Result<ConnectResult, String> {
    let brain_url = normalize_brain(&brain);
    let token = token.trim().to_string();
    if token.is_empty() {
        return Ok(ConnectResult { ok: false, message: "Enter the brain's token.".into(), brain_url });
    }
    let relay = find_relay(&app)?;
    let dir = state_dir();
    let _ = std::fs::create_dir_all(&dir);

    // 1. Test the brain (reachable with this token?).
    let client = reqwest::blocking::Client::new();
    let info = client
        .get(format!("{brain_url}/info"))
        .header("Authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .map_err(|e| format!("Could not reach the brain at {brain_url}: {e}"))?;
    if !info.status().is_success() {
        return Ok(ConnectResult {
            ok: false,
            message: "The brain rejected the token. Check it and try again.".into(),
            brain_url,
        });
    }

    // 2. Start this machine's host (exposes its files + shell), detached.
    let host_port = 8766u16;
    let config = dir.join("config.toml");
    let name = "relay-client";
    let root = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));

    let already = client
        .get(format!("http://127.0.0.1:{host_port}/info"))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    if !already {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("host.log"))
            .map_err(|e| format!("could not open host log: {e}"))?;
        let _ = std::process::Command::new(&relay)
            .arg("host")
            .args(["--bind", &format!("0.0.0.0:{host_port}")])
            .args(["--root", &root.to_string_lossy()])
            .args(["--name", name])
            .args(["--config", &config.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|e| format!("could not start the host: {e}"))?;
        let mut up = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if client
                .get(format!("http://127.0.0.1:{host_port}/info"))
                .timeout(std::time::Duration::from_secs(1))
                .send()
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                up = true;
                break;
            }
        }
        if !up {
            return Ok(ConnectResult {
                ok: false,
                message: "The host started but isn't responding. Check ~/.relay-client/host.log.".into(),
                brain_url,
            });
        }
    }

    // 3. Read the host's token (saved next to the config).
    let host_token = std::fs::read_to_string(dir.join("host.toml"))
        .ok()
        .and_then(|s| {
            let v: std::collections::HashMap<String, String> = toml::from_str(&s).ok()?;
            v.get("token").cloned().filter(|t| !t.trim().is_empty())
        })
        .ok_or("could not read the host token (~/.relay-client/host.toml)")?;

    // 4. Pair this host with the brain (records it in the brain's device list).
    let host_addr = lan_ip().unwrap_or_else(|| "127.0.0.1".into());
    let host_url = format!("{host_addr}:{host_port}");
    let pair = std::process::Command::new(&relay)
        .arg("pair-device")
        .args(["--brain", &brain_url])
        .args(["--token", &token])
        .args(["--name", name])
        .args(["--host", &host_url])
        .args(["--host-token", &host_token])
        .output()
        .map_err(|e| format!("could not run pairing: {e}"))?;
    if !pair.status.success() {
        let msg = String::from_utf8_lossy(&pair.stderr).to_string();
        return Ok(ConnectResult { ok: false, message: format!("Pairing failed: {msg}"), brain_url });
    }

    // 5. Save the creds so the app remembers the brain.
    let cfg = format!(
        "# Relay Client config\nbrain_url = \"{brain_url}\"\nbrain_token = \"{token}\"\nhost_addr = \"{host_addr}\"\nhost_port = {host_port}\nhost_name = \"{name}\"\n"
    );
    std::fs::write(&config, cfg).map_err(|e| e.to_string())?;

    Ok(ConnectResult {
        ok: true,
        message: "Connected. This machine is now in the brain's device list.".into(),
        brain_url,
    })
}

/// Best guess at this machine's LAN IPv4 (so the brain can reach it).
fn lan_ip() -> Option<String> {
    let out = std::process::Command::new("hostname").arg("-I").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    for tok in s.split_whitespace() {
        if tok.chars().all(|c| c.is_ascii_digit() || c == '.') && tok.contains('.') {
            return Some(tok.to_string());
        }
    }
    None
}

/// The saved brain URL (so the UI can show the current connection).
#[tauri::command]
fn relay_saved() -> Result<Option<String>, String> {
    let path = state_dir().join("config.toml");
    let s = std::fs::read_to_string(path).ok();
    Ok(s.and_then(|s| {
        let v: std::collections::HashMap<String, String> = toml::from_str(&s).ok()?;
        v.get("brain_url").cloned()
    }))
}

/// Open the brain's UI in the default browser.
#[tauri::command]
fn relay_open_ui() -> Result<String, String> {
    let path = state_dir().join("config.toml");
    let url = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| {
            let v: std::collections::HashMap<String, String> = toml::from_str(&s).ok()?;
            v.get("brain_url").cloned()
        })
        .ok_or("not connected yet")?;
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("could not open a browser: {e}"))?;
    Ok(url)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![relay_connect, relay_saved, relay_open_ui])
        .run(tauri::generate_context!())
        .expect("error while running the Relay Client");
}
