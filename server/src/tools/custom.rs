//! Tools the user adds in the app. Each one is a named command with a few
//! text parameters: the model fills in the parameters, Relay puts them into
//! the command (quoted for the shell, so a value can never add commands of
//! its own) and runs it in the project folder like `run_command`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Longest name, description and command Relay accepts.
pub const MAX_NAME: usize = 64;
pub const MAX_DESCRIPTION: usize = 2000;
pub const MAX_COMMAND: usize = 8000;
pub const MAX_PARAMS: usize = 16;
pub const MAX_TOOLS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CustomTool {
    /// What the model calls it, e.g. `git_log`. Lowercase letters, digits
    /// and underscores.
    pub name: String,
    /// Tells the model what the tool does and when to use it.
    pub description: String,
    /// The command to run. `{param}` is replaced with that parameter's value.
    pub command: String,
    pub parameters: Vec<CustomParam>,
    pub enabled: bool,
    /// Kill the command after this many seconds. 0 = the agent's command
    /// timeout.
    pub timeout_seconds: u64,
}

impl Default for CustomTool {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            command: String::new(),
            parameters: Vec::new(),
            enabled: true,
            timeout_seconds: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CustomParam {
    pub name: String,
    pub description: String,
    pub required: bool,
}

impl Default for CustomParam {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            required: true,
        }
    }
}

fn valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
        && s.len() <= MAX_NAME
}

/// Names the model already knows: a custom tool can't take one of these.
pub fn reserved(name: &str) -> bool {
    super::catalog().iter().any(|(n, _)| *n == name)
        || crate::knowledge::TOOL_NAMES.contains(&name)
}

impl CustomTool {
    /// Checks one tool on its own (duplicates are checked by [`validate_all`]).
    pub fn validate(&self) -> Result<(), String> {
        let name = &self.name;
        if !valid_ident(name) {
            return Err(format!(
                "tool name '{name}' must start with a lowercase letter and use only a-z, 0-9 and _ (at most {MAX_NAME} characters)"
            ));
        }
        if reserved(name) {
            return Err(format!("'{name}' is a built-in tool; pick another name"));
        }
        if self.description.trim().is_empty() {
            return Err(format!("tool '{name}' needs a description, so the model knows when to use it"));
        }
        if self.description.len() > MAX_DESCRIPTION {
            return Err(format!("tool '{name}': the description is too long (at most {MAX_DESCRIPTION} characters)"));
        }
        if self.command.trim().is_empty() {
            return Err(format!("tool '{name}' needs a command to run"));
        }
        if self.command.len() > MAX_COMMAND {
            return Err(format!("tool '{name}': the command is too long (at most {MAX_COMMAND} characters)"));
        }
        if self.timeout_seconds > super::MAX_COMMAND_TIMEOUT {
            return Err(format!(
                "tool '{name}': the timeout must be at most {} seconds",
                super::MAX_COMMAND_TIMEOUT
            ));
        }
        if self.parameters.len() > MAX_PARAMS {
            return Err(format!("tool '{name}' has too many parameters (at most {MAX_PARAMS})"));
        }
        for (i, p) in self.parameters.iter().enumerate() {
            if !valid_ident(&p.name) {
                return Err(format!(
                    "tool '{name}': parameter '{}' must start with a lowercase letter and use only a-z, 0-9 and _",
                    p.name
                ));
            }
            if self.parameters[..i].iter().any(|q| q.name == p.name) {
                return Err(format!("tool '{name}' has two parameters named '{}'", p.name));
            }
            if !self.command.contains(&format!("{{{}}}", p.name)) {
                return Err(format!(
                    "tool '{name}': the command doesn't use parameter '{0}'. Put {{{0}}} where its value goes",
                    p.name
                ));
            }
        }
        Ok(())
    }

    /// The OpenAI-style definition sent to the model.
    pub fn definition(&self) -> Value {
        let mut props = serde_json::Map::new();
        for p in &self.parameters {
            props.insert(
                p.name.clone(),
                json!({ "type": "string", "description": p.description }),
            );
        }
        let required: Vec<&str> = self
            .parameters
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name.as_str())
            .collect();
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": required,
                }
            }
        })
    }

    /// The command with the model's values put in, quoted for `shell`.
    /// A missing optional parameter is left out.
    pub fn expand(&self, arguments: &str, shell: &str) -> Result<String, String> {
        let args: Value = serde_json::from_str(if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        })
        .map_err(|e| format!("arguments are not valid JSON ({e})"))?;
        let mut command = self.command.clone();
        for p in &self.parameters {
            let value = match args.get(&p.name) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                // Small models often send numbers and booleans unquoted.
                Some(v @ (Value::Number(_) | Value::Bool(_))) => Some(v.to_string()),
                Some(_) => return Err(format!("parameter '{}' must be text", p.name)),
            };
            let quoted = match value {
                Some(v) => quote(&v, shell)?,
                None if p.required => {
                    return Err(format!("missing required parameter '{}'", p.name));
                }
                None => String::new(),
            };
            command = command.replace(&format!("{{{}}}", p.name), &quoted);
        }
        Ok(command)
    }
}

