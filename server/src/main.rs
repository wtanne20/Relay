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
    /// Link this computer to a Relay brain, so the brain's agent can work
    /// with this computer's files while the brain does the thinking.
    ///
    /// First time: `relay link --brain <address> --code <6 digits>` (the brain
    /// shows the code under Devices). After that, plain `relay link` reconnects.
    /// Without --brain it looks for brains on your tailnet.
    Link {
        /// The brain's address, e.g. 100.101.102.103 or my-desktop (port 8765 is added).
        #[arg(long)]
        brain: Option<String>,
        /// The 6-digit pairing code shown on the brain.
        #[arg(long)]
        code: Option<String>,
        /// This computer's name in the brain's device list (default: its host name).
        #[arg(long)]
        name: Option<String>,
        /// The folder the agent may use on this computer (default: your home folder).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Only pair (save the link), don't stay connected.
        #[arg(long)]
        pair_only: bool,
    },
    /// Forget the brain this computer is linked to.
    Unlink,
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
        Command::Link { brain, code, name, root, pair_only } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "relay=info,tower_http=warn".into()),
                )
                .init();
            let dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let path = relay::link::DeviceConfig::path_in(&dir);
            let cfg = match (brain, code) {
                (Some(brain), Some(code)) => {
                    let cfg = relay::link::pair_and_save(
                        &path,
                        &brain,
                        &code,
                        name.as_deref(),
                        root.as_deref(),
                    )
                    .await?;
                    println!(
                        "\n  Paired with {} ({}).\n  This computer is \"{}\"; the agent may use {}.\n",
                        if cfg.brain_name.is_empty() { "the brain" } else { &cfg.brain_name },
                        cfg.brain_url,
                        cfg.name,
                        cfg.root.display()
                    );
                    cfg
                }
                (None, None) if relay::link::DeviceConfig::load(&path).is_some() => {
                    relay::link::DeviceConfig::load(&path)
                        .ok_or_else(|| anyhow::anyhow!("not linked yet"))?
                }
                (None, Some(_)) => anyhow::bail!("--code needs --brain too"),
                (_, None) => {
                    println!("\n  Looking for Relay brains on your tailnet...");
                    let found = relay::link::discover_brains().await;
                    if found.is_empty() {
                        println!("  None found. Check that Relay is running on your main computer and");
                        println!("  that Tailscale is connected on both, then run:");
                    } else {
                        for b in &found {
                            println!("    {}  {}  (Relay {})", b.name, b.url, b.version);
                        }
                        println!("\n  On the brain, open Devices > Add a computer to get a code, then run:");
                    }
                    println!("    relay link --brain <address> --code <6 digits>\n");
                    return Ok(());
                }
            };
            if pair_only {
                return Ok(());
            }
            println!(
                "  Linking to {} at {} (Ctrl+C to stop)...",
                if cfg.brain_name.is_empty() { "the brain" } else { &cfg.brain_name },
                cfg.brain_url
            );
            let shutdown = tokio_util::sync::CancellationToken::new();
            let stop = shutdown.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                stop.cancel();
            });
            relay::link::run_device_link(cfg, shutdown, |state| match state {
                relay::link::LinkState::Connected => println!("  Connected."),
                relay::link::LinkState::Offline { message } => {
                    println!("  Brain not reachable ({message}); retrying...")
                }
                relay::link::LinkState::Revoked => println!(
                    "  The brain no longer knows this computer. Pair again with a new code."
                ),
                relay::link::LinkState::Connecting => {}
            })
            .await
        }
        Command::Unlink => {
            let dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let path = relay::link::DeviceConfig::path_in(&dir);
            if path.exists() {
                std::fs::remove_file(&path)?;
                println!("Unlinked. Remove this computer from the brain's Devices list too.");
            } else {
                println!("This computer is not linked to a brain.");
            }
            Ok(())
        }
    }
}
