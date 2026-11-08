//! Native staking + bank actions a DAO executes from its treasury.
//!
//! Complements `DaoVoteDelegation` (voting-power delegates). This module
//! builds CosmosMsgs for `MsgDelegate` / `MsgSend` so Foundation-style
//! proposals (spread stake, gas a new operator, instantiate a subDAO) share
//! one builder.

use cosmwasm_std::{AnyMsg, Binary, Coin, CosmosMsg, Decimal, Uint256, WasmMsg};

/// 6-decimal micro units (uterp / uthiol).
pub const MICRO: u128 = 1_000_000;

/// Live morocco-1 operator account (quoted). Operator bech32 is derived.
pub const NEW_VALIDATOR_ACCOUNT: &str = "terp1gr67fce5ymhyz93qe0flqlgv2p424a48x8atl8";
/// Derived `terpvaloper` for [`NEW_VALIDATOR_ACCOUNT`] (same words, valoper HRP).
pub const NEW_VALIDATOR_OPERATOR: &str = "terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllk";

pub const STAKE_DENOM: &str = "uterp";
pub const GAS_DENOM: &str = "uthiol";

pub const FIAB_NAME: &str = "FIAB-SUBDAO";
pub const FIAB_DESCRIPTION: &str = "'Say Fuck It And Begin'. A forward deployment subdao for Terp Network Foundation, deploying and interacting with Terp Network's ecosystem in a minimally constrained structure. Allows us to curate data points, and reflect for improving the stewardship and collaborative support of growth.";

/// Live TerpNet Foundation dao-dao core (gov 56 / v6.1 constants).
pub const PARENT_CORE: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";
pub const PROPOSAL_A: &str = "terp1p5582k3sw008jkmw0f48ucszn660nazqr9x3ut6h5aqj4frq8f0shaey4g";
pub const PRE_PROPOSE_A: &str = "terp1armcr7lme0ac3l59dx6ne3et3pe9mfyv57d8pc9m5tyxtzm64dpq9nnwpp";

/// morocco-1 codes queried 2026-10-08. Every row below is cw2 `2.8.0-alpha.2`
/// except cw4-group, which has no `info` query.
///
/// `2.8.0-alpha.3` (`0ec88c33`) is the patched release and is not stored yet.
/// `DaoDaoSuite::upload_patch_set` stores replacements for core, voting-cw4,
/// and cw4-group. Proposal single, proposal multiple (code 25), and
/// pre-propose stay on these IDs.
pub const CODE_CORE: u64 = 23;
pub const CODE_PROPOSAL_SINGLE: u64 = 24;
pub const CODE_PROPOSAL_MULTIPLE: u64 = 25;
pub const CODE_PRE_PROPOSE_SINGLE: u64 = 31;
pub const CODE_VOTING_CW4: u64 = 56;
pub const CODE_CW4_GROUP: u64 = 61;

/// 200_000 TERP in uterp.
pub fn terp_200k() -> Uint256 {
    Uint256::from(200_000u128 * MICRO)
}

/// 100_000 TERP in uterp.
pub fn terp_100k() -> Uint256 {
    Uint256::from(100_000u128 * MICRO)
}

/// 1_000_000 TERP / THIOL in micro units (1e12).
pub fn million_tokens() -> Uint256 {
    Uint256::from(1_000_000u128 * MICRO)
}

/// 7.10 THIOL in uthiol.
pub fn thiol_7_10() -> Uint256 {
    Uint256::from(7_100_000u128)
}

pub fn coin(amount: Uint256, denom: &str) -> Coin {
    Coin {
        denom: denom.to_string(),
        amount,
    }
}

/// `CosmosMsg::Staking(Delegate)` — for cw-multi-test. Live morocco-1
/// dao-proposal-single was built without the staking variant; use
/// [`delegate_any_msg`] on chain.
pub fn delegate_msg(validator: impl Into<String>, amount: Uint256, denom: &str) -> CosmosMsg {
    CosmosMsg::Staking(cosmwasm_std::StakingMsg::Delegate {
        validator: validator.into(),
        amount: coin(amount, denom),
    })
}

