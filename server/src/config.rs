//! config.toml handling. The config file lives in the "data directory"; the
//! database, logs, system prompt and default skills folder are resolved
//! relative to that directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    /// config.toml and LM Studio's skills folder are always included.
    pub skills_paths: Vec<String>,
    /// Skill names turned off in the app.
    pub disabled_skills: Vec<String>,
    /// Shell used by run_command ("powershell.exe" or "pwsh" on Windows).
    pub shell: String,
    pub agent: AgentConfig,
    pub sampling: SamplingConfig,
    pub llama: LlamaConfig,
    pub push: PushConfig,
}

/// When the agent must ask before using a tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Never ask (fully autonomous).
    #[default]
    Never,
    /// Ask before run_command.
    Commands,
    /// Ask before run_command, write_file and edit_file.
    AllChanges,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToolToggles {
    pub list_dir: bool,
    pub read_file: bool,
    pub write_file: bool,
    pub edit_file: bool,
    pub run_command: bool,
}

impl Default for ToolToggles {
    fn default() -> Self {
        Self { list_dir: true, read_file: true, write_file: true, edit_file: true, run_command: true }
    }
}

impl ToolToggles {
    pub fn enabled(&self, name: &str) -> bool {
        match name {
            "list_dir" => self.list_dir,
            "read_file" => self.read_file,
            "write_file" => self.write_file,
            "edit_file" => self.edit_file,
            "run_command" => self.run_command,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentConfig {
    /// Maximum model round-trips per prompt.
    pub max_steps: usize,
    /// Default run_command timeout when the model doesn't give one.
    pub command_timeout_seconds: u64,
    pub approval: ApprovalMode,
    /// Let reasoning models think before answering (Qwen3 enable_thinking).
    pub thinking: bool,
    pub tools: ToolToggles,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_steps: 50,
            command_timeout_seconds: 120,
            approval: ApprovalMode::Never,
            thinking: true,
            tools: ToolToggles::default(),
        }
    }
}

/// Sent with every completion request, so changes apply immediately.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SamplingConfig {
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub min_p: f64,
    pub repeat_penalty: f64,
    pub presence_penalty: f64,
    /// Maximum tokens per reply; 0 = no limit.
    pub max_tokens: u32,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: 0.6,
            top_p: 0.95,
            top_k: 20,
            min_p: 0.0,
            repeat_penalty: 1.0,
            presence_penalty: 0.0,
            max_tokens: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LlamaConfig {
    /// Path to llama-server.exe (or the llama.cpp folder that contains it).
    /// Empty = find or download it automatically.
    pub llama_server_path: String,
    /// Path to the GGUF model file. Empty = pick one automatically.
    pub model_path: String,
    /// Extra folders to look for GGUF models in (LM Studio's folder and
    /// <data>/models are always searched).
    pub models_dirs: Vec<String>,
    /// Port llama-server listens on (always bound to 127.0.0.1).
    pub port: u16,
    /// Context window in tokens (-c).
    pub context_size: u32,
    /// Layers offloaded to the GPU (-ngl); 99 = all.
    pub gpu_layers: u32,
    /// "on", "off" or "auto" (--flash-attn).
    pub flash_attn: String,
    /// KV cache quantization (--cache-type-k / --cache-type-v).
    pub cache_type_k: String,
    pub cache_type_v: String,
    /// Any other llama-server arguments.
    pub extra_args: Vec<String>,
    /// How long to wait for /health to report ready after launch.
    pub health_timeout_seconds: u64,
    /// Model name sent in chat completion requests (llama-server ignores it).
    pub model_name: String,
    /// Download llama.cpp and pick a model automatically when not configured.
    pub auto_setup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PushConfig {
    /// Send Expo push notifications when a task finishes, errors, stops or
    /// needs approval.
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: String::new(),
            bind: "0.0.0.0:8765".into(),
            system_prompt_path: "system_prompt.md".into(),
            skills_paths: Vec::new(),
            disabled_skills: Vec::new(),
            shell: if cfg!(windows) { "powershell.exe".into() } else { "sh".into() },
            agent: AgentConfig::default(),
            sampling: SamplingConfig::default(),
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
            models_dirs: Vec::new(),
            port: 8080,
            context_size: 65536,
            gpu_layers: 99,
            flash_attn: "on".into(),
            cache_type_k: "q8_0".into(),
            cache_type_v: "q8_0".into(),
            extra_args: Vec::new(),
            health_timeout_seconds: 600,
            model_name: "local".into(),
            auto_setup: true,
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
        if p.is_dir() { p.join(LLAMA_SERVER_EXE) } else { p }
    }

    /// llama-server arguments other than -m and --port (Relay adds those).
    /// Defaults: --jinja -ngl 99 -c 65536 --flash-attn on --cache-type-k q8_0
    /// --cache-type-v q8_0 ... --host 127.0.0.1
    pub fn server_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "--jinja".into(),
            "-ngl".into(),
            self.gpu_layers.to_string(),
            "-c".into(),
            self.context_size.to_string(),
            "--flash-attn".into(),
            self.flash_attn.clone(),
            "--cache-type-k".into(),
            self.cache_type_k.clone(),
            "--cache-type-v".into(),
            self.cache_type_v.clone(),
        ];
        a.extend(self.extra_args.iter().cloned());
        a
    }
}

