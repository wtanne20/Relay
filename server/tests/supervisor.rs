//! Phase 2: llama-server supervision, using the mock_llama example binary as
//! a stand-in for llama-server.exe.

use std::path::PathBuf;
use std::time::Duration;

use relay::config::{Config, LlamaConfig, LoadedConfig, SharedConfig};
use relay::model_manager::{ModelManager, ModelState};
use relay::setup::Hardware;

/// Starts a manager whose data folder (config.toml, logs/) is `dir`.
fn start(
    llama: LlamaConfig,
    dir: &std::path::Path,
) -> (ModelManager, std::sync::Arc<SharedConfig>) {
    std::fs::create_dir_all(dir).expect("mkdir");
    let shared = SharedConfig::new(LoadedConfig {
        config: Config {
            llama,
            token: "t".into(),
            ..Default::default()
        },
        path: dir.join("config.toml"),
        data_dir: dir.to_path_buf(),
        token_generated: false,
    });
    (
        ModelManager::start(shared.clone(), Hardware::default()),
        shared,
    )
}

/// Path to the mock_llama example binary, building it if needed.
fn mock_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.join("examples");
    let p = dir.join(format!("mock_llama{}", std::env::consts::EXE_SUFFIX));
    if !p.is_file() {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--example", "mock_llama"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .ok()?;
        assert!(status.success(), "building the mock_llama example failed");
    }
    assert!(
        p.is_file(),
        "mock_llama example not found at {}",
        p.display()
    );
    Some(p)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(18080)
}

fn setup(extra: &[&str]) -> Option<(LlamaConfig, PathBuf)> {
    let exe = mock_exe()?;
    let dir = std::env::temp_dir().join(format!("relay-sup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok()?;
    let model = dir.join("model.gguf");
    std::fs::write(&model, b"GGUF").ok()?;
    let mut cfg = LlamaConfig {
        llama_server_path: exe.to_string_lossy().into_owned(),
        model_path: model.to_string_lossy().into_owned(),
        port: free_port(),
        health_timeout_seconds: 20,
        ..Default::default()
    };
    cfg.extra_args.extend(extra.iter().map(|s| s.to_string()));
    Some((cfg, dir))
}

async fn wait_for(m: &ModelManager, state: ModelState, secs: u64) -> bool {
    for _ in 0..secs * 10 {
        if m.status().state == state {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn health(port: u16) -> bool {
    matches!(reqwest::get(format!("http://127.0.0.1:{port}/health")).await, Ok(r) if r.status().is_success())
}

#[tokio::test]
async fn launches_waits_for_health_logs_and_shuts_down() {
    let Some((cfg, dir)) = setup(&["--load-ms", "1500"]) else {
        return;
    };
    let port = cfg.port;
    let (m, _shared) = start(cfg, &dir);
    assert_eq!(m.status().state, ModelState::Starting);
    assert!(
        wait_for(&m, ModelState::Ready, 20).await,
        "status: {:?}",
        m.status()
    );
    assert!(!m.status().external);
    assert!(health(port).await, "/health should return ok");

    let log = std::fs::read_to_string(dir.join("logs/llama-server.log")).unwrap_or_default();
    assert!(log.contains("mock llama-server listening"), "log: {log}");

    m.shutdown().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!health(port).await, "child should be killed on shutdown");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn gives_up_after_three_restarts_then_restart_recovers() {
    let Some((cfg, dir)) = setup(&["--exit-after-ms", "300"]) else {
        return;
    };
    let (m, _shared) = start(cfg, &dir);
    assert!(
        wait_for(&m, ModelState::Error, 30).await,
        "status: {:?}",
        m.status()
    );
    let st = m.status();
    assert!(
        st.message.as_deref().unwrap_or("").contains("Gave up"),
        "{st:?}"
    );
    assert_eq!(st.recent_restarts, 3);

    // A manual restart starts a fresh attempt.
    m.restart().await;
    assert!(wait_for(&m, ModelState::Starting, 5).await);
    m.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn uses_existing_server_on_port() {
    let Some((mut cfg, dir)) = setup(&[]) else {
        return;
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    cfg.port = listener.local_addr().expect("addr").port();
    cfg.llama_server_path = "does-not-exist.exe".into();
    tokio::spawn(
        axum::serve(
            listener,
            axum::Router::new().route(
                "/health",
                axum::routing::get(|| async { "{\"status\":\"ok\"}" }),
            ),
        )
        .into_future(),
    );
    let (m, _shared) = start(cfg, &dir);
    assert!(
        wait_for(&m, ModelState::Ready, 10).await,
        "{:?}",
        m.status()
    );
    assert!(m.status().external);
    m.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn clear_error_when_not_configured() {
    let dir = std::env::temp_dir().join(format!("relay-sup-{}", uuid::Uuid::new_v4()));
    let cfg = LlamaConfig {
        port: free_port(),
        auto_setup: false,
        ..Default::default()
    };
    let (m, _shared) = start(cfg, &dir);
    assert!(wait_for(&m, ModelState::Error, 10).await);
    assert!(
        m.status()
            .message
            .unwrap_or_default()
            .contains("llama-server is not set up")
    );
    m.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn first_run_picks_a_model_and_saves_it() {
    let Some((mut cfg, dir)) = setup(&[]) else {
        return;
    };
    // No model configured: one is waiting in <data>/models.
    cfg.model_path = String::new();
    let model_dir = dir.join("models").join("pub").join("Some-GGUF");
    std::fs::create_dir_all(&model_dir).expect("mkdir");
    let model = model_dir.join("some-Q4_K_M.gguf");
    std::fs::write(&model, b"GGUF").expect("write");
    let (m, shared) = start(cfg, &dir);
    assert!(
        wait_for(&m, ModelState::Ready, 20).await,
        "{:?}",
        m.status()
    );
    // Compare as paths so separators don't matter on Windows.
    assert_eq!(
        std::path::Path::new(&shared.get().llama.model_path)
            .components()
            .collect::<Vec<_>>(),
        model.components().collect::<Vec<_>>()
    );
    let saved = std::fs::read_to_string(dir.join("config.toml")).expect("config saved");
    assert!(saved.contains("some-Q4_K_M.gguf"));
    m.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}
