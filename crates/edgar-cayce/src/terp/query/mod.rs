//! Read-only chain facts for the gate.
//!
//! `Family` maps onto `gate::QueryFamily` via [`Family::from_tool`] using the
//! design §3.2 tool names (`QueryFamily::tool`). S4 labels (`gov`,
//! `upgrade_version`, `bank_mint`, …) are a different enum.
//!
//! The RLM calls [`gather_facts_text`] when the gate route is `Need::Chain` or
//! `Need::Mixed` and [`chain_facts_enabled`] is true (`EDGAR_CHAIN_FACTS=1`).

mod cache;
mod cite;
mod pool;
pub mod tools;

#[allow(unused_imports)] // re-export for the gate; this binary does not name it yet
pub use cite::Cited;
pub use pool::Pool;

use anyhow::{anyhow, Result};

use cite::citation;
use pool::Budget;
use tools::BondStatus;

/// §3.2 tool families. Map onto `gate::QueryFamily` without renaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    NodeVersion,
    GovLatest,
    GovProposal,
    Validators,
    Staking,
    IbcChannels,
    Upgrade,
    Supply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub family: Family,
    pub lines: Vec<String>,
    pub citations: Vec<String>,
}

impl Facts {
    pub fn text(&self) -> String {
        let mut parts = self.lines.clone();
        parts.extend(self.citations.iter().cloned());
        parts.join("\n")
    }
}

impl std::fmt::Display for Facts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

/// True only when the process asked for live chain facts.
pub fn chain_facts_enabled() -> bool {
    enabled_flag(std::env::var("EDGAR_CHAIN_FACTS").ok().as_deref())
}

pub(crate) fn enabled_flag(v: Option<&str>) -> bool {
    v == Some("1")
}

/// Map a `gate::QueryFamily::tool()` name onto a §3.2 [`Family`].
///
/// Several gate variants collapse onto one family (e.g. all staking reads →
/// `Staking`). Tools this layer does not implement yet (`contract_smart`)
/// return `None`.
pub fn from_tool(tool: &str) -> Option<Family> {
    match tool {
        "node_versions" => Some(Family::NodeVersion),
        "gov_latest" | "gov_active" | "gov_params" => Some(Family::GovLatest),
        "gov_proposal" | "gov_tally" => Some(Family::GovProposal),
        "validators" | "validator" => Some(Family::Validators),
        "staking_params" | "staking_pool" | "slashing_params" => Some(Family::Staking),
        "ibc_channels" | "ibc_clients" | "ibc_connections" => Some(Family::IbcChannels),
        "upgrade_current" | "upgrade_applied" => Some(Family::Upgrade),
        "bank_supply" | "balance" | "mint_inflation" => Some(Family::Supply),
        "contract_smart" => None,
        _ => None,
    }
}

/// Fetch live facts for the gate's tool list. Dedupes by [`Family`], skips
/// unknown tools, and joins each [`Facts::text`] with blank lines. Errors on
/// one family become a short note and do not abort the rest.
pub async fn gather_facts_text(tools: &[&str], question: &str) -> String {
    gather_facts_text_with(pool::global(), tools, question).await
}