fn proto_varint(buf: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        buf.push((v as u8) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

fn proto_ld(buf: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    proto_varint(buf, u64::from((field << 3) | 2));
    proto_varint(buf, bytes.len() as u64);
    buf.extend_from_slice(bytes);
}

fn proto_string(buf: &mut Vec<u8>, field: u32, s: &str) {
    proto_ld(buf, field, s.as_bytes());
}

/// `/cosmos.staking.v1beta1.MsgDelegate` protobuf body.
pub fn encode_msg_delegate(
    delegator: &str,
    validator: &str,
    amount: Uint256,
    denom: &str,
) -> Vec<u8> {
    let mut coin_pb = Vec::new();
    proto_string(&mut coin_pb, 1, denom);
    proto_string(&mut coin_pb, 2, &amount.to_string());
    let mut msg = Vec::new();
    proto_string(&mut msg, 1, delegator);
    proto_string(&mut msg, 2, validator);
    proto_ld(&mut msg, 3, &coin_pb);
    msg
}

/// `/cosmos.gov.v1.MsgDeposit`. Field 1 is the proposal id, field 2 the depositor,
/// field 3 the coin. The on-chain DAO has no gov-deposit variant, so this is
/// wrapped as `CosmosMsg::Any` and the depositor must be the DAO itself.
pub fn encode_msg_deposit(
    proposal_id: u64,
    depositor: &str,
    amount: Uint256,
    denom: &str,
) -> Vec<u8> {
    let mut coin_pb = Vec::new();
    proto_string(&mut coin_pb, 1, denom);
    proto_string(&mut coin_pb, 2, &amount.to_string());
    let mut msg = Vec::new();
    proto_varint(&mut msg, 1 << 3);
    proto_varint(&mut msg, proposal_id);
    proto_string(&mut msg, 2, depositor);
    proto_ld(&mut msg, 3, &coin_pb);
    msg
}

pub fn gov_deposit_any_msg(
    depositor: impl Into<String>,
    proposal_id: u64,
    amount: Uint256,
    denom: &str,
) -> CosmosMsg {
    let depositor = depositor.into();
    CosmosMsg::Any(AnyMsg {
        type_url: "/cosmos.gov.v1.MsgDeposit".into(),
        value: Binary::from(encode_msg_deposit(proposal_id, &depositor, amount, denom)),
    })
}

fn proto_key(buf: &mut Vec<u8>, field: u32, wire: u32) {
    proto_varint(buf, u64::from((field << 3) | wire));
}

fn proto_int64(buf: &mut Vec<u8>, field: u32, v: i64) {
    proto_key(buf, field, 0);
    proto_varint(buf, v as u64);
}

fn encode_any(type_url: &str, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    proto_string(&mut out, 1, type_url);
    proto_ld(&mut out, 2, value);
    out
}

/// `/cosmos.upgrade.v1beta1.Plan` without the deprecated time and client fields.
pub fn encode_upgrade_plan(name: &str, height: i64, info: &str) -> Vec<u8> {
    let mut plan = Vec::new();
    proto_string(&mut plan, 1, name);
    proto_int64(&mut plan, 3, height);
    proto_string(&mut plan, 4, info);
    plan
}

/// `/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade`. `authority` is the gov module.
pub fn encode_msg_software_upgrade(
    authority: &str,
    name: &str,
    height: i64,
    info: &str,
) -> Vec<u8> {
    let mut msg = Vec::new();
    proto_string(&mut msg, 1, authority);
    proto_ld(&mut msg, 2, &encode_upgrade_plan(name, height, info));
    msg
}

/// Prepared expedited software-upgrade draft. v6.4 is not a second message.
#[derive(Clone, Debug, PartialEq)]
pub struct UpgradeDraft {
    pub authority: String,
    pub plan_name: String,
    pub height: i64,
    pub info: String,
    pub metadata: String,
    pub title: String,
    pub summary: String,
    pub deposit_amount: Uint256,
    pub deposit_denom: String,
}

pub fn v63_upgrade_draft_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../networks/upgrades/v6.3/draft_proposal.json")
}

pub fn load_upgrade_draft(path: &std::path::Path) -> Result<UpgradeDraft, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| format!("parse {}: {err}", path.display()))?;
    if value.get("expedited").and_then(|v| v.as_bool()) != Some(true) {
        return Err("draft expedited must be true".into());
    }
    let messages = value
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or("draft messages")?;
    if messages.len() != 1 {
        return Err("draft must contain exactly one message".into());
    }
    let msg = &messages[0];
    let type_url = msg.get("@type").and_then(|v| v.as_str()).unwrap_or("");
    if type_url != "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade" {
        return Err(format!("draft message is {type_url}"));
    }
    let authority = msg
        .get("authority")
        .and_then(|v| v.as_str())
        .ok_or("draft authority")?
        .to_string();
    let plan = msg.get("plan").ok_or("draft plan")?;
    if plan.get("upgraded_client_state").map(|v| !v.is_null()) == Some(true) {
        return Err("draft plan.upgraded_client_state must stay empty".into());
    }
    let time = plan.get("time").and_then(|v| v.as_str()).unwrap_or("");
    if !time.is_empty() && time != "0001-01-01T00:00:00Z" {
        return Err("draft plan.time is not the empty sentinel; refusing to drop it".into());
    }
    let plan_name = plan
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("draft plan.name")?
        .to_string();
    let height = plan
        .get("height")
        .and_then(|v| v.as_str())
        .ok_or("draft plan.height")?
        .parse::<i64>()
        .map_err(|_| "draft plan.height".to_string())?;
    let info = plan
        .get("info")
        .and_then(|v| v.as_str())
        .ok_or("draft plan.info")?
        .to_string();
    let metadata = value
        .get("metadata")
        .and_then(|v| v.as_str())
        .ok_or("draft metadata")?
        .to_string();
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or("draft title")?
        .to_string();
    let summary = value
        .get("summary")
        .and_then(|v| v.as_str())
        .ok_or("draft summary")?
        .to_string();
    let deposit = value
        .get("deposit")
        .and_then(|v| v.as_str())
        .ok_or("draft deposit")?;
    let split = deposit
        .find(|c: char| !c.is_ascii_digit())
        .ok_or("draft deposit needs a denom")?;
    let deposit_amount = deposit[..split]
        .parse::<u128>()
        .map(Uint256::from)
        .map_err(|_| "draft deposit amount".to_string())?;
    let deposit_denom = deposit[split..].to_string();
    if metadata.len() > 255 || title.is_empty() || title.len() > 255 || summary.is_empty() {
        return Err("draft title, metadata, or summary is outside the gov length limits".into());
    }
    Ok(UpgradeDraft {
        authority,
        plan_name,
        height,
        info,
        metadata,
        title,
        summary,
        deposit_amount,
        deposit_denom,
    })
}

