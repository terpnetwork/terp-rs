//! Morning news desk: plain language for Discord and for the spoken clip.
//!
//! Team notes land in `./data/desk-inbox` as small text files. The header
//! says how the desk may use the note. The body is never executed.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContribMode {
    /// Read the body as the author wrote it.
    Quote,
    /// Keep the first sentence and attribute it.
    Summarize,
    /// Show on the written desk. Leave it out of the spoken clip.
    Omit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contrib {
    pub mode: ContribMode,
    pub from: String,
    pub topic: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StakeAction {
    Joined,
    Left,
    Jailed,
    Unjailed,
    Bonded,
    Unbonded,
    StatusChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakeMove {
    pub name: String,
    pub action: StakeAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desk {
    /// One line for the Discord message. The bullets live in `discord`.
    pub caption: String,
    pub discord: String,
    pub speech: String,
}

pub fn parse_contrib(raw: &str) -> Result<Contrib, String> {
    let raw = raw.trim_start_matches('\u{feff}');
    let (head, body) = raw
        .split_once("\n---")
        .ok_or_else(|| "missing --- between the header and the body".to_string())?;
    let mut mode = None;
    let mut from = None;
    let mut topic = None;
    for line in head.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("header line is not key: value ({line})"))?;
        let value = clean_inline(value);
        if value.is_empty() {
            return Err(format!("empty {key}"));
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "mode" => {
                mode = Some(match value.to_ascii_lowercase().as_str() {
                    "quote" | "direct" => ContribMode::Quote,
                    "summarize" | "summary" => ContribMode::Summarize,
                    "omit" | "written" => ContribMode::Omit,
                    other => return Err(format!("unknown mode {other}")),
                });
            }
            "from" => from = Some(value),
            "topic" => topic = Some(value),
            other => return Err(format!("unknown header {other}")),
        }
    }
    let mode = mode.ok_or_else(|| "mode is required".to_string())?;
    let from = from.ok_or_else(|| "from is required".to_string())?;
    let topic = topic.unwrap_or_else(|| "the desk".into());
    let body = clean_body(body);
    if body.is_empty() {
        return Err("body is empty".into());
    }
    if body.chars().count() > 2000 {
        return Err("body is longer than 2000 characters".into());
    }
    Ok(Contrib {
        mode,
        from,
        topic,
        body,
    })
}

fn clean_inline(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect()
}

fn clean_body(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c == '\n' || c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    out.trim().to_string()
}

pub fn compose_desk(
    day: &str,
    bonded_line: &str,
    stake_changes: &str,
    events: &[String],
    dao_lines: &[String],
    contribs: &[Contrib],
    rejected: &[String],
    jail: &str,
    moves: &[StakeMove],
) -> Desk {
    let (proposals, ibc, other) = split_events(events);
    let mut discord = format!("# Terp desk {day}\n");
    discord.push_str("\n## Proposals\n");
    push_section(&mut discord, &proposals, "No new chain proposals since the last desk.");
    discord.push_str("\n## Foundation DAO\n");
    push_section(&mut discord, dao_lines, "No new foundation action since the last desk.");
    discord.push_str("\n## Stake\n");
    if !bonded_line.trim().is_empty() {
        discord.push_str("- ");
        discord.push_str(bonded_line.trim());
        discord.push('\n');
    }
    if moves.is_empty() {
        let quiet = stake_changes.trim();
        if quiet.is_empty() || quiet.contains("no validator") {
            discord.push_str("- No validator moved by more than one percent.\n");
        } else {
            discord.push_str("- ");
            discord.push_str(quiet);
            discord.push('\n');
        }
    } else {
        for line in written_moves(moves) {
            discord.push_str("- ");
            discord.push_str(&line);
            discord.push('\n');
        }
    }
    if !jail.trim().is_empty() {
        discord.push_str("- ");
        discord.push_str(&spoken(jail));
        discord.push('\n');
    }
    discord.push_str("\n## IBC\n");
    push_section(&mut discord, &ibc, "No new channels or clients since the last desk.");
    if !other.is_empty() {
        discord.push_str("\n## Also\n");
        push_section(&mut discord, &other, "");
    }
    if !contribs.is_empty() || !rejected.is_empty() {
        discord.push_str("\n## From the team\n");
        for note in contribs {
            let label = match note.mode {
                ContribMode::Quote => "as sent",
                ContribMode::Summarize => "folded to one sentence",
                ContribMode::Omit => "written only",
            };
            discord.push_str(&format!(
                "- {} ({label}), {}: {}\n",
                note.from,
                note.topic,
                one_paragraph(&note.body, 400)
            ));
        }
        for name in rejected {
            discord.push_str(&format!(
                "- left unread: `{name}` needs mode quote, summarize, or omit\n"
            ));
        }
    }

    let mut speech = format!("Good morning. This is the Terp desk for {}. ", speak_day(day));
    speech.push_str(&proposal_speech(&proposals));
    speech.push(' ');
    speech.push_str(&dao_speech(dao_lines));
    speech.push(' ');
    speech.push_str(&stake_speech(bonded_line, moves));
    speech.push(' ');
    speech.push_str(&ibc_speech(&ibc));
    if !jail.trim().is_empty() {
        speech.push(' ');
        speech.push_str(&narrate_jail(jail));
    }
    for note in contribs {
        match note.mode {
            ContribMode::Omit => {}
            ContribMode::Quote => {
                speech.push_str(&format!(
                    " From {}, in their own words, on {}: {}.",
                    note.from,
                    note.topic,
                    one_paragraph(&note.body, 240)
                ));
            }
            ContribMode::Summarize => {
                speech.push_str(&format!(
                    " From {}, on {}: {}.",
                    note.from,
                    note.topic,
                    first_sentence(&note.body)
                ));
            }
        }
    }
    speech.push_str(" That is the desk.");
    let speech = say_numbers(&drop_raw_tokens(&speech));
    let speech = speech.split_whitespace().collect::<Vec<_>>().join(" ");
    let caption = format!("Terp desk for {day}");
    Desk {
        caption,
        discord,
        speech,
    }
}

fn push_section(out: &mut String, lines: &[String], empty: &str) {
    if lines.is_empty() {
        if !empty.is_empty() {
            out.push_str("- ");
            out.push_str(empty);
            out.push('\n');
        }
        return;
    }
    for line in lines.iter().take(8) {
        out.push_str("- ");
        out.push_str(&spoken(line));
        out.push('\n');
    }
    if lines.len() > 8 {
        out.push_str(&format!("- {} more lines are in the attached file.\n", lines.len() - 8));
    }
}

fn split_events(events: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut proposals = Vec::new();
    let mut ibc = Vec::new();
    let mut other = Vec::new();
    for event in events {
        let plain = spoken(event).to_ascii_lowercase();
        if plain.contains("proposal") || plain.contains("voting") || plain.contains("deposit period") {
            proposals.push(event.clone());
        } else if plain.contains("ibc")
            || plain.starts_with("opened ")
            || plain.starts_with("closed ")
            || plain.starts_with("counterparty ")
            || plain.starts_with("client ")
            || plain.contains("channel")
        {
            ibc.push(event.clone());
        } else {
            other.push(event.clone());
        }
    }
    (proposals, ibc, other)
}

fn proposal_speech(lines: &[String]) -> String {
    if lines.is_empty() {
        return "No new chain proposal since the last desk.".into();
    }
    let told: Vec<String> = lines.iter().take(3).map(|l| narrate_gov(l)).collect();
    if lines.len() == 1 {
        return format!("One chain proposal. {}", told[0]);
    }
    format!("{} chain proposals. {}", lines.len(), told.join(" "))
}

fn dao_speech(lines: &[String]) -> String {
    if lines.is_empty() {
        return "The foundation DAO took no new action.".into();
    }
    let told: Vec<String> = lines.iter().take(3).map(|l| narrate_dao(l)).collect();
    if lines.len() == 1 {
        return format!("Foundation DAO. {}", told[0]);
    }
    format!("Foundation DAO recorded {} actions. {}", lines.len(), told.join(" "))
}

fn stake_speech(bonded: &str, moves: &[StakeMove]) -> String {
    let trend = bonded_trend(bonded);
    let people = narrate_moves(moves);
    match (trend, people) {
        (Some(trend), Some(people)) => format!("{trend} {people}"),
        (Some(trend), None) => format!(
            "{trend} No single validator moved by more than one percent."
        ),
        (None, Some(people)) => people,
        (None, None) => {
            "The validator set was quiet, and no stake moved by more than one percent.".into()
        }
    }
}

fn bonded_trend(bonded: &str) -> Option<String> {
    let start = bonded.rfind('(')?;
    let end = bonded.rfind(')')?;
    if end <= start {
        return None;
    }
    let inner = bonded[start + 1..end].trim();
    if inner.eq_ignore_ascii_case("n/a") {
        return None;
    }
    let negative = inner.starts_with('-');
    let digits: String = inner.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let pct: f64 = digits.parse().ok()?;
    if pct < 1.0 {
        return None;
    }
    let whole = pct.round() as u64;
    let words = say_cardinal(whole.max(1));
    if negative {
        Some(format!("Bonded stake is about {words} percent lower."))
    } else {
        Some(format!("Bonded stake is about {words} percent higher."))
    }
}

fn narrate_moves(moves: &[StakeMove]) -> Option<String> {
    if moves.is_empty() {
        return None;
    }
    let mut named: Vec<(String, Vec<StakeAction>)> = Vec::new();
    let mut anon: Vec<StakeAction> = Vec::new();
    for mv in moves {
        match spoken_validator(&mv.name) {
            Some(name) => {
                if let Some(slot) = named.iter_mut().find(|(n, _)| n == &name) {
                    if !slot.1.contains(&mv.action) {
                        slot.1.push(mv.action);
                    }
                } else {
                    named.push((name, vec![mv.action]));
                }
            }
            None => anon.push(mv.action),
        }
    }
    let mut sentences = Vec::new();
    for (name, actions) in named.iter().take(4) {
        sentences.push(named_sentence(name, actions));
    }
    if named.len() > 4 {
        sentences.push("Other validators also moved.".into());
    }
    if !anon.is_empty() {
        sentences.push(anon_sentence(&anon));
    }
    if sentences.is_empty() {
        None
    } else {
        Some(sentences.join(" "))
    }
}

fn named_sentence(name: &str, actions: &[StakeAction]) -> String {
    let jailed = actions.contains(&StakeAction::Jailed);
    let unbonded = actions.contains(&StakeAction::Unbonded) || actions.contains(&StakeAction::Left);
    let bonded = actions.contains(&StakeAction::Bonded);
    let joined = actions.contains(&StakeAction::Joined);
    let unjailed = actions.contains(&StakeAction::Unjailed);
    let status = actions.contains(&StakeAction::StatusChanged);
    if jailed && unbonded {
        return format!("{name} was jailed and unbonded.");
    }
    if jailed {
        return format!("{name} was jailed.");
    }
    if unjailed && bonded {
        return format!("{name} was unjailed and added stake.");
    }
    if unjailed {
        return format!("{name} was unjailed.");
    }
    if joined {
        return format!("{name} joined the set.");
    }
    if unbonded && !bonded {
        return format!("{name} unbonded.");
    }
    if bonded {
        return format!("{name} added stake.");
    }
    if status {
        return format!("{name} changed status.");
    }
    format!("{name} moved stake.")
}

fn anon_sentence(actions: &[StakeAction]) -> String {
    let n = actions.len();
    let who = if n == 1 {
        "A validator".into()
    } else {
        format!("{n} validators")
    };
    let jailed = actions.iter().any(|a| *a == StakeAction::Jailed);
    let unbonded = actions
        .iter()
        .any(|a| matches!(a, StakeAction::Unbonded | StakeAction::Left));
    let bonded = actions.iter().any(|a| *a == StakeAction::Bonded);
    if jailed && unbonded {
        return format!("{who} jailed and unbonded.");
    }
    if jailed {
        return format!("{who} jailed.");
    }
    if unbonded && !bonded {
        return format!("{who} unbonded.");
    }
    if bonded {
        return format!("{who} added stake.");
    }
    format!("{who} moved stake.")
}

fn written_moves(moves: &[StakeMove]) -> Vec<String> {
    moves
        .iter()
        .take(12)
        .map(|mv| {
            let name = spoken(&mv.name);
            let name = if spoken_validator(&name).is_none() {
                "unnamed validator".to_string()
            } else {
                name
            };
            match mv.action {
                StakeAction::Joined => format!("{name} joined"),
                StakeAction::Left => format!("{name} left"),
                StakeAction::Jailed => format!("{name} jailed"),
                StakeAction::Unjailed => format!("{name} unjailed"),
                StakeAction::Bonded => format!("{name} added stake"),
                StakeAction::Unbonded => format!("{name} unbonded"),
                StakeAction::StatusChanged => format!("{name} changed status"),
            }
        })
        .collect()
}

fn spoken_validator(name: &str) -> Option<String> {
    let name = spoken(name);
    if name.is_empty() || name == "?" || is_bech32(&name) {
        None
    } else {
        Some(name)
    }
}

fn is_bech32(name: &str) -> bool {
    let Some(pos) = name.find('1') else {
        return false;
    };
    (2..21).contains(&pos)
        && (10..91).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn ibc_speech(lines: &[String]) -> String {
    if lines.is_empty() {
        return "No new IBC channel or client.".into();
    }
    let told: Vec<String> = lines.iter().take(3).map(|l| narrate_ibc(l)).collect();
    if lines.len() == 1 {
        return told[0].clone();
    }
    format!("On IBC, {}", told.join(" "))
}

fn narrate_gov(raw: &str) -> String {
    let line = drop_raw_tokens(&spoken(raw));
    let lower = line.to_ascii_lowercase();
    let title = gov_title(&line);
    let id = first_int(&line).map(|n| n.to_string()).unwrap_or_default();
    let verb = if lower.contains("passed") {
        "passed"
    } else if lower.contains("rejected") {
        "was rejected"
    } else if lower.contains("failed") {
        "failed"
    } else if lower.contains("deposit") {
        "is in the deposit period"
    } else if lower.contains("voting") {
        "is in the voting period"
    } else if lower.contains("submitted") {
        "was submitted"
    } else {
        "is on the desk"
    };
    let sentence = if title.to_ascii_lowercase().contains("period") {
        if id.is_empty() {
            format!("{title}.")
        } else {
            format!("Proposal {id}, {title}.")
        }
    } else if id.is_empty() {
        format!("{title}, {verb}.")
    } else {
        format!("Proposal {id}, {title}, {verb}.")
    };
    tidy(&sentence)
}

fn gov_title(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    for marker in ["submitted", "entered voting", "left in voting"] {
        if let Some(i) = lower.find(marker) {
            let rest = line[i + marker.len()..].trim().trim_matches(|c: char| c == '.' || c == ',');
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    line.trim().trim_end_matches('.').to_string()
}

fn narrate_dao(raw: &str) -> String {
    let line = drop_raw_tokens(&spoken(raw));
    let title = line
        .split(['—', '-'])
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.to_ascii_lowercase().contains("dao"))
        .last()
        .unwrap_or(line.trim());
    let title = title.trim().trim_end_matches('.');
    if title.is_empty() {
        "A new proposal opened.".into()
    } else {
        format!("New proposal: {title}.")
    }
}

fn narrate_ibc(raw: &str) -> String {
    let line = spoken(raw);
    let lower = line.to_ascii_lowercase();
    let chain = chain_partner(&line);
    let closed = lower.contains("closed") || lower.contains("close");
    if lower.contains("client") && !lower.contains("channel") {
        return match chain {
            Some(chain) => format!("A new IBC client opened with {chain}."),
            None => "A new IBC client opened.".into(),
        };
    }
    if closed {
        return match chain {
            Some(chain) => format!("A channel with {chain} closed."),
            None => "An IBC channel closed.".into(),
        };
    }
    match chain {
        Some(chain) => format!("A new IBC channel opened with {chain}."),
        None => "A new IBC channel opened.".into(),
    }
}

fn chain_partner(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
        let (name, num) = token.rsplit_once('-')?;
        if name.eq_ignore_ascii_case("channel") || name.eq_ignore_ascii_case("client") {
            return None;
        }
        if name.len() < 2 || !name.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        num.parse::<u16>().ok()?;
        Some(name.to_string())
    })
}

fn narrate_jail(raw: &str) -> String {
    let line = drop_raw_tokens(&spoken(raw));
    let name = line
        .split_whitespace()
        .find(|w| spoken_validator(w).is_some() && !w.eq_ignore_ascii_case("closest") && !w.eq_ignore_ascii_case("jail"))
        .map(|w| w.to_string());
    let left = line
        .split('(')
        .nth(1)
        .and_then(|s| first_int(s))
        .map(|n| format!("{n} blocks left"))
        .unwrap_or_else(|| "some blocks left".into());
    match name {
        Some(name) => format!("{name} is closest to jail, with {left}."),
        None => format!("A validator is closest to jail, with {left}."),
    }
}

fn speak_day(day: &str) -> String {
    let parts: Vec<&str> = day.split('-').collect();
    if parts.len() != 3 {
        return day.to_string();
    }
    let Ok(year) = parts[0].parse::<u64>() else {
        return day.to_string();
    };
    let Ok(month) = parts[1].parse::<u8>() else {
        return day.to_string();
    };
    let Ok(dom) = parts[2].parse::<u8>() else {
        return day.to_string();
    };
    let Some(month) = month_name(month) else {
        return day.to_string();
    };
    if !(1..=31).contains(&dom) {
        return day.to_string();
    }
    format!("{month} {}, {}", ordinal(dom), year_words(year))
}

fn month_name(month: u8) -> Option<&'static str> {
    Some(match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => return None,
    })
}

