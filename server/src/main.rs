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
    /// Register this device (host) with a Relay brain so it can be paired.
    PairDevice {
        /// The brain's address, e.g. http://my-pc:8765.
        #[arg(long)]
        brain: String,
        /// The brain's token (from its config.toml).
        #[arg(long)]
        token: String,
        /// This device's name (shown in the brain's device list).
        #[arg(long, default_value = "this-device")]
        name: String,
        /// This device's host address (what the brain will connect to).
        #[arg(long, default_value = "127.0.0.1:8766")]
        host: String,
        /// This device's host token (from host.toml).
        #[arg(long)]
        host_token: Option<String>,
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
        Command::PairDevice {
            brain,
            token,
            name,
            host,
            host_token,
        } => {
            // Register this device with the brain's /devices endpoint.
            let brain = brain.trim_end_matches('/');
            let host_token = match host_token {
                Some(t) if !t.trim().is_empty() => t,
                _ => {
                    let state_path =
                        config_path.parent().unwrap_or(Path::new(".")).join("host.toml");
                    if let Ok(s) = std::fs::read_to_string(&state_path)
                        && let Ok(v) = toml::from_str::<std::collections::HashMap<String, String>>(&s)
                        && let Some(t) = v.get("token")
                        && !t.trim().is_empty()
                    {
                        t.clone()
                    } else {
                        anyhow::bail!(
                            "no host token. Start the host first (`relay host`), \
                             or pass --host-token."
                        )
                    }
                }
            };
            let body = serde_json::json!({
                "name": name,
                "url": host,
                "token": host_token,
            });
            let client = reqwest::Client::new();
            let resp = client
                .post(format!("{brain}/devices"))
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("could not reach the brain at {brain}: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                println!("Paired with the brain. This device is now in its device list.");
                println!("  Name:  {name}");
                println!("  Host:  {host}");
                println!("  Brain: {brain}");
                Ok(())
            } else {
                anyhow::bail!("brain returned {status}: {text}")
            }
        }
    }
}
