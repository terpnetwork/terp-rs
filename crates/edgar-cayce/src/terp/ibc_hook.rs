//! ibc-monitor speaks Slack-shaped webhooks. This module turns one of those
//! posts into a Discord line and decides whether it is a new step in the
//! expiry buffer (warning, then critical, then expired).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookMemory {
    pub label: String,
    pub hours: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAlert {
    pub key: String,
    pub label: String,
    pub hours: i64,
    pub message: String,
}

/// `true` when this status should be posted.
///
/// The same chain and client at the same label is quiet, so a retried
/// webhook does not double-post. A later label (warning → critical →
/// expired) posts. The same label with more hours left than last time
/// posts too: the client was updated and has fallen back into the buffer.
pub fn should_forward(prev: Option<&HookMemory>, label: &str, hours: i64) -> bool {
    if !matches!(label, "warning" | "critical" | "expired") {
        return false;
    }
    let Some(prev) = prev else {
        return true;
    };
    if prev.label != label {
        return true;
    }
    hours > prev.hours.saturating_add(6)
}

pub fn parse_hook(body: &Value) -> Option<HookAlert> {
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let mut chain = String::new();
    let mut client = String::new();
    let mut counterparty = String::new();
    let mut status = String::new();
    if let Some(fields) = body.pointer("/attachments/0/fields").and_then(|v| v.as_array()) {
        for field in fields {
            let title = field.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let value = field.get("value").and_then(|v| v.as_str()).unwrap_or("");
            match title {
                "chain" => chain = value.to_string(),
                "client" => client = value.to_string(),
                "counterparty" => counterparty = value.to_string(),
                "status" => status = value.to_string(),
                _ => {}
            }
        }
    }
    let label = hook_label(&status, text)?;
    if chain.is_empty() || client.is_empty() {
        return None;
    }
    let hours = hours_of(&status);
    let when = if hours < 0 {
        format!("{}h past expiry", -hours)
    } else {
        format!("{hours}h left")
    };
    let advice = if label == "expired" {
        "this client is expired; packets on it fail until it is replaced"
    } else {
        "update this client before the trusting period ends"
    };
    let cp = if counterparty.is_empty() {
        "?".to_string()
    } else {
        counterparty
    };
    Some(HookAlert {
        key: format!("{chain}:{client}"),
        label: label.to_string(),
        hours,
        message: format!(
            "**IBC client {label}** `{client}` on `{chain}`\ncounterparty `{cp}` — {when}\n{advice}"
        ),
    })
}

fn hook_label(status: &str, text: &str) -> Option<&'static str> {
    let status = status.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    for (name, label) in [
        ("expired", "expired"),
        ("critical", "critical"),
        ("warning", "warning"),
    ] {
        if status.starts_with(name) || text.contains(name) {
            return Some(label);
        }
    }
    None
}

fn hours_of(status: &str) -> i64 {
    let lower = status.to_ascii_lowercase();
    let ago = lower.contains("ago");
    let chars: Vec<char> = lower.chars().collect();
    let mut num = String::new();
    for (i, c) in chars.iter().copied().enumerate() {
        let digit_started = num.contains(|d: char| d.is_ascii_digit());
        if c.is_ascii_digit() || (c == '.' && digit_started) {
            num.push(c);
        } else if c == '-' && num.is_empty() && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
            // A spaced dash ("warning - 40.0h left") is a label separator.
            num.push('-');
        } else if digit_started {
            break;
        }
    }
    let hours = num.parse::<f64>().unwrap_or(0.0).trunc() as i64;
    if ago { -hours.abs() } else { hours }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(label: &str, hours: &str) -> serde_json::Value {
        json!({
            "text": format!("⚠️ ibc alert: {label}"),
            "attachments": [{
                "fields": [
                    {"title": "chain", "value": "morocco-1", "short": true},
                    {"title": "client", "value": "07-tendermint-23", "short": true},
                    {"title": "counterparty", "value": "osmosis-1", "short": true},
                    {"title": "status", "value": format!("{label} - {hours}"), "short": true}
                ]
            }]
        })
    }

    #[test]
    fn buffer_posts_each_step_once_and_a_refreshed_client_again() {
        let warning = parse_hook(&sample("warning", "40.0h left")).unwrap();
        assert!(warning.message.contains("40h left"));
        assert!(warning.message.contains("07-tendermint-23"));
        assert!(should_forward(None, &warning.label, warning.hours));
        let mem = HookMemory {
            label: warning.label.clone(),
            hours: warning.hours,
        };
        assert!(!should_forward(Some(&mem), "warning", 40));
        assert!(should_forward(Some(&mem), "critical", 20));
        assert!(should_forward(Some(&mem), "expired", -2));
        assert!(should_forward(Some(&mem), "warning", 48));
        assert!(!should_forward(Some(&mem), "error", 1));
    }
}
