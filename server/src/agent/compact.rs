//! Keeps long sessions going: when the prompt nears the model's context
//! size, older messages are folded into a summary the model writes itself.
//! Every message stays stored and shown; only the prompt gets shorter.

use serde_json::{Value, json};

use crate::config::Config;
use crate::db::Message;

/// Compact once the prompt passes this share of the context.
pub const TRIGGER: f64 = 0.75;
/// Recent messages kept word for word after compacting.
pub const KEEP: f64 = 0.25;
/// What's kept when llama-server already refused the prompt as too long.
pub const KEEP_AFTER_OVERFLOW: f64 = 0.12;
/// Starting guess for tokens per byte of request JSON, until llama-server
/// reports a real count.
pub const START_TOKENS_PER_BYTE: f64 = 0.3;

const SUMMARY_PROMPT: &str = "You keep the running summary of a long working session between a user and an AI agent. Older messages are being dropped from the agent's memory, and this summary is all it will have of them, so it must be able to carry on from it alone.\n\n\
Write the updated summary in Markdown with these sections:\n\
## Goal\nWhat the user wants overall, and what they asked for most recently.\n\
## Decisions and preferences\nChoices made, and the user's preferences, rules and constraints. Quote the user where the exact words matter.\n\
## Work done\nFiles created or changed (with paths), commands run and what they showed, key names, APIs, versions and settings.\n\
## Current state\nWhere things stand now, including anything broken, failing or unverified.\n\
## Next steps\nWhat's left to do, in order.\n\n\
Keep facts exact: paths, names, numbers and error messages. Merge the summary so far with the new messages, drop chit-chat and anything that was later undone or replaced, and keep it concise without losing anything the agent will need. Reply with the summary only.";

/// The latest summary in a history, and the messages after the point it
/// covers (summaries themselves left out).
pub fn view(history: &[Message]) -> (Option<&Message>, Vec<&Message>) {
    let summary = history
        .iter()
        .rev()
        .find(|m| m.role == "summary" && m.covers_upto.is_some());
    let upto = summary.and_then(|s| s.covers_upto).unwrap_or(i64::MIN);
    let rest = history
        .iter()
        .filter(|m| m.role != "summary" && m.id > upto)
        .collect();
    (summary, rest)
}

/// What the summary adds to the system prompt. The user's latest request is
/// repeated word for word when it's no longer among the kept messages.
pub fn system_section(summary: &Message, history: &[Message], kept: &[&Message]) -> String {
    let mut s = format!(
        "\n# Earlier in this conversation\nOlder messages were summarized to fit your context. The summary below replaces them; the messages after it are the most recent ones, word for word.\n\n{}\n",
        summary.content.trim()
    );
    let latest = history.iter().rev().find(|m| m.role == "user");
    if let Some(u) = latest
        && !kept.iter().any(|k| k.id == u.id)
    {
        s.push_str("\n# The user's latest request, word for word\n");
        s.push_str(&cut(&u.content, 8000));
        s.push('\n');
    }
    s
}

/// Size of a request in bytes of JSON, which is what the estimate scales.
/// Pictures count as this many bytes, whatever their size: the model sees
/// them as a fixed number of tiles, not as their base64 text.
pub const PICTURE_BYTES: usize = 6000;

pub fn prompt_bytes(body: &Value) -> usize {
    let len = |v: &Value| serde_json::to_string(v).map(|s| s.len()).unwrap_or(0);
    let pictures: usize = body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["content"].as_array())
        .flatten()
        .filter_map(|p| p.pointer("/image_url/url").and_then(Value::as_str))
        .map(|u| u.len().saturating_sub(PICTURE_BYTES))
        .sum();
    (len(&body["messages"]) + body.get("tools").map(len).unwrap_or(0)).saturating_sub(pictures)
}

/// Where to split `rest` so the kept part fits in `keep_bytes`: the start of
/// a user or assistant message, so tool results stay with their call. At
/// least one message is summarized and the newest turn is always kept.
/// None when there's nothing that can be summarized.
pub fn choose_cut(rest: &[&Message], keep_bytes: usize) -> Option<usize> {
    let starts: Vec<usize> = (1..rest.len())
        .filter(|&i| rest[i].role != "tool")
        .collect();
    let last = *starts.last()?;
    let mut size = 0usize;
    let mut best = last;
    for i in (1..rest.len()).rev() {
        size += message_bytes(rest[i]);
        if size > keep_bytes {
            break;
        }
        if rest[i].role != "tool" {
            best = i;
        }
    }
    Some(best.min(last))
}

fn message_bytes(m: &Message) -> usize {
    m.content.len()
        + m.tool_calls
            .iter()
            .map(|c| c.name.len() + c.arguments.len() + 40)
            .sum::<usize>()
        + 40
}

/// One message as plain text for the summarizer. Long parts are cut: the
/// summary needs what happened, not every byte of a file.
pub fn render(m: &Message) -> String {
    match m.role.as_str() {
        "user" if !m.images.is_empty() => format!(
            "USER (attached {} picture{}):\n{}\n",
            m.images.len(),
            if m.images.len() == 1 { "" } else { "s" },
            cut(&m.content, 6000)
        ),
        "user" => format!("USER:\n{}\n", cut(&m.content, 6000)),
        "assistant" => {
            let mut s = String::from("AGENT:\n");
            if !m.content.trim().is_empty() {
                s.push_str(&cut(&m.content, 4000));
                s.push('\n');
            }
            for c in &m.tool_calls {
                s.push_str(&format!("[called {} {}]\n", c.name, cut(&c.arguments, 400)));
            }
            s
        }
        "tool" => format!(
            "RESULT of {}{}:\n{}\n",
            m.tool_name.as_deref().unwrap_or("tool"),
            if m.is_error { " (error)" } else { "" },
            cut(&m.content, 1200)
        ),
        _ => String::new(),
    }
}