/// The v6.3 submit plus the v6.4 checksum file it must stay aligned with.
/// Every field is read from `networks/upgrades/`. Nothing here is a second copy.
#[derive(Clone, Debug)]
pub struct UpgradeCut {
    pub draft: UpgradeDraft,
    pub halt_time_utc: String,
    pub v64_height: i64,
    pub submit_by_utc: String,
    pub v63_commit: String,
    pub v64_commit: String,
}

pub fn load_upgrade_cut(draft_path: &std::path::Path) -> Result<UpgradeCut, String> {
    let draft = load_upgrade_draft(draft_path)?;
    let dir = draft_path.parent().ok_or("draft has no directory")?;
    let draft_json = read_json(draft_path)?;
    let proposal_json = read_json(&dir.join("proposal.json"))?;
    if draft_json != proposal_json {
        return Err("networks/upgrades/v6.3/proposal.json and draft_proposal.json differ".into());
    }
    let meta = read_json(&dir.join("metadata.json"))?;
    let v63_bins = read_json(&dir.join("cosmovisor.json"))?;
    let v64_bins = read_json(&dir.join("../v6.4/cosmovisor.json"))?;
    if read_json(&dir.join("binaries.json"))? != v63_bins {
        return Err("v6.3/binaries.json is not v6.3/cosmovisor.json".into());
    }
    if read_json(&dir.join("../v6.4/binaries.json"))? != v64_bins {
        return Err("v6.4/binaries.json is not v6.4/cosmovisor.json".into());
    }
    let halt_height = meta
        .pointer("/halt/height")
        .and_then(|v| v.as_i64())
        .ok_or("metadata halt.height")?;
    if halt_height != draft.height {
        return Err(format!(
            "metadata halt.height {halt_height} != draft {}",
            draft.height
        ));
    }
    let halt_time_utc = json_string(&meta, "/halt/time_utc")?;
    let v64_height = meta
        .pointer("/next_plan/height")
        .and_then(|v| v.as_i64())
        .ok_or("metadata next_plan.height")?;
    if v64_height != draft.height + 2 {
        return Err(format!(
            "v6.4 height {v64_height} is not draft height {} + 2",
            draft.height
        ));
    }
    if json_string(&meta, "/next_plan/name")? != "v6.4" || json_string(&meta, "/plan")? != "v6.3" {
        return Err("metadata plan names must be v6.3 then v6.4".into());
    }
    let deposit = format!("{}{}", draft.deposit_amount, draft.deposit_denom);
    if json_string(&meta, "/deposit")? != deposit {
        return Err("metadata deposit != draft deposit".into());
    }
    let info: serde_json::Value = serde_json::from_str(&draft.info)
        .map_err(|err| format!("draft plan.info is not JSON: {err}"))?;
    if info != v63_bins {
        return Err("draft plan.info is not networks/upgrades/v6.3/cosmovisor.json".into());
    }
    let v63_commit = json_string(&meta, "/binary_commits/v6.3.0")?;
    let v64_commit = json_string(&meta, "/binary_commits/v6.4.0")?;
    if !is_git_sha(&v63_commit) || !is_git_sha(&v64_commit) {
        return Err("metadata binary_commits must be 40 hex characters".into());
    }
    check_published_bins(
        &v63_bins,
        meta.pointer("/v6_3_tarball_sha256")
            .ok_or("v6.3 checksums")?,
        "v6.3.0",
    )?;
    check_published_bins(
        &v64_bins,
        meta.pointer("/v6_4_tarball_sha256")
            .ok_or("v6.4 checksums")?,
        "v6.4.0",
    )?;
    Ok(UpgradeCut {
        draft,
        halt_time_utc,
        v64_height,
        submit_by_utc: json_string(&meta, "/submit_by_utc")?,
        v63_commit,
        v64_commit,
    })
}