/// Checks the whole list: each tool, and no two with the same name.
pub fn validate_all(tools: &[CustomTool]) -> Result<(), String> {
    if tools.len() > MAX_TOOLS {
        return Err(format!("at most {MAX_TOOLS} tools of your own"));
    }
    for (i, t) in tools.iter().enumerate() {
        t.validate()?;
        if tools[..i].iter().any(|u| u.name == t.name) {
            return Err(format!("there is already a tool named '{}'", t.name));
        }
    }
    Ok(())
}

/// One value as a single literal argument for the shell, so nothing in it
/// is run or expanded.
pub fn quote(value: &str, shell: &str) -> Result<String, String> {
    let lower = shell.to_lowercase();
    if lower.contains("powershell") || lower.contains("pwsh") {
        // PowerShell treats the curly single quotes as quotes too.
        let mut s = String::from("'");
        for c in value.chars() {
            if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
                s.push(c);
            }
            s.push(c);
        }
        s.push('\'');
        Ok(s)
    } else if lower.ends_with("cmd") || lower.ends_with("cmd.exe") {
        // cmd.exe has no quoting that stops % and ! from expanding.
        if value
            .chars()
            .any(|c| matches!(c, '"' | '%' | '!' | '\r' | '\n' | '^'))
        {
            return Err("values can't contain \" % ! ^ or line breaks when the shell is cmd".into());
        }
        Ok(format!("\"{value}\""))
    } else {
        Ok(format!("'{}'", value.replace('\'', r"'\''")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> CustomTool {
        CustomTool {
            name: "git_log".into(),
            description: "Show recent commits".into(),
            command: "git log -n {count} --grep {text}".into(),
            parameters: vec![
                CustomParam {
                    name: "count".into(),
                    description: "How many".into(),
                    required: true,
                },
                CustomParam {
                    name: "text".into(),
                    description: "Filter".into(),
                    required: false,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn validates() {
        assert!(tool().validate().is_ok());
        let mut t = tool();
        t.name = "Git Log".into();
        assert!(t.validate().is_err());
        t.name = "run_command".into();
        assert!(t.validate().unwrap_err().contains("built-in"));
        let mut t = tool();
        t.command = "git log".into();
        assert!(t.validate().unwrap_err().contains("{count}"));
        let mut t = tool();
        t.description = " ".into();
        assert!(t.validate().is_err());
        assert!(validate_all(&[tool(), tool()]).unwrap_err().contains("already"));
    }

    #[test]
    fn quotes_values_for_each_shell() {
        let t = tool();
        assert_eq!(
            t.expand(r#"{"count": 5, "text": "it's; rm -rf /"}"#, "sh").unwrap(),
            r"git log -n '5' --grep 'it'\''s; rm -rf /'"
        );
        assert_eq!(
            t.expand(r#"{"count": "3", "text": "a'b $(x)"}"#, "powershell.exe").unwrap(),
            "git log -n '3' --grep 'a''b $(x)'"
        );
        assert_eq!(
            t.expand(r#"{"count": "1", "text": "a’b"}"#, "pwsh").unwrap(),
            "git log -n '1' --grep 'a\u{2019}\u{2019}b'"
        );
        assert_eq!(t.expand(r#"{"count": "2"}"#, "sh").unwrap(), "git log -n '2' --grep ");
        assert!(t.expand("{}", "sh").unwrap_err().contains("count"));
        assert!(t.expand(r#"{"count": "1", "text": "100%"}"#, "cmd.exe").is_err());
        assert_eq!(
            t.expand(r#"{"count": "1", "text": "a b"}"#, "cmd.exe").unwrap(),
            "git log -n \"1\" --grep \"a b\""
        );
    }

    #[test]
    fn definition_lists_parameters() {
        let d = tool().definition();
        assert_eq!(d.pointer("/function/name").unwrap(), "git_log");
        assert_eq!(d.pointer("/function/parameters/required/0").unwrap(), "count");
        assert!(d.pointer("/function/parameters/properties/text").is_some());
    }
}
