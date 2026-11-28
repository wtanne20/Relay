//! MCP (Model Context Protocol) servers: tools other people publish, such as
//! GitHub, databases or Home Assistant, that Relay's agent can use.
//!
//! A server is either a local program Relay starts and talks to over its
//! stdin/stdout ("stdio"), or a web address ("Streamable HTTP"). Relay
//! connects to each enabled server, lists its tools and offers them to the
//! model as `<server>__<tool>`. Every call goes through Relay's approvals
//! like the built-in tools: see [`needs_approval`].

pub mod client;
pub mod hub;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use hub::{McpHub, ServerState, ServerStatus, ToolInfo};

pub const MAX_SERVERS: usize = 50;
pub const MAX_NAME: usize = 32;
/// What the model sees between the server's name and the tool's.
pub const SEPARATOR: &str = "__";
/// Default wait for one tool call.
pub const DEFAULT_TIMEOUT: u64 = 120;
pub const MAX_TIMEOUT: u64 = 3600;
/// Longest tool result passed to the model.
pub const MAX_RESULT_CHARS: usize = 20_000;

/// One MCP server, as saved in config.toml and sent by the app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct McpServerConfig {
    /// Short id, also the prefix of its tools' names, e.g. `github`.
    pub name: String,
    /// A note for the user (the model is told what each tool does by the
    /// server itself).
    pub description: String,
    pub enabled: bool,
    /// stdio: the program to start, e.g. `npx`, `uvx` or a full path.
    pub command: String,
    pub args: Vec<String>,
    /// Extra environment variables for the program (API keys and such).
    pub env: BTreeMap<String, String>,
    /// Folder the program starts in. Empty = Relay's data folder.
    pub cwd: String,
    /// HTTP: the server's address. When set, `command` is ignored.
    pub url: String,
    /// HTTP: extra headers, e.g. `Authorization: Bearer ...`.
    pub headers: BTreeMap<String, String>,
    /// Seconds to wait for one tool call. 0 = [`DEFAULT_TIMEOUT`].
    pub timeout_seconds: u64,
    /// Tools (by the server's own name) the agent may not use.
    pub disabled_tools: Vec<String>,
    /// Run this server's tools without asking, even when "Ask before acting"
    /// is on. Off by default.
    pub trusted: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            enabled: true,
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            timeout_seconds: 0,
            disabled_tools: Vec::new(),
            trusted: false,
        }
    }
}

/// How Relay talks to a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    Http,
}

impl McpServerConfig {
    pub fn transport(&self) -> Transport {
        if self.url.trim().is_empty() {
            Transport::Stdio
        } else {
            Transport::Http
        }
    }

    pub fn timeout(&self) -> std::time::Duration {
        let s = if self.timeout_seconds == 0 {
            DEFAULT_TIMEOUT
        } else {
            self.timeout_seconds
        };
        std::time::Duration::from_secs(s)
    }

    /// Trims every field (what the app sends may have stray spaces).
    pub fn cleaned(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().to_string();
        self.command = self.command.trim().to_string();
        self.cwd = self.cwd.trim().to_string();
        self.url = self.url.trim().to_string();
        self.args.retain(|a| !a.is_empty());
        self.env = std::mem::take(&mut self.env)
            .into_iter()
            .map(|(k, v)| (k.trim().to_string(), v))
            .filter(|(k, _)| !k.is_empty())
            .collect();
        self.headers = std::mem::take(&mut self.headers)
            .into_iter()
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .filter(|(k, _)| !k.is_empty())
            .collect();
        self.disabled_tools.retain(|t| !t.trim().is_empty());
        self
    }