pub const LLAMA_SERVER_EXE: &str = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };

impl Config {
    /// Checks values coming from the app before they are saved.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let s = &self.sampling;
        let a = &self.agent;
        let l = &self.llama;
        let checks: [(bool, &str); 12] = [
            ((0.0..=2.0).contains(&s.temperature), "temperature must be between 0 and 2"),
            ((0.0..=1.0).contains(&s.top_p), "top_p must be between 0 and 1"),
            (s.top_k <= 1000, "top_k must be at most 1000"),
            ((0.0..=1.0).contains(&s.min_p), "min_p must be between 0 and 1"),
            ((0.5..=2.0).contains(&s.repeat_penalty), "repeat_penalty must be between 0.5 and 2"),
            ((-2.0..=2.0).contains(&s.presence_penalty), "presence_penalty must be between -2 and 2"),
            ((1..=500).contains(&a.max_steps), "max_steps must be between 1 and 500"),
            ((1..=3600).contains(&a.command_timeout_seconds), "command timeout must be between 1 and 3600 seconds"),
            ((512..=1_048_576).contains(&l.context_size), "context size must be between 512 and 1048576"),
            (l.gpu_layers <= 999, "gpu_layers must be at most 999"),
            (["on", "off", "auto"].contains(&l.flash_attn.as_str()), "flash_attn must be on, off or auto"),
            (
                [&l.cache_type_k, &l.cache_type_v].iter().all(|t| CACHE_TYPES.contains(&t.as_str())),
                "cache types must be one of f16, bf16, q8_0, q5_1, q5_0, q4_1, q4_0",
            ),
        ];
        for (ok, msg) in checks {
            if !ok {
                return Err(msg.to_string());
            }
        }
        if l.extra_args.iter().any(|a| matches!(a.as_str(), "--host" | "--port" | "-m" | "--model")) {
            return Err("extra_args may not set --host, --port or the model".into());
        }
        Ok(())
    }
}

pub const CACHE_TYPES: &[&str] = &["f16", "bf16", "q8_0", "q5_1", "q5_0", "q4_1", "q4_0"];

/// Where Relay keeps its data when no --config is given: $RELAY_CONFIG if set,
/// else ./config.toml if it
/// exists (running from a source checkout), otherwise %LOCALAPPDATA%\Relay
/// on Windows or ~/.local/share/relay elsewhere.
pub fn default_config_path() -> PathBuf {
    // RELAY_CONFIG points the server and the desktop app at another config,
    // e.g. the throwaway one the dev scripts use.
    if let Some(p) = std::env::var_os("RELAY_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let local = PathBuf::from("config.toml");
    if local.exists() {
        return local;
    }
    default_data_dir().join("config.toml")
}

pub fn default_data_dir() -> PathBuf {
    if cfg!(windows)
        && let Some(d) = std::env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(d).join("Relay");
    }
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(d).join("relay");
    }
    if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(h).join(".local").join("share").join("relay");
    }
    PathBuf::from(".")
}