fn read_json(path: &std::path::Path) -> Result<serde_json::Value, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("parse {}: {err}", path.display()))
}

fn json_string(value: &serde_json::Value, pointer: &str) -> Result<String, String> {
    value
        .pointer(pointer)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("missing {pointer}"))
}

fn is_git_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn check_published_bins(
    cosmo: &serde_json::Value,
    shas: &serde_json::Value,
    tag: &str,
) -> Result<(), String> {
    let bins = cosmo
        .get("binaries")
        .and_then(|v| v.as_object())
        .ok_or_else(|| format!("{tag} cosmovisor.json has no binaries"))?;
    if bins.len() != 2 || !bins.contains_key("linux/amd64") || !bins.contains_key("linux/arm64") {
        return Err(format!(
            "{tag} cosmovisor.json must list linux/amd64 and linux/arm64 only"
        ));
    }
    for (arch, url) in bins {
        let url = url.as_str().ok_or_else(|| format!("{tag} {arch} url"))?;
        if url.contains("file://") || url.contains("darwin") {
            return Err(format!("{tag} {arch} is not a public linux tarball"));
        }
        let sha = shas
            .get(arch)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("metadata checksum for {tag} {arch}"))?;
        if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{tag} {arch} checksum is not 64 hex characters"));
        }
        let prefix = format!("https://s3.terp.network/releases/terp-core/{tag}/");
        if !url.starts_with(&prefix) || !url.contains(&format!("checksum=sha256:{sha}")) {
            return Err(format!("{tag} {arch} url does not match metadata"));
        }
    }
    Ok(())
}

/// `/cosmos.gov.v1.MsgSubmitProposal` carrying one software-upgrade message.
/// `proposer` is the account that signs, which for a DAO execution is the DAO.
pub fn encode_msg_submit_upgrade(proposer: &str, draft: &UpgradeDraft) -> Vec<u8> {
    let upgrade = encode_msg_software_upgrade(
        &draft.authority,
        &draft.plan_name,
        draft.height,
        &draft.info,
    );
    let upgrade_any = encode_any("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade", &upgrade);
    let mut coin = Vec::new();
    proto_string(&mut coin, 1, &draft.deposit_denom);
    proto_string(&mut coin, 2, &draft.deposit_amount.to_string());
    let mut msg = Vec::new();
    proto_ld(&mut msg, 1, &upgrade_any);
    proto_ld(&mut msg, 2, &coin);
    proto_string(&mut msg, 3, proposer);
    // Field 4 (metadata) stays empty, matching tools/v63-upgrade.
    proto_string(&mut msg, 5, &draft.title);
    proto_string(&mut msg, 6, &draft.summary);
    proto_key(&mut msg, 7, 0);
    proto_varint(&mut msg, 1);
    msg
}

pub fn gov_submit_upgrade_any_msg(proposer: impl Into<String>, draft: &UpgradeDraft) -> CosmosMsg {
    let proposer = proposer.into();
    CosmosMsg::Any(AnyMsg {
        type_url: "/cosmos.gov.v1.MsgSubmitProposal".into(),
        value: Binary::from(encode_msg_submit_upgrade(&proposer, draft)),
    })
}

/// `/cosmos.gov.v1.MsgVote` with `VOTE_OPTION_YES`. `voter` must be the DAO.
pub fn encode_msg_vote_yes(proposal_id: u64, voter: &str) -> Vec<u8> {
    let mut msg = Vec::new();
    proto_key(&mut msg, 1, 0);
    proto_varint(&mut msg, proposal_id);
    proto_string(&mut msg, 2, voter);
    proto_key(&mut msg, 3, 0);
    proto_varint(&mut msg, 1);
    msg
}

pub fn gov_vote_yes_any_msg(voter: impl Into<String>, proposal_id: u64) -> CosmosMsg {
    let voter = voter.into();
    CosmosMsg::Any(AnyMsg {
        type_url: "/cosmos.gov.v1.MsgVote".into(),
        value: Binary::from(encode_msg_vote_yes(proposal_id, &voter)),
    })
}

