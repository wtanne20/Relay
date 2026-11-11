//! Model routing: quick questions go to a small model, heavy work to a big
//! one. The choice is made from the prompt alone (no model call), so it
//! costs nothing and works while no model is loaded.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Quick,
    Heavy,
}

/// What the prompt looks like, for [`classify`].
pub struct Ask<'a> {
    pub prompt: &'a str,
    /// Pictures or files are attached.
    pub attachments: bool,
    /// The chat works in a project folder (the agent has file and command
    /// tools there), rather than being a plain chat.
    pub has_folder: bool,
    /// An earlier reply in this chat was routed to the heavy model.
    pub was_heavy: bool,
    pub quick_max_words: u32,
}

/// Words that ask for real work: writing or changing code, planning,
/// careful analysis. A word matches with a plain ending ("fixes",
/// "planning"); one ending in `*` is a stem that matches any ending.
const HEAVY_WORDS: &[&str] = &[
    "implement",
    "refactor",
    "debug",
    "fix",
    "build",
    "create",
    "write",
    "writing",
    "rewrite",
    "generate",
    "code",
    "program",
    "script",
    "function",
    "class",
    "analy*",
    "review",
    "compare",
    "design",
    "plan",
    "architect*",
    "optimi*",
    "translate",
    "summari*",
    "step by step",
    "in detail",
    "detailed",
    "thorough",
    "essay",
    "report",
    "prove",
    "derive",
    "algorithm",
    "migrate",
    "test",
];

/// Picks the model size for a prompt, with a short reason for the app.
pub fn classify(a: &Ask) -> (Route, &'static str) {
    if a.has_folder {
        return (Route::Heavy, "project work");
    }
    if a.attachments {
        return (Route::Heavy, "has attachments");
    }
    if a.was_heavy {
        return (Route::Heavy, "this chat needed the big model before");
    }
    let words = a.prompt.split_whitespace().count();
    if words > a.quick_max_words as usize {
        return (Route::Heavy, "long prompt");
    }
    if a.prompt.contains("```") || a.prompt.lines().count() > 6 {
        return (Route::Heavy, "has code or long text");
    }
    let lower = a.prompt.to_lowercase();
    if HEAVY_WORDS.iter().any(|w| has_word_start(&lower, w)) {
        return (Route::Heavy, "asks for real work");
    }
    (Route::Quick, "quick question")
}

/// Endings a listed word may carry and still count.
const ENDINGS: &[&str] = &["", "s", "es", "d", "ed", "ing", "ning", "ting"];

/// True if `word` (see [`HEAVY_WORDS`]) appears in `hay` as a word.
fn has_word_start(hay: &str, word: &str) -> bool {
    let (needle, stem) = match word.strip_suffix('*') {
        Some(w) => (w, true),
        None => (word, false),
    };
    hay.match_indices(needle).any(|(i, _)| {
        let starts = hay[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let rest = &hay[i + needle.len()..];
        let ending: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
        starts && (stem || ENDINGS.contains(&ending.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(prompt: &str) -> Ask<'_> {
        Ask {
            prompt,
            attachments: false,
            has_folder: false,
            was_heavy: false,
            quick_max_words: 60,
        }
    }

    #[test]
    fn short_questions_are_quick() {
        for p in [
            "what's the capital of France?",
            "how many ounces in a cup",
            "Is 97 prime?",
            "hi",
        ] {
            assert_eq!(classify(&ask(p)).0, Route::Quick, "{p}");
        }
    }

    #[test]
    fn real_work_is_heavy() {
        for p in [
            "Write a Python script that renames my photos by date",
            "can you refactor this?",
            "Explain step by step how TLS works",
            "summarize this article for me",
            "```rust\nfn main() {}\n```",
        ] {
            assert_eq!(classify(&ask(p)).0, Route::Heavy, "{p}");
        }
        let long = "word ".repeat(61);
        assert_eq!(classify(&ask(&long)).0, Route::Heavy);
    }

    #[test]
    fn words_match_at_their_start_only() {
        // "prefix" contains "fix", "contest" contains "test".
        assert_eq!(classify(&ask("what does prefix mean")).0, Route::Quick);
        assert_eq!(classify(&ask("who won the contest")).0, Route::Quick);
        assert_eq!(classify(&ask("any tests failing?")).0, Route::Heavy);
        // "planet" is not "plan", "classic" is not "class".
        assert_eq!(classify(&ask("how far is the planet mars")).0, Route::Quick);
        assert_eq!(classify(&ask("best classic movies?")).0, Route::Quick);
        assert_eq!(classify(&ask("help me with planning a trip")).0, Route::Heavy);
    }

    #[test]
    fn context_makes_it_heavy() {
        let mut a = ask("hi");
        a.has_folder = true;
        assert_eq!(classify(&a).0, Route::Heavy);
        let mut a = ask("what is this?");
        a.attachments = true;
        assert_eq!(classify(&a).0, Route::Heavy);
        let mut a = ask("thanks!");
        a.was_heavy = true;
        assert_eq!(classify(&a).0, Route::Heavy);
    }
}
