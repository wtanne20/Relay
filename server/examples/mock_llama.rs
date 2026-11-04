//! Scripted stand-in for llama-server, for trying Relay without a GPU:
//!
//!   cargo run --example mock_llama -- --port 8080
//!
//! Accepts (and ignores) llama-server's other arguments, so it can also be set
//! as `llama_server_path` in config.toml. Extra flags for tests:
//!   --load-ms N        report 503 on /health for N ms (simulated model load)
//!   --exit-after-ms N  exit with code 1 after N ms (simulated crash)

#[path = "../tests/common/mock_llama.rs"]
mod mock_llama;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let port: u16 = get("--port").and_then(|p| p.parse().ok()).unwrap_or(8080);
    let load_ms: u64 = get("--load-ms").and_then(|p| p.parse().ok()).unwrap_or(0);
    if let Some(ms) = get("--exit-after-ms").and_then(|p| p.parse::<u64>().ok()) {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            eprintln!("mock llama-server: simulated crash");
            std::process::exit(1);
        });
    }
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mock llama-server: cannot bind 127.0.0.1:{port}: {e}");
            std::process::exit(2);
        }
    };
    println!("mock llama-server listening on 127.0.0.1:{port} (load {load_ms} ms)");
    mock_llama::serve(
        listener,
        mock_llama::MockOptions {
            load_ms,
            ..Default::default()
        },
    )
    .await;
}
