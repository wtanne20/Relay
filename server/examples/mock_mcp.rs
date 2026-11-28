//! A tiny MCP server for trying Relay's MCP support without installing one:
//!
//!   cargo run --example mock_mcp                  (stdio: add it as a program)
//!   cargo run --example mock_mcp -- --http 8790   (Streamable HTTP at /mcp,
//!                                                  header "Authorization: Bearer secret")
//!
//! Tools: echo (read-only), add_note and fail.

#[path = "../tests/common/mock_mcp.rs"]
mod mock_mcp;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.iter().position(|a| a == "--http") {
        Some(i) => {
            let port: u16 = args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(8790);
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .expect("bind");
            eprintln!("mock MCP server on http://127.0.0.1:{port}/mcp");
            let _ = axum::serve(listener, mock_mcp::router()).await;
        }
        None => tokio::task::spawn_blocking(mock_mcp::serve_stdio)
            .await
            .expect("stdio"),
    }
}