/// dao-voting `does_vote_count_pass` for a percent quorum (`votes >= total * percent`).
/// `quorum_percent` is a decimal string such as `"0.2"`.
pub fn quorum_met(votes: u128, total_power: u128, quorum_percent: &str) -> Result<bool, String> {
    let percent: Decimal = quorum_percent
        .parse()
        .map_err(|err| format!("quorum percent {quorum_percent}: {err}"))?;
    let den = Uint256::from(10u128.pow(percent.decimal_places()));
    let left = Uint256::from(votes)
        .checked_mul(den)
        .map_err(|_| "quorum numerator overflow".to_string())?;
    let right = Uint256::from(total_power)
        .checked_mul(Uint256::from(percent.atomics().u128()))
        .map_err(|_| "quorum denominator overflow".to_string())?;
    Ok(left >= right)
}

/// Smallest yes-only vote weight that meets `quorum_percent` of `total_power`.
pub fn smallest_yes_meeting_quorum(
    total_power: u128,
    quorum_percent: &str,
) -> Result<u128, String> {
    if total_power == 0 {
        return Err("total power is zero".into());
    }
    let mut lo = 0u128;
    let mut hi = total_power;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if quorum_met(mid, total_power, quorum_percent)? {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

/// Strict majority of total power. This is the early-pass test in
/// dao-proposal-single while the proposal is still open, `allow_revoting`
/// is false, and `min_voting_period` is unset: `yes * 2 > total_power`.
pub fn early_majority(yes: u128, total_power: u128) -> bool {
    yes.saturating_mul(2) > total_power
}

/// True when the smallest yes-only vote that meets quorum is already an
/// early majority. Quorum by itself does not make the proposal executable.
pub fn executes_when_quorum_met(total_power: u128, quorum_percent: &str) -> Result<bool, String> {
    let yes = smallest_yes_meeting_quorum(total_power, quorum_percent)?;
    Ok(early_majority(yes, total_power))
}

/// One cosmos-gov deposit. `spec` is `<proposal-id>:<amount><denom>`,
/// for example `60:5000000000uterp`. The amount is already in micro-units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GovDepositSpec {
    pub proposal_id: u64,
    pub amount: Uint256,
    pub denom: String,
}

pub fn parse_gov_deposit(spec: &str) -> Result<GovDepositSpec, String> {
    let (id, rest) = spec
        .split_once(':')
        .ok_or_else(|| format!("deposit must look like 60:5000000000uterp, got {spec}"))?;
    let proposal_id: u64 = id.parse().map_err(|_| format!("proposal id {id}"))?;
    let split = rest
        .find(|c: char| c.is_ascii_alphabetic())
        .ok_or_else(|| format!("deposit {spec} needs a denom"))?;
    if split == 0 || !rest[..split].chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "deposit amount must be an integer before the denom, got {spec}"
        ));
    }
    let amount: u128 = rest[..split]
        .parse()
        .map_err(|_| format!("deposit amount {}", &rest[..split]))?;
    let denom = rest[split..].to_string();
    if denom.is_empty() || !denom.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("deposit denom in {spec}"));
    }
    Ok(GovDepositSpec {
        proposal_id,
        amount: Uint256::from(amount),
        denom,
    })
}

/// Remaining half of the morocco-1 10,000 TERP minimum, in uterp.
pub const HALF_MIN_DEPOSIT_UTERP: u128 = 5_000 * MICRO;

/// Live-chain delegate. On-chain CosmosMsg has no `staking` variant.
pub fn delegate_any_msg(
    delegator: impl Into<String>,
    validator: impl Into<String>,
    amount: Uint256,
    denom: &str,
) -> CosmosMsg {
    CosmosMsg::Any(AnyMsg {
        type_url: "/cosmos.staking.v1beta1.MsgDelegate".into(),
        value: Binary::from(encode_msg_delegate(
            &delegator.into(),
            &validator.into(),
            amount,
            denom,
        )),
    })
}

/// `CosmosMsg::Bank(Send)` from the DAO treasury.
pub fn bank_send_msg(to: impl Into<String>, amount: Uint256, denom: &str) -> CosmosMsg {
    CosmosMsg::Bank(cosmwasm_std::BankMsg::Send {
        to_address: to.into(),
        amount: vec![coin(amount, denom)],
    })
}

