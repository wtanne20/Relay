//! Scripted stand-in for stable-diffusion.cpp's sd-server, for trying image
//! chats without a GPU:
//!
//!   cargo run --example mock_sd -- --listen-port 8771
//!
//! Accepts (and ignores) sd-server's other arguments, so it can be set as
//! `sd_server_path` in config.toml.

#[path = "../tests/common/mock_sd.rs"]
mod mock_sd;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args
        .iter()
        .position(|a| a == "--listen-port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(1234);
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mock sd-server: cannot bind 127.0.0.1:{port}: {e}");
            std::process::exit(2);
        }
    };
    println!("mock sd-server listening on 127.0.0.1:{port}");
    mock_sd::serve(
        listener,
        mock_sd::MockSdOptions {
            load_ms: 1500,
            step_ms: 250,
        },
    )
    .await;
}