fn ordinal(n: u8) -> String {
    let word = say_cardinal(n as u64);
    if word.ends_with("one") && !word.ends_with("eleven") {
        return format!("{}first", word.trim_end_matches("one"));
    }
    if word.ends_with("two") && !word.ends_with("twelve") {
        return format!("{}second", word.trim_end_matches("two"));
    }
    if word.ends_with("three") && !word.ends_with("thirteen") {
        return format!("{}third", word.trim_end_matches("three"));
    }
    if word.ends_with("five") {
        return format!("{}fifth", word.trim_end_matches("five"));
    }
    if word.ends_with("eight") {
        return format!("{}eighth", word.trim_end_matches("eight"));
    }
    if word.ends_with("nine") {
        return format!("{}ninth", word.trim_end_matches("nine"));
    }
    if word.ends_with('y') {
        return format!("{}ieth", word.trim_end_matches('y'));
    }
    format!("{word}th")
}

fn year_words(year: u64) -> String {
    if (2000..2010).contains(&year) {
        let rest = year - 2000;
        if rest == 0 {
            return "two thousand".into();
        }
        return format!("two thousand {}", say_cardinal(rest));
    }
    if (2010..2100).contains(&year) {
        return format!("twenty {}", say_cardinal(year - 2000));
    }
    say_cardinal(year)
}