/// Re-encode a bech32 account as `terpvaloper…`. No-op if already valoper.
pub fn to_valoper(addr: &str) -> Result<String, String> {
    if addr.starts_with("terpvaloper1") {
        return Ok(addr.to_string());
    }
    if !addr.contains('1') {
        return Err(format!("not bech32: {addr}"));
    }
    // Keep payload+checksum; swap HRP. Caller should prefer a quoted valoper.
    // Full convert: decode words minus checksum, re-checksum under new HRP.
    convert_hrp(addr, "terpvaloper")
}

fn convert_hrp(addr: &str, new_hrp: &str) -> Result<String, String> {
    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let (hrp, data) = addr
        .split_once('1')
        .ok_or_else(|| format!("not bech32: {addr}"))?;
    let _ = hrp;
    let vals: Result<Vec<u8>, _> = data
        .bytes()
        .map(|c| {
            CHARSET
                .iter()
                .position(|&x| x == c)
                .map(|i| i as u8)
                .ok_or("bad bech32 char")
        })
        .collect();
    let mut vals = vals.map_err(|e| e.to_string())?;
    if vals.len() < 6 {
        return Err("bech32 too short".into());
    }
    vals.truncate(vals.len() - 6);
    Ok(bech32_encode(new_hrp, &vals))
}

fn bech32_polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ (*v as u32);
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn bech32_encode(hrp: &str, data: &[u8]) -> String {
    const CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let mut values: Vec<u8> = hrp.bytes().map(|x| x >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|x| x & 31));
    values.extend_from_slice(data);
    values.extend_from_slice(&[0; 6]);
    let polymod = bech32_polymod(&values) ^ 1;
    let mut checksum = [0u8; 6];
    for (i, c) in checksum.iter_mut().enumerate() {
        *c = ((polymod >> (5 * (5 - i))) & 31) as u8;
    }
    let mut out = String::from(hrp);
    out.push('1');
    for v in data.iter().chain(checksum.iter()) {
        out.push(CHARSET[*v as usize] as char);
    }
    out
}

/// One Foundation-style native delegation campaign.
#[derive(Clone, Debug)]
pub struct NativeDelegationPlan {
    pub stake_denom: String,
    pub gas_denom: String,
    pub new_validator_operator: String,
    pub new_validator_account: String,
    pub new_validator_stake: Uint256,
    pub new_validator_gas: Uint256,
    pub other_validators: Vec<String>,
    pub other_stake: Uint256,
}

impl NativeDelegationPlan {
    /// morocco-1 defaults: 200k TERP + 7.10 THIOL to the new operator,
    /// 100k TERP to every other bonded validator.
    pub fn terp_foundation(other_validators: Vec<String>) -> Self {
        let others: Vec<String> = other_validators
            .into_iter()
            .filter(|v| v != NEW_VALIDATOR_OPERATOR && v != NEW_VALIDATOR_ACCOUNT)
            .collect();
        Self {
            stake_denom: STAKE_DENOM.into(),
            gas_denom: GAS_DENOM.into(),
            new_validator_operator: NEW_VALIDATOR_OPERATOR.into(),
            new_validator_account: NEW_VALIDATOR_ACCOUNT.into(),
            new_validator_stake: terp_200k(),
            new_validator_gas: thiol_7_10(),
            other_validators: others,
            other_stake: terp_100k(),
        }
    }

    /// 100k TERP to each bonded validator. No new-operator stake or THIOL —
    /// they get gas off-proposal so they can create the valoper first.
    pub fn bonded_set(other_validators: Vec<String>) -> Self {
        let mut plan = Self::terp_foundation(other_validators);
        plan.new_validator_stake = Uint256::zero();
        plan.new_validator_gas = Uint256::zero();
        plan
    }

    pub fn cosmos_msgs(&self) -> Vec<CosmosMsg> {
        let mut msgs = Vec::with_capacity(self.other_validators.len() + 2);
        if !self.new_validator_stake.is_zero() {
            msgs.push(delegate_msg(
                &self.new_validator_operator,
                self.new_validator_stake,
                &self.stake_denom,
            ));
        }
        if !self.new_validator_gas.is_zero() {
            msgs.push(bank_send_msg(
                &self.new_validator_account,
                self.new_validator_gas,
                &self.gas_denom,
            ));
        }
        for v in &self.other_validators {
            msgs.push(delegate_msg(v, self.other_stake, &self.stake_denom));
        }
        msgs
    }