/// Groups rendered messages into pieces of at most about `max_bytes`.
pub fn chunks(rendered: &[String], max_bytes: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for r in rendered {
        if !cur.is_empty() && cur.len() + r.len() > max_bytes {
            out.push(std::mem::take(&mut cur));
        }
        // A single message bigger than a chunk is cut to fit.
        if r.len() > max_bytes {
            cur.push_str(&cut(r, max_bytes / 2));
        } else {
            cur.push_str(r);
        }
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Longest summary to ask for, in tokens.
pub fn summary_tokens(ctx: u64) -> u64 {
    (ctx / 12).clamp(1024, 8192).min(ctx / 4)
}

/// A non-streaming request that folds `part` into the summary so far.
pub fn summary_request(
    conf: &Config,
    previous: Option<&str>,
    part: &str,
    max_tokens: u64,
) -> Value {
    let so_far = previous
        .filter(|p| !p.trim().is_empty())
        .unwrap_or("(none yet: this is the start of the session)");
    json!({
        "model": conf.llama.model_name,
        "messages": [
            {"role": "system", "content": SUMMARY_PROMPT},
            {"role": "user", "content": format!(
                "SUMMARY SO FAR:\n{so_far}\n\nNEW MESSAGES TO FOLD IN:\n{part}\n\nWrite the updated summary."
            )},
        ],
        "stream": false,
        "temperature": 0.3,
        "max_tokens": max_tokens,
        "chat_template_kwargs": { "enable_thinking": false },
    })
}

/// The summary text from a model reply, without any thinking.
pub fn clean_summary(raw: &str) -> Option<String> {
    let text = raw.rsplit("</think>").next().unwrap_or(raw).trim();
    (text.len() >= 20).then(|| text.to_string())
}

fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [... {} more bytes cut]", &s[..end], s.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ToolCall;

    fn msg(id: i64, role: &str, content: &str) -> Message {
        Message {
            id,
            session_id: "s".into(),
            role: role.into(),
            content: content.into(),
            reasoning: None,
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            is_error: false,
            created_at: String::new(),
            covers_upto: None,
            images: vec![],
        }
    }

    #[test]
    fn view_starts_after_the_latest_summary() {
        let mut s1 = msg(4, "summary", "old");
        s1.covers_upto = Some(2);
        let mut s2 = msg(7, "summary", "new");
        s2.covers_upto = Some(5);
        let h = vec![
            msg(1, "user", "a"),
            msg(2, "assistant", "b"),
            msg(3, "user", "c"),
            s1,
            msg(5, "assistant", "d"),
            msg(6, "user", "e"),
            s2,
            msg(8, "assistant", "f"),
        ];
        let (summary, rest) = view(&h);
        assert_eq!(summary.map(|s| s.content.as_str()), Some("new"));
        let ids: Vec<i64> = rest.iter().map(|m| m.id).collect();
        assert_eq!(ids, [6, 8]);
    }

    #[test]
    fn cuts_at_turn_starts_and_keeps_the_newest() {
        let mut a = msg(2, "assistant", "");
        a.tool_calls = vec![ToolCall {
            id: "c".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        }];
        let big = "x".repeat(5000);
        let h = [
            msg(1, "user", "build it"),
            a,
            msg(3, "tool", &big),
            msg(4, "assistant", "done part one"),
            msg(5, "user", "now part two"),
            msg(6, "assistant", "ok"),
        ];
        let rest: Vec<&Message> = h.iter().collect();
        // Room for the last two: cut before "now part two".
        assert_eq!(choose_cut(&rest, 120), Some(4));
        // No room at all: still keep the newest turn.
        assert_eq!(choose_cut(&rest, 0), Some(5));
        // Room for everything: only the first message is summarized.
        assert_eq!(choose_cut(&rest, 1_000_000), Some(1));
        // A lone message can't be summarized.
        assert_eq!(choose_cut(&rest[..1], 100), None);
    }

    #[test]
    fn repeats_the_latest_request_when_it_was_summarized() {
        let mut s = msg(4, "summary", "## Goal\nan app");
        s.covers_upto = Some(2);
        let h = vec![
            msg(1, "user", "make me an app"),
            msg(2, "assistant", "on it"),
            msg(3, "assistant", "still going"),
        ];
        let kept: Vec<&Message> = vec![&h[2]];
        let text = system_section(&s, &h, &kept);
        assert!(text.contains("## Goal\nan app"));
        assert!(text.contains("word for word\nmake me an app"));
        let kept: Vec<&Message> = h.iter().collect();
        assert!(!system_section(&s, &h, &kept).contains("latest request"));
    }

    #[test]
    fn chunks_stay_under_the_limit() {
        let parts: Vec<String> = (0..10).map(|i| format!("{i}{}", "y".repeat(300))).collect();
        let out = chunks(&parts, 1000);
        assert!(out.len() >= 4);
        assert!(out.iter().all(|c| c.len() <= 1100));
        let huge = vec!["z".repeat(5000)];
        assert!(chunks(&huge, 1000)[0].len() < 1000);
    }

    #[test]
    fn cleans_summaries() {
        assert_eq!(
            clean_summary("<think>hm</think>\n## Goal\nShip the app").as_deref(),
            Some("## Goal\nShip the app")
        );
        assert_eq!(clean_summary("  ok "), None);
    }
}