pub async fn gather_facts_text_with(pool: &Pool, tools: &[&str], question: &str) -> String {
    let mut ordered: Vec<Family> = Vec::new();
    for tool in tools {
        let Some(family) = from_tool(tool) else {
            continue;
        };
        if ordered.contains(&family) {
            continue;
        }
        ordered.push(family);
    }
    let mut blocks = Vec::new();
    for family in ordered {
        match facts_for_with(pool, family, question).await {
            Ok(facts) => {
                let t = facts.text();
                if !t.is_empty() {
                    blocks.push(t);
                }
            }
            Err(e) => {
                blocks.push(format!(
                    "(chain facts unavailable for {:?}: {e})",
                    family
                ));
            }
        }
    }
    blocks.join("

")
}

pub async fn facts_for(family: Family, question: &str) -> Result<Facts> {
    facts_for_with(pool::global(), family, question).await
}

pub async fn facts_for_with(pool: &Pool, family: Family, question: &str) -> Result<Facts> {
    let mut budget = Budget::new();
    let (lines, citations) = match family {
        Family::NodeVersion => {
            let cited = tools::node_versions_in(pool, &mut budget).await?;
            (tools::format_versions(&cited.data), vec![citation(&cited)])
        }
        Family::GovLatest => {
            if let Some(id) = proposal_id(question) {
                proposal_facts(pool, id, &mut budget).await?
            } else {
                let cited = tools::gov_latest_in(pool, latest_n(question), &mut budget).await?;
                (tools::format_gov(&cited.data), vec![citation(&cited)])
            }
        }
        Family::GovProposal => {
            let id =
                proposal_id(question).ok_or_else(|| anyhow!("no proposal id in the question"))?;
            proposal_facts(pool, id, &mut budget).await?
        }
        Family::Validators => validator_facts(pool, question, &mut budget).await?,
        Family::Staking => staking_facts(pool, question, &mut budget).await?,
        Family::IbcChannels => {
            let hint = counterparty_hint(question);
            let cited = tools::ibc_channels_in(pool, hint.as_deref(), &mut budget).await?;
            (tools::format_channels(&cited.data), vec![citation(&cited)])
        }
        Family::Upgrade => {
            let plan = tools::upgrade_current_in(pool, &mut budget).await?;
            let vers = tools::node_versions_in(pool, &mut budget).await?;
            let mut lines = tools::format_upgrade(&plan.data);
            lines.extend(tools::format_versions(&vers.data));
            (lines, vec![citation(&plan), citation(&vers)])
        }
        Family::Supply => supply_facts(pool, question, &mut budget).await?,
    };
    Ok(Facts {
        family,
        lines,
        citations,
    })
}

async fn proposal_facts(
    pool: &Pool,
    id: u64,
    budget: &mut Budget,
) -> Result<(Vec<String>, Vec<String>)> {
    let cited = tools::gov_proposal_in(pool, id, budget).await?;
    let lines = tools::format_proposal(&cited.data);
    let citations = vec![citation(&cited), cited.data.tally.source.line()];
    Ok((lines, citations))
}

async fn validator_facts(
    pool: &Pool,
    question: &str,
    budget: &mut Budget,
) -> Result<(Vec<String>, Vec<String>)> {
    if let Some(addr) = valoper_from(question)? {
        let cited = tools::validator_in(pool, &addr, budget).await?;
        let lines = vec![tools::format_validator_row(&cited.data)];
        return Ok((lines, vec![citation(&cited)]));
    }
    let status = bond_status_from(question);
    let how_many = is_how_many(question);
    let limit = if how_many { 1 } else { list_limit(question) };
    let cited = tools::validators_in(pool, status, limit, how_many, budget).await?;
    let lines = tools::format_validators(&cited.data, how_many, status);
    Ok((lines, vec![citation(&cited)]))
}

async fn staking_facts(
    pool: &Pool,
    question: &str,
    budget: &mut Budget,
) -> Result<(Vec<String>, Vec<String>)> {
    let pool_c = tools::staking_pool_in(pool, budget).await?;
    let params = tools::staking_params_in(pool, budget).await?;
    let mut lines = tools::format_pool(&pool_c.data);
    lines.extend(tools::format_params(&params.data));
    let mut citations = vec![citation(&pool_c), citation(&params)];
    if let Some(addr) = valoper_from(question)? {
        let one = tools::validator_in(pool, &addr, budget).await?;
        lines.push(tools::format_validator_row(&one.data));
        citations.push(citation(&one));
    }
    Ok((lines, citations))
}

async fn supply_facts(
    pool: &Pool,
    question: &str,
    budget: &mut Budget,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut lines = Vec::new();
    let mut citations = Vec::new();
    for denom in supply_denoms(question) {
        let cited = tools::bank_supply_in(pool, Some(&denom), budget).await?;
        lines.extend(tools::format_supply(&cited.data));
        citations.push(citation(&cited));
    }
    Ok((lines, citations))
}

fn proposal_id(q: &str) -> Option<u64> {
    if let Some(n) = hash_number(q) {
        return Some(n);
    }
    let lower = q.to_ascii_lowercase();
    let bytes = q.as_bytes();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("proposal") {
        let abs = from + rel;
        let mut k = abs + "proposal".len();
        if k < bytes.len() && bytes[k].eq_ignore_ascii_case(&b's') {
            k += 1;
        }
        while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'#' | b':') {
            k += 1;
        }
        let start = k;
        while k < bytes.len() && bytes[k].is_ascii_digit() {
            k += 1;
        }
        if k > start {
            if let Ok(n) = q[start..k].parse::<u64>() {
                if n > 0 {
                    return Some(n);
                }
            }
        }
        from = abs + "proposal".len();
    }
    None
}