    /// Checks one server on its own (duplicates are checked by
    /// [`validate_all`]).
    pub fn validate(&self) -> Result<(), String> {
        let name = &self.name;
        if !valid_name(name) {
            return Err(format!(
                "server name '{name}' must start with a lowercase letter and use only a-z, 0-9 and single _ (at most {MAX_NAME} characters)"
            ));
        }
        match self.transport() {
            Transport::Stdio => {
                if self.command.is_empty() {
                    return Err(format!(
                        "server '{name}' needs a command to start it (like npx or uvx) or a web address"
                    ));
                }
            }
            Transport::Http => {
                let ok = reqwest::Url::parse(&self.url).is_ok_and(|u| {
                    matches!(u.scheme(), "http" | "https") && u.host_str().is_some()
                });
                if !ok {
                    return Err(format!(
                        "server '{name}': the address must start with http:// or https://"
                    ));
                }
                for k in self.headers.keys() {
                    if reqwest::header::HeaderName::from_bytes(k.as_bytes()).is_err() {
                        return Err(format!("server '{name}': '{k}' isn't a valid header name"));
                    }
                }
            }
        }
        for k in self.env.keys() {
            if k.contains('=') || k.contains('\0') {
                return Err(format!(
                    "server '{name}': '{k}' isn't a valid variable name"
                ));
            }
        }
        if self.timeout_seconds > MAX_TIMEOUT {
            return Err(format!(
                "server '{name}': the timeout must be at most {MAX_TIMEOUT} seconds"
            ));
        }
        if self.description.len() > 2000 {
            return Err(format!("server '{name}': the description is too long"));
        }
        Ok(())
    }

    /// Whether the user turned this one tool off.
    pub fn tool_allowed(&self, tool: &str) -> bool {
        !self.disabled_tools.iter().any(|t| t == tool)
    }
}

/// Lowercase letter first, then a-z, 0-9 and `_`, never two `_` in a row
/// (that's the separator) and not ending in one.
pub fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
        && s.len() <= MAX_NAME
        && !s.contains(SEPARATOR)
        && !s.ends_with('_')
}

/// Checks the whole list: each server, and no two with the same name.
pub fn validate_all(servers: &[McpServerConfig]) -> Result<(), String> {
    if servers.len() > MAX_SERVERS {
        return Err(format!("at most {MAX_SERVERS} MCP servers"));
    }
    for (i, s) in servers.iter().enumerate() {
        s.validate()?;
        if servers[..i].iter().any(|t| t.name == s.name) {
            return Err(format!("there is already an MCP server named '{}'", s.name));
        }
    }
    Ok(())
}

/// The name the model calls a server's tool by. OpenAI-style tool names
/// allow only `a-zA-Z0-9_-` and 64 characters, so anything else becomes `_`
/// and long names are cut (with a short hash so two long names stay apart).
pub fn exposed_name(server: &str, tool: &str) -> String {
    let clean: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let full = format!("{server}{SEPARATOR}{clean}");
    if full.len() <= 64 {
        return full;
    }
    use sha2::Digest;
    let hash = sha2::Sha256::digest(tool.as_bytes());
    let tag: String = hash.iter().take(3).map(|b| format!("{b:02x}")).collect();
    format!("{}_{tag}", &full[..57])
}

/// The server a tool name the model used belongs to, going by its prefix.
pub fn server_of(exposed: &str) -> Option<&str> {
    let (server, rest) = exposed.split_once(SEPARATOR)?;
    (valid_name(server) && !rest.is_empty()).then_some(server)
}

/// Whether a call to an MCP tool is asked about first. MCP tools can do
/// anything their server can, so with "Ask before acting" on (commands or
/// all changes) they're asked about, unless the server says the tool only
/// reads, or the user trusts the server.
pub fn needs_approval(mode: crate::config::ApprovalMode, read_only: bool, trusted: bool) -> bool {
    !matches!(mode, crate::config::ApprovalMode::Never) && !read_only && !trusted
}

