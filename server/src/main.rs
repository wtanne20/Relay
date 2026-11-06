use std::path::{Path, PathBuf};

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
    /// Run a web search the way the agent does, to check it works.
    Search {
        /// What to search for.
        query: String,
    },
    /// Serve this device's files and shell to a Relay PC (the "hands" role).
    Host {
        /// Bind address.
        #[arg(long, default_value = "0.0.0.0:8766")]
        bind: String,
        /// Bearer token the PC uses. Generated and saved next to --config if omitted.
        #[arg(long)]
        token: Option<String>,
        /// Friendly name for the PC's device list.
        #[arg(long, default_value = "this-device")]
        name: String,
        /// Folder the agent is confined to.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Shell for run_command.
        #[arg(long, default_value = "sh")]
        shell: String,
        /// Default command timeout (seconds).
        #[arg(long, default_value_t = 120)]
        timeout: u64,
    },
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
        Command::Search { query } => {
            let key = LoadedConfig::load(&config_path)
                .map(|c| c.config.agent.search_api_key.clone())
                .unwrap_or_default();
            let hits = relay::web::search(&query, 8, &key).await?;
            println!("{}", relay::web::format_hits(&query, &hits));
            Ok(())
        }
        Command::Migrate(args) => {
            let path = config_path;
            tokio::task::spawn_blocking(move || migrate::run(&path, args)).await?
        }
        Command::Host {
            bind,
            token,
            name,
            root,
            shell,
            timeout,
        } => {
            // The host keeps a tiny state file (token) next to its config so
            // re-starts reuse the same token the PC was paired with.
            let state_path = config_path.parent().unwrap_or(Path::new(".")).join("host.toml");
            let token = match token {
                Some(t) if !t.trim().is_empty() => t,
                _ => {
                    if let Ok(s) = std::fs::read_to_string(&state_path)
                        && let Ok(v) = toml::from_str::<std::collections::HashMap<String, String>>(&s)
                        && let Some(t) = v.get("token")
                        && !t.trim().is_empty()
                    {
                        t.clone()
                    } else {
                        let t = relay::config::generate_token();
                        let _ = std::fs::write(
                            &state_path,
                            format!("# Relay host state\n\ntoken = \"{}\"\n", t),
                        );
                        t
                    }
                }
            };
            let root = std::path::absolute(&root).unwrap_or(root);
            if !root.is_dir() {
                anyhow::bail!("host root {} is not a folder", root.display());
            }
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "relay=info,tower_http=warn".into()),
                )
                .init();
            let result = relay::run_host(relay::HostOptions {
                bind,
                token,
                name,
                root,
                shell,
                timeout,
            })
            .await;
            if let Err(e) = &result {
                eprintln!("\n  Relay host could not start: {e:#}\n");
            }
            result
        }
    }
}
