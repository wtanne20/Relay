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
    #[arg(long, global = true, default_value = "config.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (default).
    Serve,
    /// Import projects, skills, system prompt and model from LM Studio Bionic.
    Migrate(MigrateArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "relay=info,tower_http=warn".into()),
                )
                .init();
            let cfg = LoadedConfig::load(&cli.config)?;
            relay::serve(cfg).await
        }
        Command::Migrate(args) => {
            let path = cli.config.clone();
            tokio::task::spawn_blocking(move || migrate::run(&path, args)).await?
        }
    }
}
