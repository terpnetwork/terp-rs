//! Communication-style profile mirroring `returniflost`.
//!
//! Workflow: export the user's messages (Discord data package or bot-logged
//! history with Message Content intent), drop the JSON/CSV into
//! `./data/style/returniflost.json`, then `/terp profile-refresh` rebuilds
//! this profile. The digest + RLM answer layers inject `STYLE_PREAMBLE`
//! so updates sound like the maintainer, not a generic bot.

use std::collections::HashMap;
use std::path::PathBuf;

/// Tunable voice knobs curated from returniflost's public posts:
/// lowercase-leaning, terse, builder cadence, no hype.
pub const STYLE_PREAMBLE: &str = "\
Voice: terse, lowercase-leaning, builder cadence. Plain words, short sentences. \
No hype, no emojis, no exclamation spam. Lead with the fact, then the link or hash. \
Sign updates like: `shipping: <what> — <where>`. Never invent numbers.";

#[derive(Debug, Default, Clone)]
pub struct StyleProfile {
    pub avg_len: f64,
    pub lowercase_ratio: f64,
    pub top_tokens: Vec<(String, usize)>,
    pub message_count: usize,
}

impl StyleProfile {
    /// Build a profile from raw message strings.
    pub fn from_messages(msgs: &[String]) -> Self {
        let mut total_len = 0usize;
        let mut lower_chars = 0usize;
        let mut alpha_chars = 0usize;
        let mut freq: HashMap<String, usize> = HashMap::new();
        for m in msgs {
            total_len += m.len();
            for c in m.chars() {
                if c.is_alphabetic() {
                    alpha_chars += 1;
                    if c.is_lowercase() {
                        lower_chars += 1;
                    }
                }
            }
            for tok in m.split_whitespace().take(50) {
                let t = tok.to_lowercase();
                if t.len() > 3 {
                    *freq.entry(t).or_insert(0) += 1;
                }
            }
        }
        let n = msgs.len().max(1);
        let mut top: Vec<(String, usize)> = freq.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        top.truncate(25);
        Self {
            avg_len: total_len as f64 / n as f64,
            lowercase_ratio: if alpha_chars == 0 {
                0.0
            } else {
                lower_chars as f64 / alpha_chars as f64
            },
            top_tokens: top,
            message_count: msgs.len(),
        }
    }

    pub fn style_hint(&self) -> String {
        format!(
            "{}\nStats: n={} avg_len={:.0} lowercase_ratio={:.2}. Favor its top tokens where natural.",
            STYLE_PREAMBLE, self.message_count, self.avg_len, self.lowercase_ratio
        )
    }
}

/// Load `data/style/returniflost.json` (array of strings or
/// `[{content|body|text}]`) if present.
pub fn load_returniflost_corpus(data_dir: &std::path::Path) -> Vec<String> {
    let path: PathBuf = data_dir.join("style").join("returniflost.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(&raw) else {
        return Vec::new();
    };
    match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|it| match it {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Object(map) => map
                    .get("content")
                    .or_else(|| map.get("body"))
                    .or_else(|| map.get("text"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