/// The user's home folder (USERPROFILE on Windows).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// LM Studio's data folder (%USERPROFILE%\.lmstudio).
pub fn lmstudio_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".lmstudio"))
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
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
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
        let header = "# Relay configuration. Most settings can be changed from the app;\n# edit this file only while Relay is stopped.\n\n";
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

    /// All skill roots: `skills/` in the data folder, LM Studio's skills
    /// folder (if present), then configured extra paths.
    pub fn skill_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.data_dir.join("skills")];
        if let Some(lm) = lmstudio_dir() {
            let d = lm.join("skills");
            if d.is_dir() {
                roots.push(d);
            }
        }
        for p in &self.config.skills_paths {
            let r = self.resolve(p);
            // `relay migrate` may already have listed LM Studio's folder.
            let key = dunce::canonicalize(&r).unwrap_or_else(|_| r.clone());
            if !roots.iter().any(|x| dunce::canonicalize(x).unwrap_or_else(|_| x.clone()) == key) {
                roots.push(r);
            }
        }
        roots
    }

    /// Folders searched for GGUF models.
    pub fn model_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.data_dir.join("models")];
        if let Some(h) = home_dir() {
            dirs.push(h.join(".lmstudio").join("models"));
            dirs.push(h.join(".cache").join("lm-studio").join("models"));
        }
        dirs.extend(self.config.llama.models_dirs.iter().map(|p| self.resolve(p)));
        dirs
    }
}

/// Runtime-editable configuration shared by the API, agent and model
/// manager. Changes are validated and written back to config.toml.
pub struct SharedConfig {
    inner: std::sync::RwLock<LoadedConfig>,
}

impl SharedConfig {
    pub fn new(loaded: LoadedConfig) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { inner: std::sync::RwLock::new(loaded) })
    }

    /// Runs `f` with read access.
    pub fn read<R>(&self, f: impl FnOnce(&LoadedConfig) -> R) -> R {
        f(&self.inner.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// A copy of the current settings.
    pub fn get(&self) -> Config {
        self.read(|l| l.config.clone())
    }

    /// Applies `f`, validates, and saves. Nothing changes if validation or
    /// saving fails.
    pub fn update(&self, f: impl FnOnce(&mut Config)) -> Result<Config, UpdateError> {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let mut next = guard.config.clone();
        f(&mut next);
        next.validate().map_err(UpdateError::Invalid)?;
        let previous = std::mem::replace(&mut guard.config, next);
        if let Err(e) = guard.save() {
            guard.config = previous;
            return Err(UpdateError::Save(e));
        }
        Ok(guard.config.clone())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("{0}")]
    Invalid(String),
    #[error("could not save config.toml: {0:#}")]
    Save(anyhow::Error),
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
        assert!(c.validate().is_ok());
    }

    #[test]
    fn example_config_parses_to_defaults() {
        let text = include_str!("../config.example.toml");
        let c: Config = toml::from_str(text).unwrap();
        let d = Config::default();
        assert_eq!(c.agent, d.agent);
        assert_eq!(c.sampling, d.sampling);
        assert_eq!(c.llama, d.llama);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn default_server_args_match_spec() {
        let args = LlamaConfig::default().server_args().join(" ");
        assert_eq!(args, "--jinja -ngl 99 -c 65536 --flash-attn on --cache-type-k q8_0 --cache-type-v q8_0");
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut c = Config::default();
        c.sampling.temperature = 5.0;
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.llama.extra_args = vec!["--host".into(), "0.0.0.0".into()];
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.llama.cache_type_k = "evil".into();
        assert!(c.validate().is_err());
    }
}
