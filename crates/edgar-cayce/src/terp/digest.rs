//! Dev digest and the daily voting-power file.
//!
//! `render_digest` is on-demand (`/terp digest-preview`). There is no
//! scheduler in the bot. `render_voting_power` turns the validator stake
//! diff into a short Discord line plus a Markdown attachment.

use crate::terp::profile::STYLE_PREAMBLE;

/// Build a digest message from pre-collected `lines` (commit subjects etc).
pub fn render_digest(lines: &[String]) -> String {
    if lines.is_empty() {
        return "no changes today. touching grass.".to_string();
    }
    let mut out = String::from("**daily dev update**\n");
    for l in lines.iter().take(15) {
        out.push_str(&format!("- {}\n", l));
    }
    out.push_str(&format!(
        "\n_{}",
        STYLE_PREAMBLE.split('.').next().unwrap_or("")
    ));
    out
}

/// One validator movement in the daily stake diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerChange {
    pub moniker: String,
    pub address: String,
    pub kind: String,
    pub before: String,
    pub after: String,
    pub delta: String,
}

/// Short channel line plus the `.md` body posted with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotingPowerPost {
    pub inline: String,
    pub filename: String,
    pub markdown: String,
}

/// `bonded_line` is already display text (`bonded 1 TERP → 2 TERP (+100.00%)`).
pub fn render_voting_power(
    day: &str,
    bonded_line: &str,
    changes: &[PowerChange],
) -> VotingPowerPost {
    let filename = format!("voting-power-{}.md", safe_day(day));
    let inline = inline_summary(day, bonded_line, changes, &filename);
    let markdown = markdown_body(day, bonded_line, changes);
    VotingPowerPost {
        inline,
        filename,
        markdown,
    }
}

fn inline_summary(day: &str, bonded_line: &str, changes: &[PowerChange], filename: &str) -> String {
    let mut lines = vec![format!("**Voting power** `{day}`"), bonded_line.to_string()];
    if changes.is_empty() {
        lines.push("no validator set or >1% token changes".into());
    } else {
        let blurbs: Vec<String> = changes.iter().take(3).map(change_blurb).collect();
        let more = changes.len().saturating_sub(blurbs.len());
        let mut summary = format!(
            "{} change{}: {}",
            changes.len(),
            if changes.len() == 1 { "" } else { "s" },
            blurbs.join(", ")
        );
        if more > 0 {
            summary.push_str(&format!(" +{more} more"));
        }
        summary.push_str(&format!(". full table attached (`{filename}`)."));
        lines.push(summary);
    }
    lines.join("\n")
}

fn change_blurb(c: &PowerChange) -> String {
    let text = match c.kind.as_str() {
        "joined" => format!("+ {}", c.moniker),
        "left" => format!("- {}", c.moniker),
        "jailed" => format!("jailed {}", c.moniker),
        "unjailed" => format!("unjailed {}", c.moniker),
        "status" => format!("{} {} → {}", c.moniker, c.before, c.after),
        "stake" => format!("{} {}", c.moniker, c.delta),
        _ => format!("{} {}", c.kind, c.moniker),
    };
    text.chars().take(80).collect()
}

fn markdown_body(day: &str, bonded_line: &str, changes: &[PowerChange]) -> String {
    let mut md = format!("# Voting power {day}\n\n{bonded_line}\n\n");
    if changes.is_empty() {
        md.push_str("No validator set changes and no stake moves of 1% or more.\n");
        return md;
    }
    md.push_str("| Validator | Address | Change | Before | After | Delta |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for c in changes.iter().take(500) {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            cell(&c.moniker),
            addr_cell(&c.address),
            cell(&c.kind),
            cell(&c.before),
            cell(&c.after),
            cell(&c.delta),
        ));
    }
    if changes.len() > 500 {
        md.push_str(&format!("\n{} more rows omitted.\n", changes.len() - 500));
    }
    md
}

fn addr_cell(address: &str) -> String {
    if address.is_empty() {
        String::new()
    } else {
        format!("`{}`", cell(address))
    }
}

fn cell(raw: &str) -> String {
    raw.replace('|', "\\|")
        .replace('\n', " ")
        .replace('\r', " ")
        .replace('`', "'")
}

fn safe_day(day: &str) -> String {
    let s: String = day
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if s.is_empty() {
        "day".into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: &str, moniker: &str) -> PowerChange {
        PowerChange {
            moniker: moniker.into(),
            address: "terpvaloper1abc".into(),
            kind: kind.into(),
            before: String::new(),
            after: String::new(),
            delta: String::new(),
        }
    }

    #[test]
    fn quiet_day_is_short_and_still_a_markdown_file() {
        let post = render_voting_power("2026-09-15", "bonded 1 TERP → 1 TERP (+0.00%)", &[]);
        assert_eq!(post.filename, "voting-power-2026-09-15.md");
        assert!(post
            .inline
            .contains("no validator set or >1% token changes"));
        assert!(post.inline.contains("**Voting power** `2026-09-15`"));
        assert!(post.markdown.starts_with("# Voting power 2026-09-15"));
        assert!(!post.inline.contains("| --- |"));
        assert!(post.markdown.contains("No validator set changes"));
    }

    #[test]
    fn changes_stay_out_of_the_inline_table_and_land_in_markdown() {
        let mut jail = change("jailed", "alpha");
        jail.before = "no".into();
        jail.after = "yes".into();
        let mut stake = change("stake", "a|b");
        stake.before = "1 TERP".into();
        stake.after = "2 TERP".into();
        stake.delta = "+100.00%".into();
        let post = render_voting_power(
            "2026-09-15",
            "bonded 10 TERP → 11 TERP (+10.00%)",
            &[jail, stake],
        );
        assert!(post.inline.contains("jailed alpha"));
        assert!(post.inline.contains("voting-power-2026-09-15.md"));
        assert!(!post.inline.contains("terpvaloper1abc"));
        assert!(post
            .markdown
            .contains("| alpha | `terpvaloper1abc` | jailed |"));
        assert!(post.markdown.contains("a\\|b"));
        assert!(post.markdown.contains("+100.00%"));
        assert!(post.inline.len() < post.markdown.len());
    }
}