fn drop_raw_tokens(raw: &str) -> String {
    let mut out = String::new();
    for token in raw.split_inclusive(char::is_whitespace) {
        let word = token.trim();
        if word.is_empty() {
            out.push_str(token);
            continue;
        }
        let bare = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if word.contains("://") || is_bech32(bare) {
            continue;
        }
        out.push_str(token);
    }
    out
}

fn first_int(raw: &str) -> Option<u64> {
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut n: u64 = 0;
            while i < chars.len() && chars[i].is_ascii_digit() {
                n = n.saturating_mul(10).saturating_add((chars[i] as u8 - b'0') as u64);
                i += 1;
            }
            return Some(n);
        }
        i += 1;
    }
    None
}

fn tidy(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn say_numbers(input: &str) -> String {
    let input = split_glued_ids(input);
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut out = String::new();
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let (spoken, next) = read_number(&chars, i);
            out.push_str(&spoken);
            i = next;
        } else if chars[i] == '#' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn read_number(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    let mut digits = String::new();
    while i < chars.len() && chars[i].is_ascii_digit() {
        digits.push(chars[i]);
        i += 1;
        if i < chars.len() && chars[i] == ',' {
            let rest = &chars[i + 1..];
            if rest.len() >= 3 && rest[..3].iter().all(|c| c.is_ascii_digit()) {
                i += 1;
                continue;
            }
            break;
        }
    }
    let mut frac = String::new();
    if i < chars.len() && chars[i] == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            frac.push(chars[i]);
            i += 1;
        }
        while frac.ends_with('0') {
            frac.pop();
        }
    }
    let mut percent = false;
    if i < chars.len() && chars[i] == '%' {
        percent = true;
        i += 1;
    }
    let mut unit = "";
    if i < chars.len() && matches!(chars[i], 'h' | 'm') && (i + 1 == chars.len() || !chars[i + 1].is_ascii_alphabetic())
    {
        unit = if chars[i] == 'h' { "hours" } else { "minutes" };
        i += 1;
    }
    let whole: u64 = digits.parse().unwrap_or(0);
    let mut spoken = say_cardinal(whole);
    if !frac.is_empty() {
        spoken.push_str(" point");
        for c in frac.chars() {
            let d = (c as u8 - b'0') as u64;
            spoken.push(' ');
            spoken.push_str(&say_cardinal(d));
        }
    }
    if percent {
        spoken.push_str(" percent");
    }
    if !unit.is_empty() {
        if whole == 1 && frac.is_empty() {
            spoken.push(' ');
            spoken.push_str(unit.trim_end_matches('s'));
        } else {
            spoken.push(' ');
            spoken.push_str(unit);
        }
    }
    (spoken, i)
}