    /// Same campaign as [`Self::cosmos_msgs`] using `CosmosMsg::Any` so
    /// morocco-1 proposal-single will deserialize it.
    pub fn cosmos_msgs_any(&self, delegator: &str) -> Vec<CosmosMsg> {
        let mut msgs = Vec::with_capacity(self.other_validators.len() + 2);
        if !self.new_validator_stake.is_zero() {
            msgs.push(delegate_any_msg(
                delegator,
                &self.new_validator_operator,
                self.new_validator_stake,
                &self.stake_denom,
            ));
        }
        if !self.new_validator_gas.is_zero() {
            msgs.push(bank_send_msg(
                &self.new_validator_account,
                self.new_validator_gas,
                &self.gas_denom,
            ));
        }
        for v in &self.other_validators {
            msgs.push(delegate_any_msg(
                delegator,
                v,
                self.other_stake,
                &self.stake_denom,
            ));
        }
        msgs
    }

    /// Append extra DAO actions (e.g. FIAB-SUBDAO instantiate) so one proposal
    /// can stake and spin a subDAO together.
    pub fn with_extra(self, extra: Vec<CosmosMsg>) -> Vec<CosmosMsg> {
        let mut msgs = self.cosmos_msgs();
        msgs.extend(extra);
        msgs
    }
}

/// Wasm instantiate of a subDAO under `admin` (parent DAO). Funds seed the
/// new core. `msg` is the dao-dao-core InstantiateMsg JSON bytes.
pub fn instantiate_subdao_msg(
    admin: impl Into<String>,
    code_id: u64,
    label: impl Into<String>,
    msg: Vec<u8>,
    funds: Vec<Coin>,
) -> CosmosMsg {
    let admin = admin.into();
    CosmosMsg::Wasm(WasmMsg::Instantiate {
        admin: Some(admin),
        code_id,
        msg: msg.into(),
        funds,
        label: label.into(),
    })
}

/// Vote-yes payload for dao-proposal-single (after the proposal exists).
pub fn vote_yes_msg(proposal_id: u64) -> dao_proposal_single::msg::ExecuteMsg {
    dao_proposal_single::msg::ExecuteMsg::Vote {
        proposal_id,
        vote: dao_voting::voting::Vote::Yes,
        rationale: None,
    }
}

