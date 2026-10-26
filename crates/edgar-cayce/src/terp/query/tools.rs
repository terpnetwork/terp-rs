//! Typed read-only LCD queries. Paths are built here. Callers never pass a free-form path.
//!
//! Parsers reuse `chain_watch` for proposals, channels, validators, and the bonded pool.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::cite::{Cited, Source};
use super::pool::{Budget, Pool, NODE_INFO_PATH};
use crate::terp::chain_watch;

const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32_GEN: [u32; 5] = [
    0x3b6a_57b2,
    0x2650_8e6d,
    0x1ea1_19fa,
    0x3d42_33dd,
    0x2a14_62b3,
];
const MAX_CHANNEL_LOOKUPS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BondStatus {
    Bonded,
    Unbonding,
    Unbonded,
    Unspecified,
}

impl BondStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bonded => "BOND_STATUS_BONDED",
            Self::Unbonding => "BOND_STATUS_UNBONDING",
            Self::Unbonded => "BOND_STATUS_UNBONDED",
            Self::Unspecified => "BOND_STATUS_UNSPECIFIED",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bonded => "bonded",
            Self::Unbonding => "unbonding",
            Self::Unbonded => "unbonded",
            Self::Unspecified => "unspecified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeVersions {
    pub app: String,
    pub sdk: String,
    pub ibc_go: String,
    pub wasmd: String,
    pub wasm_08: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally {
    pub yes: String,
    pub no: String,
    pub abstain: String,
    pub veto: String,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovProposal {
    pub id: String,
    pub title: String,
    pub status: String,
    pub summary: String,
    pub tally: Tally,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorRow {
    pub operator: String,
    pub moniker: String,
    pub status: String,
    pub jailed: bool,
    pub tokens: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorSet {
    pub rows: Vec<ValidatorRow>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingPool {
    pub bonded_tokens: String,
    pub not_bonded_tokens: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakingParams {
    pub unbonding_time: String,
    pub max_validators: String,
    pub bond_denom: String,
    pub min_commission_rate: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IbcChannel {
    pub port_id: String,
    pub channel_id: String,
    pub counterparty: String,
    pub state: String,
    pub chain_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradePlan {
    pub name: String,
    pub height: String,
    pub info: String,
    pub scheduled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coin {
    pub denom: String,
    pub amount: String,
}

pub async fn node_versions() -> Result<Cited<NodeVersions>> {
    let mut budget = Budget::new();
    node_versions_in(super::pool::global(), &mut budget).await
}

pub async fn gov_latest(n: u32) -> Result<Cited<Vec<GovItem>>> {
    let mut budget = Budget::new();
    gov_latest_in(super::pool::global(), n, &mut budget).await
}

pub async fn gov_proposal(id: u64) -> Result<Cited<GovProposal>> {
    let mut budget = Budget::new();
    gov_proposal_in(super::pool::global(), id, &mut budget).await
}

pub async fn validators(status: BondStatus, limit: u32) -> Result<Cited<ValidatorSet>> {
    let mut budget = Budget::new();
    validators_in(super::pool::global(), status, limit, false, &mut budget).await
}

pub async fn validator(operator: &str) -> Result<Cited<ValidatorRow>> {
    let mut budget = Budget::new();
    validator_in(super::pool::global(), operator, &mut budget).await
}

pub async fn staking_pool() -> Result<Cited<StakingPool>> {
    let mut budget = Budget::new();
    staking_pool_in(super::pool::global(), &mut budget).await
}

pub async fn staking_params() -> Result<Cited<StakingParams>> {
    let mut budget = Budget::new();
    staking_params_in(super::pool::global(), &mut budget).await
}

pub async fn ibc_channels(counterparty: Option<&str>) -> Result<Cited<Vec<IbcChannel>>> {
    let mut budget = Budget::new();
    ibc_channels_in(super::pool::global(), counterparty, &mut budget).await
}

pub async fn upgrade_current() -> Result<Cited<UpgradePlan>> {
    let mut budget = Budget::new();
    upgrade_current_in(super::pool::global(), &mut budget).await
}

pub async fn bank_supply(denom: Option<&str>) -> Result<Cited<Vec<Coin>>> {
    let mut budget = Budget::new();
    bank_supply_in(super::pool::global(), denom, &mut budget).await
}

pub(crate) async fn node_versions_in(
    pool: &Pool,
    budget: &mut Budget,
) -> Result<Cited<NodeVersions>> {
    let cited = pool.get_json(NODE_INFO_PATH, budget).await?;
    let versions = parse_node_versions(&cited.data)?;
    Ok(cited.map(|_| versions))
}

pub(crate) async fn gov_latest_in(
    pool: &Pool,
    n: u32,
    budget: &mut Budget,
) -> Result<Cited<Vec<GovItem>>> {
    let n = check_latest_n(n)?;
    let cited = pool.get_json(&gov_latest_path(n), budget).await?;
    let items = parse_gov_items(&cited.data);
    Ok(cited.map(|_| items))
}

pub(crate) async fn gov_proposal_in(
    pool: &Pool,
    id: u64,
    budget: &mut Budget,
) -> Result<Cited<GovProposal>> {
    if id == 0 {
        anyhow::bail!("proposal id must be >= 1");
    }
    let path = format!("/cosmos/gov/v1/proposals/{id}");
    let tally_path = format!("/cosmos/gov/v1/proposals/{id}/tally");
    let prop = pool.get_json(&path, budget).await?;
    let tally = pool.get_json(&tally_path, budget).await?;
    let item = parse_gov_items(&prop.data)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("proposal {id} not found"))?;
    let counts = parse_tally_counts(&tally.data);
    let data = GovProposal {
        id: item.id,
        title: item.title,
        status: item.status,
        summary: item.summary,
        tally: Tally {
            yes: counts.0,
            no: counts.1,
            abstain: counts.2,
            veto: counts.3,
            source: tally.source(),
        },
    };
    Ok(prop.map(|_| data))
}

pub(crate) async fn validators_in(
    pool: &Pool,
    status: BondStatus,
    limit: u32,
    count_total: bool,
    budget: &mut Budget,
) -> Result<Cited<ValidatorSet>> {
    let limit = check_limit(limit)?;
    let cited = pool
        .get_json(&validators_path(status, limit, count_total), budget)
        .await?;
    let set = ValidatorSet {
        total: parse_total(&cited.data),
        rows: parse_validator_rows(&cited.data),
    };
    Ok(cited.map(|_| set))
}

pub(crate) async fn validator_in(
    pool: &Pool,
    operator: &str,
    budget: &mut Budget,
) -> Result<Cited<ValidatorRow>> {
    require_bech32(operator, "terpvaloper")?;
    let cited = pool.get_json(&validator_path(operator), budget).await?;
    let row = parse_validator_rows(&cited.data)
        .into_iter()
        .find(|r| r.operator == operator)
        .ok_or_else(|| anyhow!("validator not found"))?;
    Ok(cited.map(|_| row))
}

pub(crate) async fn staking_pool_in(
    pool: &Pool,
    budget: &mut Budget,
) -> Result<Cited<StakingPool>> {
    let cited = pool
        .get_json("/cosmos/staking/v1beta1/pool", budget)
        .await?;
    let parsed = parse_pool(&cited.data);
    Ok(cited.map(|_| parsed))
}

pub(crate) async fn staking_params_in(
    pool: &Pool,
    budget: &mut Budget,
) -> Result<Cited<StakingParams>> {
    let cited = pool
        .get_json("/cosmos/staking/v1beta1/params", budget)
        .await?;
    let parsed = parse_staking_params(&cited.data);
    Ok(cited.map(|_| parsed))
}

pub(crate) async fn ibc_channels_in(
    pool: &Pool,
    counterparty: Option<&str>,
    budget: &mut Budget,
) -> Result<Cited<Vec<IbcChannel>>> {
    let cited = pool
        .get_json("/ibc/core/channel/v1/channels?pagination.limit=100", budget)
        .await?;
    let mut rows = narrow(parse_channel_rows(&cited.data), counterparty);
    let n = rows.len().min(MAX_CHANNEL_LOOKUPS);
    for row in rows.iter_mut().take(n) {
        if budget.exhausted() {
            break;
        }
        if row.port_id.is_empty() || row.channel_id.is_empty() {
            continue;
        }
        let path = client_state_path(&row.channel_id, &row.port_id);
        match pool.get_json(&path, budget).await {
            Ok(state) => row.chain_id = parse_client_chain_id(&state.data),
            Err(_) if budget.exhausted() => break,
            Err(_) => {}
        }
    }
    if let Some(needle) = counterparty {
        rows.retain(|r| matches(r, needle));
    }
    Ok(cited.map(|_| rows))
}

pub(crate) async fn upgrade_current_in(
    pool: &Pool,
    budget: &mut Budget,
) -> Result<Cited<UpgradePlan>> {
    let cited = pool
        .get_json("/cosmos/upgrade/v1beta1/current_plan", budget)
        .await?;
    let plan = parse_upgrade(&cited.data);
    Ok(cited.map(|_| plan))
}

pub(crate) async fn bank_supply_in(
    pool: &Pool,
    denom: Option<&str>,
    budget: &mut Budget,
) -> Result<Cited<Vec<Coin>>> {
    let cited = pool.get_json(&supply_path(denom)?, budget).await?;
    let coins = parse_supply(&cited.data);
    Ok(cited.map(|_| coins))
}

pub(crate) fn format_versions(v: &NodeVersions) -> Vec<String> {
    vec![
        field("app", &v.app),
        field("sdk", &v.sdk),
        field("ibc-go", &v.ibc_go),
        field("wasmd", &v.wasmd),
        field("08-wasm", &v.wasm_08),
    ]
}

pub(crate) fn format_gov(items: &[GovItem]) -> Vec<String> {
    if items.is_empty() {
        return vec!["no proposals in this window".into()];
    }
    let mut lines = Vec::new();
    for (i, p) in items.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(gov_lines(&p.id, &p.title, &p.status, &p.summary));
    }
    lines
}

pub(crate) fn format_proposal(p: &GovProposal) -> Vec<String> {
    let mut lines = gov_lines(&p.id, &p.title, &p.status, &p.summary);
    lines.push(format!(
        "tally yes {} · no {} · abstain {} · veto {}",
        p.tally.yes, p.tally.no, p.tally.abstain, p.tally.veto
    ));
    lines
}

pub(crate) fn format_validators(
    set: &ValidatorSet,
    how_many: bool,
    status: BondStatus,
) -> Vec<String> {
    if how_many {
        let n = set.total.unwrap_or(set.rows.len() as u64);
        return vec![format!("{} validators: {n}", status.label())];
    }
    if set.rows.is_empty() {
        return vec![format!("no {} validators", status.label())];
    }
    set.rows.iter().map(format_validator_row).collect()
}

pub(crate) fn format_validator_row(v: &ValidatorRow) -> String {
    let jail = if v.jailed { " jailed" } else { "" };
    let status = v.status.strip_prefix("BOND_STATUS_").unwrap_or(&v.status);
    format!(
        "{} `{}` {status}{jail} {}",
        v.moniker,
        v.operator,
        chain_watch::format_uthiol(&v.tokens)
    )
}

pub(crate) fn format_pool(p: &StakingPool) -> Vec<String> {
    vec![
        format!("bonded {}", chain_watch::format_uthiol(&p.bonded_tokens)),
        format!(
            "not_bonded {}",
            chain_watch::format_uthiol(&p.not_bonded_tokens)
        ),
    ]
}

pub(crate) fn format_params(p: &StakingParams) -> Vec<String> {
    vec![
        format!("unbonding_time {}", p.unbonding_time),
        format!("max_validators {}", p.max_validators),
        format!("bond_denom {}", p.bond_denom),
        format!("min_commission_rate {}", p.min_commission_rate),
    ]
}

pub(crate) fn format_channels(rows: &[IbcChannel]) -> Vec<String> {
    if rows.is_empty() {
        return vec!["no matching IBC channels".into()];
    }
    rows.iter()
        .map(|r| {
            let chain = r.chain_id.as_deref().unwrap_or("chain_id unread");
            format!(
                "{}/{} ↔ {} {} {chain}",
                r.port_id, r.channel_id, r.counterparty, r.state
            )
        })
        .collect()
}

pub(crate) fn format_upgrade(p: &UpgradePlan) -> Vec<String> {
    if !p.scheduled {
        vec!["no current upgrade plan".into()]
    } else {
        vec![format!("plan {} at height {}", p.name, p.height)]
    }
}

pub(crate) fn format_supply(coins: &[Coin]) -> Vec<String> {
    if coins.is_empty() {
        return vec!["no supply rows".into()];
    }
    coins.iter().map(format_coin).collect()
}

pub(crate) fn require_bech32(addr: &str, hrp: &str) -> Result<()> {
    if is_bech32(addr, hrp) {
        Ok(())
    } else {
        anyhow::bail!("address failed bech32 check")
    }
}

pub(crate) fn is_bech32(addr: &str, hrp: &str) -> bool {
    if addr.len() < 8 || addr.len() > 90 {
        return false;
    }
    if addr
        .bytes()
        .any(|b| !b.is_ascii() || b.is_ascii_uppercase())
    {
        return false;
    }
    let Some((got, data)) = addr.rsplit_once('1') else {
        return false;
    };
    if got != hrp || data.len() < 6 {
        return false;
    }
    let mut values = Vec::with_capacity(hrp.len() * 2 + 1 + data.len());
    values.extend(hrp.bytes().map(|b| b >> 5));
    values.push(0);
    values.extend(hrp.bytes().map(|b| b & 31));
    for c in data.bytes() {
        let Some(pos) = CHARSET.iter().position(|&x| x == c) else {
            return false;
        };
        values.push(pos as u8);
    }
    polymod(&values) == 1
}

fn field(name: &str, value: &str) -> String {
    if value.is_empty() {
        format!("{name} unknown")
    } else {
        format!("{name} {value}")
    }
}

fn gov_lines(id: &str, title: &str, status: &str, summary: &str) -> Vec<String> {
    let mut lines = vec![format!("#{id} {title}"), format!("status {status}")];
    if !summary.is_empty() {
        lines.push(summary.to_string());
    }
    lines.push(chain_watch::proposal_url(&chain_watch::explorer_base(), id));
    lines
}

fn format_coin(c: &Coin) -> String {
    if c.denom == "uterp" || c.denom == "uthiol" {
        format!(
            "{} {} ({})",
            c.denom,
            c.amount,
            chain_watch::format_uthiol(&c.amount)
        )
    } else {
        format!("{} {}", c.denom, c.amount)
    }
}

fn parse_node_versions(v: &Value) -> Result<NodeVersions> {
    let app = &v["application_version"];
    let deps = app["build_deps"].as_array();
    let versions = NodeVersions {
        app: scalar(&app["version"]),
        sdk: scalar(&app["cosmos_sdk_version"]),
        ibc_go: dep_version(deps, |p| {
            p == "github.com/cosmos/ibc-go/v11"
                || (p.contains("/ibc-go/") && !p.contains("08-wasm") && !p.contains("/modules/"))
        }),
        wasmd: dep_version(deps, |p| p.rsplit('/').next() == Some("wasmd")),
        wasm_08: dep_version(deps, |p| p.contains("08-wasm")),
    };
    if versions.app.is_empty() && versions.sdk.is_empty() && versions.ibc_go.is_empty() {
        anyhow::bail!("node_info missing version fields");
    }
    Ok(versions)
}

fn dep_version(deps: Option<&Vec<Value>>, pred: impl Fn(&str) -> bool) -> String {
    let Some(deps) = deps else {
        return String::new();
    };
    for dep in deps {
        let path = dep["path"].as_str().unwrap_or("");
        if pred(path) {
            return scalar(&dep["version"]);
        }
    }
    String::new()
}

fn parse_gov_items(v: &Value) -> Vec<GovItem> {
    chain_watch::parse_proposals(&as_proposal_list(v))
        .into_iter()
        .map(|p| GovItem {
            id: p.id,
            title: p.title,
            status: p.status,
            summary: p.summary,
        })
        .collect()
}

fn as_proposal_list(v: &Value) -> Value {
    if v.get("proposals").is_some() {
        return v.clone();
    }
    if let Some(p) = v.get("proposal") {
        return json!({"proposals": [p.clone()]});
    }
    v.clone()
}

fn parse_tally_counts(v: &Value) -> (String, String, String, String) {
    let t = &v["tally"];
    (
        scalar(&t["yes_count"]),
        scalar(&t["no_count"]),
        scalar(&t["abstain_count"]),
        scalar(&t["no_with_veto_count"]),
    )
}

fn parse_validator_rows(v: &Value) -> Vec<ValidatorRow> {
    chain_watch::parse_validators(&as_validator_list(v))
        .into_iter()
        .map(|(operator, snap)| ValidatorRow {
            operator,
            moniker: snap.moniker,
            status: snap.status,
            jailed: snap.jailed,
            tokens: snap.tokens,
        })
        .collect()
}

fn as_validator_list(v: &Value) -> Value {
    if v.get("validators").is_some() {
        return v.clone();
    }
    if let Some(one) = v.get("validator") {
        return json!({"validators": [one.clone()]});
    }
    v.clone()
}

fn parse_total(v: &Value) -> Option<u64> {
    let t = &v["pagination"]["total"];
    t.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| t.as_u64())
}

fn parse_pool(v: &Value) -> StakingPool {
    StakingPool {
        bonded_tokens: chain_watch::parse_pool_bonded(v),
        not_bonded_tokens: scalar(&v["pool"]["not_bonded_tokens"]),
    }
}

fn parse_staking_params(v: &Value) -> StakingParams {
    let p = &v["params"];
    StakingParams {
        unbonding_time: scalar(&p["unbonding_time"]),
        max_validators: scalar(&p["max_validators"]),
        bond_denom: scalar(&p["bond_denom"]),
        min_commission_rate: scalar(&p["min_commission_rate"]),
    }
}

fn parse_channel_rows(v: &Value) -> Vec<IbcChannel> {
    let notes = chain_watch::parse_channels(v);
    let raw = v["channels"].as_array();
    notes
        .into_iter()
        .enumerate()
        .map(|(i, note)| {
            let (port_id, channel_id) = raw
                .and_then(|arr| arr.get(i))
                .map(|ch| (scalar(&ch["port_id"]), scalar(&ch["channel_id"])))
                .unwrap_or_else(|| split_port_channel(&note.key));
            IbcChannel {
                port_id,
                channel_id,
                counterparty: note.counterparty,
                state: note.state,
                chain_id: None,
            }
        })
        .collect()
}

fn split_port_channel(key: &str) -> (String, String) {
    match key.split_once('/') {
        Some((p, c)) => (p.to_string(), c.to_string()),
        None => (String::new(), key.to_string()),
    }
}

fn parse_client_chain_id(v: &Value) -> Option<String> {
    let s = v
        .pointer("/identified_client_state/client_state/chain_id")
        .or_else(|| v.pointer("/client_state/chain_id"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())?;
    Some(s.to_string())
}

fn parse_upgrade(v: &Value) -> UpgradePlan {
    let plan = &v["plan"];
    if !plan.is_object() {
        return UpgradePlan {
            name: String::new(),
            height: String::new(),
            info: String::new(),
            scheduled: false,
        };
    }
    let name = scalar(&plan["name"]);
    UpgradePlan {
        scheduled: !name.is_empty(),
        height: scalar(&plan["height"]),
        info: scalar(&plan["info"]),
        name,
    }
}

fn parse_supply(v: &Value) -> Vec<Coin> {
    if let Some(amount) = v.get("amount").filter(|a| a.is_object()) {
        let denom = scalar(&amount["denom"]);
        if denom.is_empty() {
            return Vec::new();
        }
        return vec![Coin {
            denom,
            amount: scalar(&amount["amount"]),
        }];
    }
    v["supply"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let denom = scalar(&c["denom"]);
                    if denom.is_empty() {
                        None
                    } else {
                        Some(Coin {
                            denom,
                            amount: scalar(&c["amount"]),
                        })
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn narrow(rows: Vec<IbcChannel>, needle: Option<&str>) -> Vec<IbcChannel> {
    let Some(needle) = needle else {
        return rows;
    };
    let hits: Vec<_> = rows
        .iter()
        .filter(|r| matches(r, needle))
        .cloned()
        .collect();
    if hits.is_empty() {
        rows
    } else {
        hits
    }
}

fn matches(row: &IbcChannel, needle: &str) -> bool {
    let n = needle.to_ascii_lowercase();
    if row.channel_id.eq_ignore_ascii_case(needle) || row.port_id.eq_ignore_ascii_case(needle) {
        return true;
    }
    if format!("{}/{}", row.port_id, row.channel_id).eq_ignore_ascii_case(needle) {
        return true;
    }
    if n.len() >= 3 && row.counterparty.to_ascii_lowercase().contains(&n) {
        return true;
    }
    row.chain_id
        .as_ref()
        .is_some_and(|c| n.len() >= 3 && c.to_ascii_lowercase().contains(&n))
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn gov_latest_path(n: u32) -> String {
    format!("/cosmos/gov/v1/proposals?pagination.limit={n}&pagination.reverse=true")
}

fn validators_path(status: BondStatus, limit: u32, count_total: bool) -> String {
    let mut path = format!(
        "/cosmos/staking/v1beta1/validators?status={}&pagination.limit={limit}",
        status.as_str()
    );
    if count_total {
        path.push_str("&pagination.count_total=true");
    }
    path
}

fn validator_path(operator: &str) -> String {
    format!("/cosmos/staking/v1beta1/validators/{operator}")
}

fn client_state_path(channel: &str, port: &str) -> String {
    format!("/ibc/core/channel/v1/channels/{channel}/ports/{port}/client_state")
}

fn supply_path(denom: Option<&str>) -> Result<String> {
    match denom {
        None => Ok("/cosmos/bank/v1beta1/supply?pagination.limit=20".into()),
        Some(d) => {
            check_denom(d)?;
            Ok(format!("/cosmos/bank/v1beta1/supply/by_denom?denom={d}"))
        }
    }
}

fn check_latest_n(n: u32) -> Result<u32> {
    if (1..=5).contains(&n) {
        Ok(n)
    } else {
        anyhow::bail!("gov_latest n must be 1..=5")
    }
}

fn check_limit(limit: u32) -> Result<u32> {
    if (1..=100).contains(&limit) {
        Ok(limit)
    } else {
        anyhow::bail!("limit must be 1..=100")
    }
}

fn check_denom(d: &str) -> Result<()> {
    if d.is_empty()
        || d.len() > 128
        || d.contains("..")
        || !d
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
    {
        anyhow::bail!("bad denom");
    }
    Ok(())
}

fn polymod(values: &[u8]) -> u32 {
    let mut chk = 1u32;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ u32::from(*v);
        for (i, g) in BECH32_GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terp::node;
    use crate::terp::query::pool::harness::{route, serve};
    use std::collections::HashMap;

    fn fixture_text(name: &str) -> String {
        let path = format!("{}/tests/fixtures/lcd/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn fixture_json(name: &str) -> Value {
        serde_json::from_str(&fixture_text(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn mini_node() -> String {
        r#"{"default_node_info":{"network":"morocco-1"},"application_version":{"version":"6.2.1","cosmos_sdk_version":"v0.55.0","build_deps":[]}}"#
            .into()
    }

    async fn fixture_pool(
        pairs: Vec<(String, String)>,
    ) -> (Pool, crate::terp::query::pool::harness::Server) {
        let mut routes = HashMap::new();
        routes.insert(
            NODE_INFO_PATH.to_string(),
            route(200, mini_node(), Some(23_889_904)),
        );
        for (path, body) in pairs {
            routes.insert(path, route(200, body, Some(23_889_904)));
        }
        let server = serve(routes).await;
        let pool = Pool::from_bases(vec![server.url.clone()]);
        (pool, server)
    }

    #[test]
    fn node_versions_fixture() {
        let v = parse_node_versions(&fixture_json("node_info.json")).unwrap();
        assert_eq!(v.app, "6.2.1");
        assert_eq!(v.sdk, "v0.55.0");
        assert_eq!(v.ibc_go, "v11.2.0");
        assert_eq!(v.wasmd, "v0.70.3");
        assert_eq!(v.wasm_08, "v11.1.0");
        let lines = format_versions(&v);
        assert!(lines.iter().any(|l| l == "ibc-go v11.2.0"));
    }

    #[test]
    fn gov_latest_fixture() {
        let items = parse_gov_items(&fixture_json("gov_latest.json"));
        assert_eq!(items[0].id, "62");
        assert_eq!(items[0].status, "PASSED");
        assert!(items[0].title.contains("Software upgrade v6.3"));
    }

    #[test]
    fn gov_proposal_fixture() {
        let items = parse_gov_items(&fixture_json("proposal_62.json"));
        assert_eq!(items[0].id, "62");
        assert_eq!(items[0].status, "PASSED");
        assert!(items[0].summary.contains("23893261"));
        let tally = parse_tally_counts(&fixture_json("tally_62.json"));
        assert_eq!(tally.0, "3773094342840");
        assert_eq!(tally.1, "0");
    }

    #[test]
    fn validators_fixture() {
        let rows = parse_validator_rows(&fixture_json("validators.json"));
        assert!(rows.iter().any(|r| r.moniker == "BlockPro"));
        assert!(rows.iter().any(|r| r.moniker == "Blackhox"));
        assert!(rows
            .iter()
            .all(|r| r.status == "BOND_STATUS_BONDED" && !r.jailed));
        let one = parse_validator_rows(&fixture_json("validator_one.json"));
        assert_eq!(one.len(), 1);
        assert!(is_bech32(&one[0].operator, "terpvaloper"));
        assert!(!is_bech32(
            "terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllx",
            "terpvaloper"
        ));
    }

    #[test]
    fn staking_pool_and_params_fixture() {
        let pool = parse_pool(&fixture_json("pool.json"));
        assert_eq!(pool.bonded_tokens, "7127109657182");
        assert_eq!(pool.not_bonded_tokens, "360035182746");
        assert!(format_pool(&pool)[0].contains("TERP"));
        let params = parse_staking_params(&fixture_json("staking_params.json"));
        assert_eq!(params.bond_denom, "uterp");
        assert_eq!(params.max_validators, "69");
        assert_eq!(params.unbonding_time, "1814400s");
    }

    #[test]
    fn ibc_channels_fixture() {
        let rows = parse_channel_rows(&fixture_json("channels.json"));
        assert_eq!(rows[0].port_id, "transfer");
        assert_eq!(rows[0].channel_id, "channel-0");
        assert!(rows[0].counterparty.contains("channel-393"));
        let chain = parse_client_chain_id(&fixture_json("client_state_transfer_channel_0.json"));
        assert_eq!(chain.as_deref(), Some("juno-1"));
    }

    #[test]
    fn upgrade_current_fixture() {
        let plan = parse_upgrade(&fixture_json("current_plan.json"));
        assert!(plan.scheduled);
        assert_eq!(plan.name, "v6.3");
        assert_eq!(plan.height, "23893261");
        assert_eq!(
            format_upgrade(&plan),
            vec!["plan v6.3 at height 23893261".to_string()]
        );
        assert_eq!(
            format_upgrade(&parse_upgrade(&json!({"plan": null}))),
            vec!["no current upgrade plan".to_string()]
        );
    }

    #[test]
    fn bank_supply_fixture() {
        let coins = parse_supply(&fixture_json("supply_uthiol.json"));
        assert_eq!(coins[0].denom, "uthiol");
        assert_eq!(coins[0].amount, "892921666796167");
        let line = format_coin(&coins[0]);
        assert!(line.contains("uthiol"));
        assert!(line.contains("TERP"));
        let uter = parse_supply(&fixture_json("supply_uterp.json"));
        assert_eq!(uter[0].denom, "uterp");
    }

    #[test]
    fn tool_paths_stay_on_the_allowlist() {
        assert!(node::validate(&gov_latest_path(1)).is_ok());
        assert!(node::validate(&gov_latest_path(5)).is_ok());
        assert!(check_latest_n(0).is_err());
        assert!(check_latest_n(6).is_err());
        assert!(node::validate("/cosmos/gov/v1/proposals/62").is_ok());
        assert!(node::validate("/cosmos/gov/v1/proposals/62/tally").is_ok());
        assert!(node::validate(&validators_path(BondStatus::Bonded, 2, true)).is_ok());
        assert!(check_limit(0).is_err());
        assert!(check_limit(101).is_err());
        assert!(node::validate(&validator_path(
            "terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllk"
        ))
        .is_ok());
        assert!(node::validate("/cosmos/staking/v1beta1/pool").is_ok());
        assert!(node::validate("/cosmos/staking/v1beta1/params").is_ok());
        assert!(node::validate("/ibc/core/channel/v1/channels?pagination.limit=100").is_ok());
        assert!(node::validate(&client_state_path("channel-0", "transfer")).is_ok());
        assert!(node::validate("/cosmos/upgrade/v1beta1/current_plan").is_ok());
        assert!(node::validate(&supply_path(Some("uthiol")).unwrap()).is_ok());
        assert!(node::validate(&supply_path(None).unwrap()).is_ok());
        assert!(supply_path(Some("../x")).is_err());
        assert!(node::validate(NODE_INFO_PATH).is_ok());
    }

    #[tokio::test]
    async fn node_versions_reads_fixture_over_32kb() {
        let body = fixture_text("node_info.json");
        assert!(
            body.len() > 32 * 1024,
            "recorded node_info should exceed 32KB"
        );
        let (pool, _server) = fixture_pool(vec![(NODE_INFO_PATH.to_string(), body)]).await;
        let mut budget = Budget::new();
        let cited = node_versions_in(&pool, &mut budget).await.unwrap();
        assert_eq!(cited.data.ibc_go, "v11.2.0");
        assert_eq!(cited.data.sdk, "v0.55.0");
        assert_eq!(cited.height, 23_889_904);
        assert!(!cited.cached);
    }

    #[tokio::test]
    async fn gov_latest_reads_fixture() {
        let (pool, _server) =
            fixture_pool(vec![(gov_latest_path(1), fixture_text("gov_latest.json"))]).await;
        let mut budget = Budget::new();
        let cited = gov_latest_in(&pool, 1, &mut budget).await.unwrap();
        assert_eq!(cited.data[0].id, "62");
        assert!(gov_latest_in(&pool, 6, &mut budget).await.is_err());
    }

    #[tokio::test]
    async fn gov_proposal_reads_fixture() {
        let (pool, _server) = fixture_pool(vec![
            (
                "/cosmos/gov/v1/proposals/62".into(),
                fixture_text("proposal_62.json"),
            ),
            (
                "/cosmos/gov/v1/proposals/62/tally".into(),
                fixture_text("tally_62.json"),
            ),
        ])
        .await;
        let mut budget = Budget::new();
        let cited = gov_proposal_in(&pool, 62, &mut budget).await.unwrap();
        assert_eq!(cited.data.status, "PASSED");
        assert_eq!(cited.data.tally.yes, "3773094342840");
        assert_eq!(cited.path, "/cosmos/gov/v1/proposals/62");
        assert_eq!(
            cited.data.tally.source.path,
            "/cosmos/gov/v1/proposals/62/tally"
        );
    }

    #[tokio::test]
    async fn validators_and_one_validator_read_fixtures() {
        let op = "terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllk";
        let (pool, _server) = fixture_pool(vec![
            (
                validators_path(BondStatus::Bonded, 2, false),
                fixture_text("validators.json"),
            ),
            (validator_path(op), fixture_text("validator_one.json")),
        ])
        .await;
        let mut budget = Budget::new();
        let set = validators_in(&pool, BondStatus::Bonded, 2, false, &mut budget)
            .await
            .unwrap();
        assert!(set.data.rows.iter().any(|r| r.moniker == "BlockPro"));
        let one = validator_in(&pool, op, &mut budget).await.unwrap();
        assert_eq!(one.data.operator, op);
        assert!(validator_in(
            &pool,
            "terpvaloper1notachecksumxxxxxxxxxxxxxxxxxx",
            &mut budget
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn staking_reads_fixtures() {
        let (pool, _server) = fixture_pool(vec![
            (
                "/cosmos/staking/v1beta1/pool".into(),
                fixture_text("pool.json"),
            ),
            (
                "/cosmos/staking/v1beta1/params".into(),
                fixture_text("staking_params.json"),
            ),
        ])
        .await;
        let mut budget = Budget::new();
        let pool_c = staking_pool_in(&pool, &mut budget).await.unwrap();
        let params = staking_params_in(&pool, &mut budget).await.unwrap();
        assert_eq!(pool_c.data.bonded_tokens, "7127109657182");
        assert_eq!(params.data.bond_denom, "uterp");
    }

    #[tokio::test]
    async fn ibc_channels_read_client_state_fixture() {
        let (pool, server) = fixture_pool(vec![
            (
                "/ibc/core/channel/v1/channels?pagination.limit=100".into(),
                fixture_text("channels.json"),
            ),
            (
                client_state_path("channel-0", "transfer"),
                fixture_text("client_state_transfer_channel_0.json"),
            ),
            (
                client_state_path("channel-1", "transfer"),
                r#"{"identified_client_state":{"client_id":"07-tendermint-3","client_state":{"chain_id":"osmosis-1"}}}"#.into(),
            ),
        ])
        .await;
        let mut budget = Budget::new();
        let one = ibc_channels_in(&pool, Some("channel-0"), &mut budget)
            .await
            .unwrap();
        assert_eq!(one.data.len(), 1);
        assert_eq!(one.data[0].chain_id.as_deref(), Some("juno-1"));
        let hits = server.hits.lock().unwrap().clone();
        assert!(hits.iter().any(|p| p.contains("channel-0")));
        assert!(hits.iter().all(|p| !p.contains("channel-1")));
        let all = ibc_channels_in(&pool, Some("osmosis"), &mut budget)
            .await
            .unwrap();
        assert_eq!(all.data.len(), 1);
        assert_eq!(all.data[0].channel_id, "channel-1");
        assert_eq!(all.data[0].chain_id.as_deref(), Some("osmosis-1"));
    }

    #[tokio::test]
    async fn upgrade_and_supply_read_fixtures() {
        let (pool, _server) = fixture_pool(vec![
            (
                "/cosmos/upgrade/v1beta1/current_plan".into(),
                fixture_text("current_plan.json"),
            ),
            (
                supply_path(Some("uthiol")).unwrap(),
                fixture_text("supply_uthiol.json"),
            ),
        ])
        .await;
        let mut budget = Budget::new();
        let plan = upgrade_current_in(&pool, &mut budget).await.unwrap();
        assert_eq!(plan.data.name, "v6.3");
        assert_eq!(plan.data.height, "23893261");
        let coins = bank_supply_in(&pool, Some("uthiol"), &mut budget)
            .await
            .unwrap();
        assert_eq!(coins.data[0].amount, "892921666796167");
    }
}