fn split_glued_ids(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let letters = i - start;
            let digits = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let boundary = i == chars.len() || !chars[i].is_ascii_alphanumeric();
            let prev_ok = start == 0 || !chars[start - 1].is_ascii_alphanumeric();
            if letters <= 4 && digits < i && boundary && prev_ok {
                out.extend(chars[start..digits].iter().copied());
                out.push(' ');
                out.extend(chars[digits..i].iter().copied());
            } else {
                out.extend(chars[start..i].iter().copied());
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn say_cardinal(n: u64) -> String {
    const SMALL: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    if n < 20 {
        return SMALL[n as usize].into();
    }
    if n < 100 {
        let tens = TENS[(n / 10) as usize];
        let rest = n % 10;
        if rest == 0 {
            return tens.into();
        }
        return format!("{tens}-{}", SMALL[rest as usize]);
    }
    if n < 1000 {
        let head = format!("{} hundred", SMALL[(n / 100) as usize]);
        let rest = n % 100;
        if rest == 0 {
            return head;
        }
        return format!("{head} {}", say_cardinal(rest));
    }
    for (scale, name) in [(1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")] {
        if n >= scale {
            let head = format!("{} {name}", say_cardinal(n / scale));
            let rest = n % scale;
            if rest == 0 {
                return head;
            }
            return format!("{head} {}", say_cardinal(rest));
        }
    }
    SMALL[0].into()
}

fn spoken(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c == '`' || c == '*' || c == '_' || c == '#' {
            continue;
        }
        if c.is_control() {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn one_paragraph(raw: &str, max: usize) -> String {
    let text = spoken(raw);
    let mut n = 0;
    let mut out = String::new();
    for c in text.chars() {
        if n >= max {
            break;
        }
        out.push(c);
        n += 1;
    }
    out.trim().trim_end_matches('.').to_string()
}

fn first_sentence(raw: &str) -> String {
    let text = spoken(raw);
    let cut = text
        .find(". ")
        .or_else(|| text.find('.'))
        .unwrap_or(text.len());
    one_paragraph(&text[..cut], 180)
}

pub fn append_line(path: &Path, text: &str) -> std::io::Result<()> {
    let line = spoken(text);
    if line.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Read every line and rename the file so a later append starts clean.
pub fn take_lines(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<String> = text.lines().map(spoken).filter(|s| !s.is_empty()).collect();
    let taken = path.with_extension("taken");
    let _ = fs::rename(path, taken);
    lines
}

pub fn take_inbox(dir: &Path) -> (Vec<Contrib>, Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    let done = dir.join("done");
    let _ = fs::create_dir_all(&done);
    let mut notes = Vec::new();
    let mut rejected = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("note")
            .to_string();
        let raw = fs::read_to_string(&path).unwrap_or_default();
        match parse_contrib(&raw) {
            Ok(note) => {
                notes.push(note);
                let _ = fs::rename(&path, done.join(&name));
            }
            Err(_) => rejected.push(name),
        }
    }
    (notes, rejected)
}

/// Host token safe to pass as a single ssh destination. No options, no spaces.
pub fn media_ssh_host(raw: &str) -> Option<&str> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 200 {
        return None;
    }
    if raw.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '@')) {
        Some(raw)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_and_summarize_and_omit() {
        let quote = parse_contrib("mode: quote\nfrom: Nova\ntopic: light client\n---\nThe patch is ready for review.\n").unwrap();
        assert_eq!(quote.mode, ContribMode::Quote);
        let summary = parse_contrib("mode: summarize\nfrom: Linus\ntopic: wasm\n---\nThe cache grew. The second sentence stays on the page only.\n").unwrap();
        assert_eq!(summary.mode, ContribMode::Summarize);
        let omit = parse_contrib("mode: omit\nfrom: Ops\n---\nNumbers for the file.\n").unwrap();
        assert_eq!(omit.mode, ContribMode::Omit);
        assert!(parse_contrib("mode: shout\nfrom: Nova\n---\nno\n").is_err());
    }

    #[test]
    fn desk_names_proposal_dao_stake_and_ibc() {
        let desk = compose_desk(
            "2026-10-28",
            "bonded 1 TERP → 2 TERP (+100.00%)",
            "1 change: jailed alpha",
            &[
                "Gov proposal #63 submitted Recover expired IBC clients. in the voting period".into(),
                "opened transfer/channel-12 toward osmosis-1".into(),
            ],
            &["proposal A11 open: fund the docs sprint".into()],
            &[parse_contrib("mode: quote\nfrom: Nova\ntopic: clients\n---\nShip the recovery.\n").unwrap()],
            &[],
            "",
            &[StakeMove { name: "alpha".into(), action: StakeAction::Jailed }],
        );
        assert!(desk.discord.contains("Recover expired"));
        assert!(desk.discord.contains("fund the docs"));
        assert!(desk.discord.contains("channel-12"));
        assert!(desk.discord.contains("- "));
        assert_eq!(desk.caption, "Terp desk for 2026-10-28");
        assert!(!desk.caption.contains("Recover"));
        assert!(desk.speech.contains("October twenty-eighth"));
        assert!(desk.speech.contains("in their own words"));
        assert!(desk.speech.contains("Foundation DAO"));
        assert!(desk.speech.contains("osmosis"));
        assert!(desk.speech.contains("alpha was jailed") || desk.speech.contains("Alpha was jailed"));
        assert!(!desk.speech.contains("channel-12"));
        assert!(!desk.speech.contains("**"));
        assert!(!desk.speech.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn summarize_keeps_the_first_sentence_only() {
        let note = parse_contrib("mode: summarize\nfrom: Linus\ntopic: wasm\n---\nThe cache grew. Do not read this aloud.\n").unwrap();
        let desk = compose_desk("2026-10-28", "bonded 1 TERP → 1 TERP (+0.00%)", "", &[], &[], &[note], &[], "", &[]);
        assert!(desk.speech.contains("The cache grew"));
        assert!(!desk.speech.contains("Do not read"));
        assert!(desk.discord.contains("Do not read"));
    }

    #[test]
    fn speech_says_words_and_skips_addresses() {
        assert_eq!(say_numbers("ship 1000 units and 63 more"), "ship one thousand units and sixty-three more");
        assert_eq!(say_numbers("up 10.00%"), "up ten percent");
        let desk = compose_desk(
            "2026-10-28",
            "bonded 1000 TERP → 900 TERP (-10.00%)",
            "2 changes: terpval1ywyt +5.00%",
            &[],
            &[],
            &[],
            &[],
            "closest to jail: `terpval1ywyt` 40/100 (60 blocks left)",
            &[
                StakeMove { name: "Alpha".into(), action: StakeAction::Jailed },
                StakeMove { name: "Alpha".into(), action: StakeAction::Unbonded },
                StakeMove { name: "terpval1ywyt".into(), action: StakeAction::Bonded },
            ],
        );
        assert!(desk.speech.contains("Alpha was jailed and unbonded"));
        assert!(desk.speech.contains("A validator added stake"));
        assert!(desk.speech.contains("sixty blocks left"));
        assert!(desk.speech.contains("ten percent lower"));
        assert!(!desk.speech.contains("terpval"));
        assert!(!desk.speech.contains("1000"));
        assert!(!desk.speech.contains("5.00"));
        assert!(desk.discord.contains("terpval1ywyt") || desk.discord.contains("unnamed validator"));
        assert!(!desk.caption.contains('\n'));
    }

    #[test]
    fn media_host_rejects_options() {
        assert_eq!(media_ssh_host("groot-wan"), Some("groot-wan"));
        assert_eq!(media_ssh_host(" edgar@10.0.0.8 "), Some("edgar@10.0.0.8"));
        assert!(media_ssh_host("-oProxyCommand=x").is_none());
        assert!(media_ssh_host("host name").is_none());
    }
}