fn hash_number(q: &str) -> Option<u64> {
    let bytes = q.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let start = i + 1;
            let mut k = start;
            while k < bytes.len() && bytes[k].is_ascii_digit() {
                k += 1;
            }
            if k > start {
                if let Ok(n) = q[start..k].parse::<u64>() {
                    if n > 0 {
                        return Some(n);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

fn latest_n(q: &str) -> u32 {
    if let Some(n) = small_number_near(q, &["latest", "recent", "last", "top"]) {
        return n.clamp(1, 5);
    }
    if q.to_ascii_lowercase().contains("proposals") {
        5
    } else {
        1
    }
}

fn list_limit(q: &str) -> u32 {
    if let Some(n) = small_number_near(q, &["top", "first", "show"]) {
        return n.clamp(1, 20);
    }
    10
}

fn small_number_near(q: &str, keys: &[&str]) -> Option<u32> {
    let lower = q.to_ascii_lowercase();
    for kw in keys {
        let Some(i) = lower.find(kw) else {
            continue;
        };
        let from = i.saturating_sub(8);
        let to = (i + kw.len() + 8).min(lower.len());
        let mut num = String::new();
        for c in lower[from..to].chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else if !num.is_empty() {
                break;
            }
        }
        if let Ok(n) = num.parse::<u32>() {
            if n > 0 {
                return Some(n);
            }
        }
    }
    None
}

fn is_how_many(q: &str) -> bool {
    let lower = q.to_ascii_lowercase();
    lower.contains("how many") || lower.contains("number of")
}

fn bond_status_from(q: &str) -> BondStatus {
    let lower = q.to_ascii_lowercase();
    if lower.contains("unbonding") {
        BondStatus::Unbonding
    } else if lower.contains("unbonded") {
        BondStatus::Unbonded
    } else {
        BondStatus::Bonded
    }
}

fn valoper_from(q: &str) -> Result<Option<String>> {
    match take_hrp(q, "terpvaloper") {
        Some(addr) if tools::is_bech32(&addr, "terpvaloper") => Ok(Some(addr)),
        Some(_) => anyhow::bail!("address failed bech32 check"),
        None => Ok(None),
    }
}

fn take_hrp(q: &str, hrp: &str) -> Option<String> {
    let marker = format!("{hrp}1");
    let lower = q.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&marker) {
        let abs = from + rel;
        let bytes = lower.as_bytes();
        let mut k = abs;
        while k < bytes.len() && bytes[k].is_ascii_alphanumeric() {
            k += 1;
        }
        let cand = &q[abs..k];
        if cand.bytes().all(|b| !b.is_ascii_uppercase()) {
            return Some(cand.to_string());
        }
        from = abs + marker.len();
    }
    None
}

fn counterparty_hint(q: &str) -> Option<String> {
    if let Some(ch) = find_channel(q) {
        return Some(ch);
    }
    for kw in ["to ", "with ", "from "] {
        if let Some(word) = word_after(q, kw) {
            return Some(word);
        }
    }
    None
}

fn find_channel(q: &str) -> Option<String> {
    let lower = q.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let key = b"channel-";
    let mut i = 0;
    while i + key.len() < bytes.len() {
        if bytes[i..].starts_with(key) {
            let start = i;
            i += key.len();
            let num = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > num {
                return Some(lower[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

fn word_after(q: &str, kw: &str) -> Option<String> {
    let lower = q.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(kw) {
        let abs = from + rel;
        let boundary = abs == 0 || !lower.as_bytes()[abs - 1].is_ascii_alphanumeric();
        if boundary {
            let rest = q[abs + kw.len()..].trim_start();
            let word: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            let skip = matches!(
                word.to_ascii_lowercase().as_str(),
                "the" | "terp" | "ibc" | "channel" | "a" | "an"
            );
            if word.len() >= 3 && !skip {
                return Some(word);
            }
        }
        from = abs + kw.len();
    }
    None
}

fn supply_denoms(q: &str) -> Vec<String> {
    if mentions_word(q, "uthiol") {
        return vec!["uthiol".into()];
    }
    if mentions_word(q, "uterp") {
        return vec!["uterp".into()];
    }
    if mentions_word(q, "thiol") {
        return vec!["uthiol".into()];
    }
    if mentions_word(q, "terp") {
        return vec!["uterp".into()];
    }
    vec!["uterp".into(), "uthiol".into()]
}

fn mentions_word(q: &str, word: &str) -> bool {
    let lower = q.to_ascii_lowercase();
    let w = word.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let wb = w.as_bytes();
    if wb.is_empty() || bytes.len() < wb.len() {
        return false;
    }
    let mut i = 0;
    while i + wb.len() <= bytes.len() {
        if &bytes[i..i + wb.len()] == wb {
            let before = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after = i + wb.len() == bytes.len() || !bytes[i + wb.len()].is_ascii_alphanumeric();
            if before && after {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terp::query::pool::harness::{route, serve};
    use crate::terp::query::pool::NODE_INFO_PATH;
    use std::collections::HashMap;

    const VALOPER: &str = "terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllk";

    #[test]
    fn flag_is_only_the_digit_one() {
        assert!(enabled_flag(Some("1")));
        assert!(!enabled_flag(Some("true")));
        assert!(!enabled_flag(Some("")));
        assert!(!enabled_flag(None));
    }

    #[test]
    fn question_args() {
        assert_eq!(proposal_id("what is proposal #62 about"), Some(62));
        assert_eq!(proposal_id("did proposal 62 pass"), Some(62));
        assert_eq!(proposal_id("most recent governance proposal"), None);
        assert_eq!(proposal_id("proposal #0"), None);
        assert_eq!(latest_n("most recent governance proposal"), 1);
        assert_eq!(latest_n("top 3 proposals"), 3);
        assert_eq!(latest_n("latest proposals"), 5);
        assert_eq!(
            valoper_from(&format!("is {VALOPER} jailed"))
                .unwrap()
                .as_deref(),
            Some(VALOPER)
        );
        assert!(
            valoper_from("is terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllx jailed").is_err()
        );
        assert_eq!(valoper_from("how many active validators").unwrap(), None);
        assert!(is_how_many("how many active validators"));
        assert_eq!(
            bond_status_from("unbonded validators"),
            BondStatus::Unbonded
        );
        assert_eq!(
            counterparty_hint("what's the IBC channel from terp to osmosis").as_deref(),
            Some("osmosis")
        );
        assert_eq!(
            counterparty_hint("channel-11 status").as_deref(),
            Some("channel-11")
        );
        assert_eq!(
            supply_denoms("total supply of TERP"),
            vec!["uterp".to_string()]
        );
        assert_eq!(supply_denoms("supply of THIOL"), vec!["uthiol".to_string()]);
        assert_eq!(
            supply_denoms("total supply"),
            vec!["uterp".to_string(), "uthiol".to_string()]
        );
    }

    #[test]
    fn seam_is_linked() {
        let _ = std::any::type_name::<Cited<u8>>();
        let _ = chain_facts_enabled;
        let _ = facts_for;
        let _ = tools::node_versions;
        let _ = tools::gov_latest;
        let _ = tools::gov_proposal;
        let _ = tools::validators;
        let _ = tools::validator;
        let _ = tools::staking_pool;
        let _ = tools::staking_params;
        let _ = tools::ibc_channels;
        let _ = tools::upgrade_current;
        let _ = tools::bank_supply;
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/lcd/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn facts_for_cites_fixture_reads() {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, fixture("node_info.json"), Some(23889904)),
        );
        routes.insert(
            "/cosmos/gov/v1/proposals?pagination.limit=1&pagination.reverse=true".into(),
            route(200, fixture("gov_latest.json"), Some(23889904)),
        );
        routes.insert(
            "/cosmos/gov/v1/proposals/62".into(),
            route(200, fixture("proposal_62.json"), Some(23889904)),
        );
        routes.insert(
            "/cosmos/gov/v1/proposals/62/tally".into(),
            route(200, fixture("tally_62.json"), Some(23889905)),
        );
        routes.insert(
            "/cosmos/upgrade/v1beta1/current_plan".into(),
            route(200, fixture("current_plan.json"), Some(23889904)),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);

        let ibc = facts_for_with(
            &pool,
            Family::NodeVersion,
            "what version of ibc is terp network on",
        )
        .await
        .unwrap();
        let ibc_text = ibc.text();
        assert!(ibc_text.contains("ibc-go v11.2.0"), "{ibc_text}");
        assert!(ibc_text.contains("— source: morocco-1 LCD "), "{ibc_text}");
        assert!(ibc_text.contains("node_info"), "{ibc_text}");
        assert!(ibc_text.contains("height 23889904"), "{ibc_text}");
        assert!(ibc_text.contains("GMT"), "{ibc_text}");
        assert!(!ibc_text.contains("cached"), "{ibc_text}");

        let gov = facts_for_with(&pool, Family::GovLatest, "most recent governance proposal")
            .await
            .unwrap();
        let gov_text = gov.text();
        assert!(gov_text.contains("#62"), "{gov_text}");
        assert!(gov_text.contains("PASSED"), "{gov_text}");
        assert!(gov_text.contains("pagination.reverse=true"), "{gov_text}");

        let prop = facts_for_with(
            &pool,
            Family::GovProposal,
            "what is proposal #62 about and did it pass",
        )
        .await
        .unwrap();
        let prop_text = prop.text();
        assert!(prop_text.contains("Software upgrade v6.3"), "{prop_text}");
        assert!(prop_text.contains("3773094342840"), "{prop_text}");
        assert!(
            prop_text.contains("/cosmos/gov/v1/proposals/62/tally"),
            "{prop_text}"
        );

        let up = facts_for_with(&pool, Family::Upgrade, "is there an upcoming chain upgrade")
            .await
            .unwrap();
        let up_text = up.text();
        assert!(
            up_text.contains("plan v6.3 at height 23893261"),
            "{up_text}"
        );
        assert!(up_text.contains("app 6.2.1"), "{up_text}");
        assert!(up_text.contains("cached"), "{up_text}");
    }

    #[test]
    fn from_tool_maps_gate_names() {
        assert_eq!(from_tool("node_versions"), Some(Family::NodeVersion));
        assert_eq!(from_tool("gov_latest"), Some(Family::GovLatest));
        assert_eq!(from_tool("gov_active"), Some(Family::GovLatest));
        assert_eq!(from_tool("gov_proposal"), Some(Family::GovProposal));
        assert_eq!(from_tool("gov_tally"), Some(Family::GovProposal));
        assert_eq!(from_tool("validators"), Some(Family::Validators));
        assert_eq!(from_tool("validator"), Some(Family::Validators));
        assert_eq!(from_tool("staking_pool"), Some(Family::Staking));
        assert_eq!(from_tool("slashing_params"), Some(Family::Staking));
        assert_eq!(from_tool("ibc_channels"), Some(Family::IbcChannels));
        assert_eq!(from_tool("ibc_clients"), Some(Family::IbcChannels));
        assert_eq!(from_tool("upgrade_current"), Some(Family::Upgrade));
        assert_eq!(from_tool("upgrade_applied"), Some(Family::Upgrade));
        assert_eq!(from_tool("bank_supply"), Some(Family::Supply));
        assert_eq!(from_tool("mint_inflation"), Some(Family::Supply));
        assert_eq!(from_tool("contract_smart"), None);
        assert_eq!(from_tool("not_a_tool"), None);
    }

        #[tokio::test]
    async fn gather_facts_text_dedupes_families() {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, fixture("node_info.json"), Some(23889904)),
        );
        routes.insert(
            "/cosmos/upgrade/v1beta1/current_plan".into(),
            route(200, fixture("current_plan.json"), Some(23889904)),
        );
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        let text = gather_facts_text_with(
            &pool,
            &["node_versions", "upgrade_current", "node_versions"],
            "what version of ibc and is there an upgrade",
        )
        .await;
        assert!(text.contains("ibc-go v11.2.0"), "{text}");
        assert!(text.contains("plan v6.3"), "{text}");
    }

    /// Manual check against the house LCD. Not part of `cargo test`.
    #[tokio::test]
    #[ignore]
    async fn live_facts_for_local_node() {
        let ibc = facts_for(
            Family::NodeVersion,
            "what version of ibc is terp network on",
        )
        .await
        .expect("ibc");
        println!("IBC\n{ibc}\n");
        assert!(ibc.text().contains("v11.2.0"), "{ibc}");

        let gov = facts_for(Family::GovLatest, "most recent governance proposal")
            .await
            .expect("gov");
        println!("GOV\n{gov}\n");
        assert!(gov.text().contains("62"), "{gov}");

        let up = facts_for(Family::Upgrade, "is there an upcoming chain upgrade")
            .await
            .expect("upgrade");
        println!("UP\n{up}\n");
        assert!(up.text().contains("v6.3"), "{up}");
    }
}
