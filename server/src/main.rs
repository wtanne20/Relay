use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use relay::config::LoadedConfig;
use relay::migrate::{self, MigrateArgs};

#[derive(Parser)]
#[command(
    name = "relay",
    version,
    about = "Personal coding-agent server backed by llama.cpp"
)]
struct Cli {
    /// Path to config.toml. Its folder holds relay.db, logs/, skills/ and system_prompt.md.
    /// Default: ./config.toml if present, else %LOCALAPPDATA%/Relay/config.toml.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (default).
    Serve,
    /// Show the QR code for connecting the phone app.
    Pair,
    /// Import projects, skills, system prompt and model from LM Studio Bionic.
    Migrate(MigrateArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(relay::config::default_config_path);
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "relay=info,tower_http=warn".into()),
                )
                .init();
            let cfg = LoadedConfig::load(&config_path)?;
            let result = relay::serve(cfg).await;
            if let Err(e) = &result {
                // Started from a shortcut, the window would close before the
                // error could be read.
                eprintln!("\n  Relay could not start: {e:#}\n");
                if cfg!(windows) {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
            result
        }
        Command::Pair => {
            let cfg = LoadedConfig::load(&config_path)?;
            relay::pair(cfg).await
        }
        Command::Migrate(args) => {
            let path = config_path;
            tokio::task::spawn_blocking(move || migrate::run(&path, args)).await?
        }
    }
}
