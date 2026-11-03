//! config.toml handling. The config file lives in the "data directory"; the
//! database, logs, system prompt and default skills folder are resolved
//! relative to that directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_LLAMA_ARGS: &[&str] = &[
    "--jinja",
    "-ngl",
    "99",
    "-c",
    "65536",
    "--flash-attn",
    "on",
    "--cache-type-k",
    "q8_0",
    "--cache-type-v",
    "q8_0",
    "--temp",
    "0.6",
    "--top-k",
    "20",
    "--top-p",
    "0.95",
    "--host",
    "127.0.0.1",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Bearer token required by every API request. Generated on first run.
    pub token: String,
    /// Address Relay listens on. Reach it only over Tailscale.
    pub bind: String,
    /// Global system prompt (relative paths are resolved against the data dir).
    pub system_prompt_path: String,
    /// Extra skill locations. Each entry may be a skill folder (containing
    /// SKILL.md) or a folder whose subfolders are skills. `skills/` next to
    /// config.toml is always included.
    pub skills_paths: Vec<String>,
    /// Maximum model round-trips per prompt.
    pub max_steps: usize,
    /// Shell used by run_command ("powershell.exe" or "pwsh" on Windows).
    pub shell: String,
    pub llama: LlamaConfig,
    pub push: PushConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlamaConfig {
    /// Path to llama-server.exe (or the llama.cpp folder that contains it).
    pub llama_server_path: String,
    /// Path to the GGUF model file.
    pub model_path: String,
    /// Port llama-server listens on (always bound to 127.0.0.1).
    pub port: u16,
    /// Extra arguments passed to llama-server. Relay adds -m and --port itself.
    pub args: Vec<String>,
    /// How long to wait for /health to report ready after launch.
    pub health_timeout_seconds: u64,
    /// Model name sent in chat completion requests (llama-server ignores it).
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PushConfig {
    /// Send Expo push notifications when a task finishes, errors or stops.
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: String::new(),
            bind: "0.0.0.0:8765".into(),
            system_prompt_path: "system_prompt.md".into(),
            skills_paths: Vec::new(),
            max_steps: 50,
            shell: if cfg!(windows) {
                "powershell.exe".into()
            } else {
                "sh".into()
            },
            llama: LlamaConfig::default(),
            push: PushConfig::default(),
        }
    }
}

impl Default for LlamaConfig {
    fn default() -> Self {
        Self {
            llama_server_path: String::new(),
            model_path: String::new(),
            port: 8080,
            args: DEFAULT_LLAMA_ARGS.iter().map(|s| s.to_string()).collect(),
            health_timeout_seconds: 600,
            model_name: "local".into(),
        }
    }
}

impl Default for PushConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl LlamaConfig {
    /// Resolves the llama-server executable. Accepts either the exe itself or
    /// the folder that contains it.
    pub fn executable(&self) -> PathBuf {
        let p = PathBuf::from(&self.llama_server_path);
        if p.is_dir() {
            let exe = if cfg!(windows) {
                "llama-server.exe"
            } else {
                "llama-server"
            };
            p.join(exe)
        } else {
            p
        }
    }
}

pub struct LoadedConfig {
    pub config: Config,
    pub path: PathBuf,
    pub data_dir: PathBuf,
    /// True if a token was generated during this load.
    pub token_generated: bool,
}

impl LoadedConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            toml::from_str::<Config>(&text)
                .with_context(|| format!("parsing {}", path.display()))?
        } else {
            Config::default()
        };

        let mut token_generated = false;
        if config.token.trim().is_empty() {
            config.token = generate_token();
            token_generated = true;
        }

        let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let data_dir = abs
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let loaded = Self {
            config,
            path: abs,
            data_dir,
            token_generated,
        };
        if token_generated || !loaded.path.exists() {
            loaded.save()?;
        }
        Ok(loaded)
    }

    pub fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(&self.config)?;
        let header = "# Relay configuration. Edit while Relay is stopped.\n\n";
        std::fs::write(&self.path, format!("{header}{text}"))
            .with_context(|| format!("writing {}", self.path.display()))
    }

    pub fn resolve(&self, p: &str) -> PathBuf {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            p
        } else {
            self.data_dir.join(p)
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("relay.db")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn system_prompt_path(&self) -> PathBuf {
        self.resolve(&self.config.system_prompt_path)
    }

    /// All configured skill roots, the default `skills/` folder first.
    pub fn skill_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.data_dir.join("skills")];
        roots.extend(self.config.skills_paths.iter().map(|p| self.resolve(p)));
        roots
    }
}

/// 32 random bytes, hex encoded.
pub fn generate_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time comparison (time depends only on the lengths).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"You are Relay, a careful and capable coding agent working inside a single project folder on the user's Windows PC.

Guidelines:
- Explore before you change things: use list_dir and read_file to understand the code.
- Make focused, minimal edits with edit_file. Use write_file for new files or full rewrites.
- Use run_command (PowerShell) to build, test and inspect. Prefer non-interactive commands.
- Paths are relative to the project folder. You cannot access anything outside it.
- When you are done, reply with a concise summary of what you changed and anything the user should check.
- If a request is ambiguous, make a reasonable choice and say what you assumed.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn token_is_hex_64() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn defaults_roundtrip() {
        let c = Config::default();
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.llama.port, 8080);
        assert_eq!(back.bind, "0.0.0.0:8765");
        assert!(back.push.enabled);
    }
}