/// Servers in the format most MCP READMEs give (Claude Desktop, Cursor,
/// VS Code): `{"mcpServers": {"name": {"command": .., "args": [..],
/// "env": {..}}}}` or `{"servers": ..}`, or just the inner map. Names are
/// made to fit Relay's rules.
pub fn parse_import(v: &Value) -> Result<Vec<McpServerConfig>, String> {
    let map = v
        .get("mcpServers")
        .or_else(|| v.get("servers"))
        .or_else(|| v.get("mcp").and_then(|m| m.get("servers")))
        .unwrap_or(v);
    let Some(map) = map.as_object() else {
        return Err("expected an object with \"mcpServers\"".into());
    };
    if map.is_empty() {
        return Err("no servers found".into());
    }
    let str_map = |x: Option<&Value>| -> BTreeMap<String, String> {
        x.and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        let v = v
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string());
                        (k.clone(), v)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for (key, s) in map {
        if !s.is_object() {
            return Err(format!("'{key}' isn't a server"));
        }
        let get = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut url = get("url");
        if url.is_empty() {
            url = get("serverUrl");
        }
        let server = McpServerConfig {
            name: import_name(key),
            command: get("command"),
            args: s
                .get("args")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|x| {
                            x.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| x.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default(),
            env: str_map(s.get("env")),
            cwd: get("cwd"),
            url,
            headers: str_map(s.get("headers")),
            enabled: !s.get("disabled").and_then(Value::as_bool).unwrap_or(false),
            ..Default::default()
        }
        .cleaned();
        out.push(server);
    }
    Ok(out)
}

/// A config key like "Home Assistant" or "brave-search" as a server name.
pub fn import_name(key: &str) -> String {
    let mut s = String::new();
    for c in key.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.is_empty() && !s.ends_with('_') {
            s.push('_');
        }
    }
    while s.ends_with('_') {
        s.pop();
    }
    if !s.starts_with(|c: char| c.is_ascii_lowercase()) {
        s.insert_str(0, "mcp_");
    }
    s.truncate(MAX_NAME);
    while s.ends_with('_') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names() {
        assert!(valid_name("github"));
        assert!(valid_name("home_assistant2"));
        assert!(!valid_name("Git"));
        assert!(!valid_name("a__b"));
        assert!(!valid_name("a_"));
        assert!(!valid_name("2fa"));
        assert_eq!(import_name("Home Assistant"), "home_assistant");
        assert_eq!(import_name("brave-search"), "brave_search");
        assert_eq!(import_name("@scope/x"), "scope_x");
        assert_eq!(import_name("123"), "mcp_123");
    }

    #[test]
    fn exposed_names_fit() {
        assert_eq!(
            exposed_name("github", "create_issue"),
            "github__create_issue"
        );
        assert_eq!(exposed_name("fs", "read.file"), "fs__read_file");
        let long = "x".repeat(80);
        let a = exposed_name("srv", &long);
        let b = exposed_name("srv", &format!("{long}y"));
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(server_of("github__create_issue"), Some("github"));
        assert_eq!(server_of("run_command"), None);
        assert_eq!(server_of("git_log"), None);
    }

    #[test]
    fn validates() {
        let ok = McpServerConfig {
            name: "time".into(),
            command: "uvx".into(),
            ..Default::default()
        };
        assert!(ok.validate().is_ok());
        let mut s = ok.clone();
        s.command.clear();
        assert!(s.validate().is_err());
        s.url = "ftp://x".into();
        assert!(s.validate().is_err());
        s.url = "https://example.com/mcp".into();
        assert!(s.validate().is_ok());
        assert_eq!(s.transport(), Transport::Http);
        assert!(
            validate_all(&[ok.clone(), ok])
                .unwrap_err()
                .contains("already")
        );
    }

    #[test]
    fn imports_claude_desktop_config() {
        let v = json!({"mcpServers": {
            "GitHub": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"],
                       "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "x"}},
            "remote-one": {"url": "https://example.com/mcp", "headers": {"Authorization": "Bearer t"}}
        }});
        let s = parse_import(&v).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "github");
        assert_eq!(s[0].args.len(), 2);
        assert_eq!(s[0].env["GITHUB_PERSONAL_ACCESS_TOKEN"], "x");
        assert_eq!(s[1].name, "remote_one");
        assert_eq!(s[1].transport(), Transport::Http);
        assert!(parse_import(&json!({"mcpServers": {}})).is_err());
    }

    #[test]
    fn approval() {
        use crate::config::ApprovalMode;
        assert!(!needs_approval(ApprovalMode::Never, false, false));
        assert!(needs_approval(ApprovalMode::Commands, false, false));
        assert!(needs_approval(ApprovalMode::AllChanges, false, false));
        assert!(!needs_approval(ApprovalMode::AllChanges, true, false));
        assert!(!needs_approval(ApprovalMode::Commands, false, true));
    }
}