pub fn execute_proposal_msg(proposal_id: u64) -> dao_proposal_single::msg::ExecuteMsg {
    dao_proposal_single::msg::ExecuteMsg::Execute { proposal_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valoper_convert_matches_quoted() {
        assert_eq!(
            to_valoper(NEW_VALIDATOR_ACCOUNT).unwrap(),
            NEW_VALIDATOR_OPERATOR
        );
        assert_eq!(
            to_valoper(NEW_VALIDATOR_OPERATOR).unwrap(),
            NEW_VALIDATOR_OPERATOR
        );
    }

    #[test]
    fn plan_excludes_new_from_others_and_uses_weight_amounts() {
        let others = vec![
            NEW_VALIDATOR_OPERATOR.to_string(),
            "terpvaloper1aaaa".into(),
            "terpvaloper1bbbb".into(),
        ];
        let plan = NativeDelegationPlan::terp_foundation(others);
        assert_eq!(plan.other_validators.len(), 2);
        let msgs = plan.cosmos_msgs();
        // 1 delegate new + 1 send + 2 others
        assert_eq!(msgs.len(), 4);
        match &msgs[0] {
            CosmosMsg::Staking(cosmwasm_std::StakingMsg::Delegate { validator, amount }) => {
                assert_eq!(validator, NEW_VALIDATOR_OPERATOR);
                assert_eq!(amount.amount, terp_200k());
                assert_eq!(amount.denom, "uterp");
            }
            other => panic!("{other:?}"),
        }
        match &msgs[1] {
            CosmosMsg::Bank(cosmwasm_std::BankMsg::Send { to_address, amount }) => {
                assert_eq!(to_address, NEW_VALIDATOR_ACCOUNT);
                assert_eq!(amount[0].amount, thiol_7_10());
                assert_eq!(amount[0].denom, "uthiol");
            }
            other => panic!("{other:?}"),
        }
        match &msgs[2] {
            CosmosMsg::Staking(cosmwasm_std::StakingMsg::Delegate { amount, .. }) => {
                assert_eq!(amount.amount, terp_100k());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bonded_set_is_others_only_no_thiol() {
        let plan = NativeDelegationPlan::bonded_set(vec![
            NEW_VALIDATOR_OPERATOR.to_string(),
            "terpvaloper1aaaa".into(),
            "terpvaloper1bbbb".into(),
        ]);
        let msgs = plan.cosmos_msgs();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|m| matches!(m, CosmosMsg::Staking(_))));
        assert!(!msgs.iter().any(|m| matches!(m, CosmosMsg::Bank(_))));
    }

    #[test]
    fn msg_delegate_proto_contains_addresses() {
        let b = encode_msg_delegate(
            PARENT_CORE,
            "terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw",
            terp_100k(),
            STAKE_DENOM,
        );
        let s = String::from_utf8_lossy(&b);
        assert!(s.contains(PARENT_CORE));
        assert!(s.contains("terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw"));
        assert!(s.contains("uterp"));
        assert!(s.contains("100000000000"));
    }

    #[test]
    fn msg_deposit_proto_carries_proposal_and_amount() {
        let amount = Uint256::from(10_000u128 * MICRO);
        let raw = encode_msg_deposit(60, PARENT_CORE, amount, STAKE_DENOM);
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains(PARENT_CORE));
        assert!(text.contains("uterp"));
        assert!(text.contains("10000000000"));
        assert_eq!(raw[0], 0x08);
        assert_eq!(raw[1], 60);
        match gov_deposit_any_msg(PARENT_CORE, 61, amount, STAKE_DENOM) {
            CosmosMsg::Any(any) => {
                assert_eq!(any.type_url, "/cosmos.gov.v1.MsgDeposit");
                assert!(any.value.len() > 8);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn foundation_quorum_does_not_execute() {
        // Live cw4 power is 2999 and the proposal quorum is 20%.
        assert_eq!(smallest_yes_meeting_quorum(2999, "0.2").unwrap(), 600);
        assert!(quorum_met(1000, 2999, "0.2").unwrap());
        assert!(!early_majority(1000, 2999));
        assert!(early_majority(1999, 2999));
        assert!(early_majority(2000, 2999));
        assert!(!executes_when_quorum_met(2999, "0.2").unwrap());
    }

    #[test]
    fn expedited_upgrade_draft_is_v63_only() {
        let path = v63_upgrade_draft_path();
        let cut = load_upgrade_cut(&path).unwrap();
        let draft = &cut.draft;
        assert_eq!(draft.plan_name, "v6.3");
        assert_ne!(cut.v63_commit, cut.v64_commit);
        assert_eq!(cut.v64_height, draft.height + 2);
        assert!(!cut.halt_time_utc.is_empty());
        assert!(!cut.submit_by_utc.is_empty());
        let info: serde_json::Value = serde_json::from_str(&draft.info).unwrap();
        let bins = info["binaries"].as_object().unwrap();
        assert_eq!(bins.len(), 2);
        assert_eq!(
            draft.authority,
            "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq"
        );
        assert_eq!(draft.deposit_amount, Uint256::from(100_000_000_000u128));
        assert_eq!(draft.deposit_denom, "uterp");
        assert!(draft.info.contains("linux/amd64"));
        assert!(draft.info.contains("linux/arm64"));
        assert!(!draft.info.contains("darwin"));
        let raw = encode_msg_submit_upgrade(PARENT_CORE, draft);
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains(&draft.height.to_string()));
        for url in bins.values() {
            let sha = url
                .as_str()
                .unwrap()
                .split_once("checksum=sha256:")
                .unwrap()
                .1;
            assert_eq!(sha.len(), 64);
            assert!(text.contains(sha));
        }
        assert!(text.contains(PARENT_CORE));
        assert!(text.contains(&draft.authority));
        assert!(text.contains("v6.3"));
        assert!(!text.contains(&draft.metadata));
        assert_eq!(
            text.matches("/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade")
                .count(),
            1
        );
        assert_eq!(*raw.last().unwrap(), 1);
        match gov_submit_upgrade_any_msg(PARENT_CORE, draft) {
            CosmosMsg::Any(any) => assert_eq!(any.type_url, "/cosmos.gov.v1.MsgSubmitProposal"),
            other => panic!("{other:?}"),
        }
        let vote = encode_msg_vote_yes(62, PARENT_CORE);
        assert_eq!(vote[0], 0x08);
        assert_eq!(vote[1], 62);
        let vote_text = String::from_utf8_lossy(&vote);
        assert!(vote_text.contains(PARENT_CORE));
        match gov_vote_yes_any_msg(PARENT_CORE, 62) {
            CosmosMsg::Any(any) => assert_eq!(any.type_url, "/cosmos.gov.v1.MsgVote"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn gov_deposit_spec_is_proposal_and_coin() {
        let spec = parse_gov_deposit("61:5000000000uterp").unwrap();
        assert_eq!(spec.proposal_id, 61);
        assert_eq!(spec.amount, Uint256::from(HALF_MIN_DEPOSIT_UTERP));
        assert_eq!(spec.denom, "uterp");
        assert!(parse_gov_deposit("60").is_err());
        assert!(parse_gov_deposit("60:uterp").is_err());
    }

    #[test]
    fn million_tokens_is_one_million_whole() {
        assert_eq!(million_tokens(), Uint256::from(1_000_000_000_000u128));
        assert_eq!(terp_100k(), Uint256::from(100_000_000_000u128));
    }
}
