//! Next-block stake move that puts one operator strictly above one third.
//!
//! [`plan_share_rebalance`] is the later foundation rebalance: empty one
//! validator into another, then shed a hub operator down to a share of the
//! bonded pool. A validator that is still the destination of a redelegation
//! cannot be a source until that entry completes, so those moves are deferred.
//!
//! Redelegate existing stake first. Delegate liquid tokens only for the
//! remainder. A redelegation does not change total bonded tokens, so voting
//! power moves on the next block. A new delegation increases the total.
//! Create-validator is the previous block and a different signer. Count its
//! self-bond with [`QuorumSnapshot::with_new_validator_self_bond`] before
//! planning. At most one `MsgBeginRedelegate` per source validator.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Foundation wallet that already holds the stake we can redelegate.
pub const FOUNDATION_DELEGATOR: &str = "terp1qfpnat6kc99gsc3233cw6fq0s0fdv7cq06alqu";

/// Foundation DAO core. The hot-wallet send lands here.
pub const FOUNDATION_DAO: &str = super::native_delegation::PARENT_CORE;

/// 36 million TERP, in whole tokens, moved off the hot wallet in the same tx.
pub const DAO_SEND_TERP: u128 = 36_000_000;

/// Gas units budgeted for one `MsgSend`, before headroom.
const GAS_PER_SEND: u64 = 200_000;
/// Gas units budgeted for one `MsgBeginRedelegate`, before headroom.
const GAS_PER_REDELEGATE: u64 = 500_000;
/// Gas units budgeted for one `MsgDelegate`, before headroom.
const GAS_PER_DELEGATE: u64 = 400_000;
/// Signature and tx overhead, before headroom.
const GAS_TX_OVERHEAD: u64 = 200_000;

/// Default self-bond on the create-validator template: 1 TERP.
pub const DEFAULT_SELF_BOND: u128 = 1_000_000;

/// 6-decimal micro units. Same scale as `native_delegation::MICRO`.
pub const MICRO: u128 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakeSource {
    pub validator: String,
    #[serde(with = "u128_json")]
    pub redelegable: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumSnapshot {
    pub delegator: String,
    pub dest_operator: String,
    /// Bonded tokens already on `dest_operator`. Zero when it is not bonded yet.
    #[serde(with = "u128_json")]
    pub dest_bonded: u128,
    #[serde(with = "u128_json")]
    pub total_bonded: u128,
    pub sources: Vec<StakeSource>,
    #[serde(with = "u128_json")]
    pub liquid: u128,
    pub denom: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankSend {
    pub to: String,
    #[serde(with = "u128_json")]
    pub amount: u128,
    pub denom: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redelegate {
    pub src: String,
    #[serde(with = "u128_json")]
    pub amount: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumPlan {
    /// At most one entry per source, largest source first.
    pub redelegates: Vec<Redelegate>,
    /// Liquid delegation to `dest`. Zero when redelegation is enough.
    #[serde(with = "u128_json")]
    pub delegate: u128,
    #[serde(with = "u128_json")]
    pub dest_after: u128,
    #[serde(with = "u128_json")]
    pub total_after: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuorumError {
    EmptyDest,
    Overflow,
    Short {
        dest_after: u128,
        total_after: u128,
        liquid: u128,
    },
    SendExceedsLiquid {
        amount: u128,
        liquid: u128,
    },
    UnknownOperator(String),
    BadTarget,
}

impl std::fmt::Display for QuorumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyDest => write!(f, "destination operator is empty"),
            Self::Overflow => write!(f, "stake amount overflowed u128"),
            Self::SendExceedsLiquid { amount, liquid } => write!(
                f,
                "bank send {amount} exceeds liquid {liquid}"
            ),
            Self::UnknownOperator(op) => write!(f, "operator {op} is not in the book"),
            Self::BadTarget => write!(f, "target share must be between 1 and 10000 basis points"),
            Self::Short {
                dest_after,
                total_after,
                liquid,
            } => write!(
                f,
                "short of one third: dest {dest_after} of total {total_after} after liquid {liquid} (need dest * 3 > total)"
            ),
        }
    }
}

impl std::error::Error for QuorumError {}

impl QuorumSnapshot {
    /// Hold `amount` of liquid back for the bank send that prefixes the tx.
    /// The redelegation planner then cannot also delegate those tokens.
    pub fn reserve_bank_send(&mut self, amount: u128) -> Result<(), QuorumError> {
        if amount > self.liquid {
            return Err(QuorumError::SendExceedsLiquid {
                amount,
                liquid: self.liquid,
            });
        }
        self.liquid -= amount;
        Ok(())
    }

    /// Create-validator lands in the previous block. Add that self-bond to
    /// the destination and to the bonded total before planning.
    pub fn with_new_validator_self_bond(mut self, self_bond: u128) -> Self {
        self.dest_bonded = self.dest_bonded.saturating_add(self_bond);
        self.total_bonded = self.total_bonded.saturating_add(self_bond);
        self
    }
}

impl QuorumPlan {
    pub fn above_one_third(&self) -> bool {
        above_one_third(self.dest_after, self.total_after)
    }
}

/// Strict one third. An exact third does not pass.
pub fn above_one_third(dest: u128, total: u128) -> bool {
    dest.saturating_mul(3) > total
}

/// Subtract in-flight redelegation balances from each source. Entries that
/// fall to zero are dropped. Same-source locks add together.
pub fn sources_after_locks(
    mut sources: Vec<StakeSource>,
    locks: impl IntoIterator<Item = (String, u128)>,
) -> Vec<StakeSource> {
    for (src, amt) in locks {
        if let Some(row) = sources.iter_mut().find(|s| s.validator == src) {
            row.redelegable = row.redelegable.saturating_sub(amt);
        }
    }
    sources.retain(|s| s.redelegable > 0 && !s.validator.is_empty());
    sources
}

/// Smallest redelegate-then-delegate plan that puts `dest` strictly above
/// one third of the bonded set on the next block.
pub fn plan_next_block(snap: &QuorumSnapshot) -> Result<QuorumPlan, QuorumError> {
    if snap.dest_operator.is_empty() {
        return Err(QuorumError::EmptyDest);
    }
    let dest = snap.dest_bonded;
    let total = snap.total_bonded;
    if above_one_third(dest, total) {
        return Ok(QuorumPlan {
            redelegates: Vec::new(),
            delegate: 0,
            dest_after: dest,
            total_after: total,
        });
    }

    let sources = coalesce_sources(&snap.dest_operator, &snap.sources);
    let r_min = amount_to_clear(dest, total, 3)?;
    let mut redelegates = Vec::new();
    let mut remaining = r_min;
    for src in &sources {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(src.redelegable);
        if take == 0 {
            continue;
        }
        redelegates.push(Redelegate {
            src: src.validator.clone(),
            amount: take,
        });
        remaining -= take;
    }
    let moved = sum_amounts(redelegates.iter().map(|r| r.amount))?;
    let dest_after_move = dest.checked_add(moved).ok_or(QuorumError::Overflow)?;
    if remaining == 0 {
        let plan = QuorumPlan {
            redelegates,
            delegate: 0,
            dest_after: dest_after_move,
            total_after: total,
        };
        debug_assert!(plan.above_one_third());
        return Ok(plan);
    }

    // Every source is already in `redelegates`. Liquid has to close the rest,
    // and it increases total bonded as well as the destination.
    let liquid_needed = amount_to_clear(dest_after_move, total, 2)?;
    if liquid_needed > snap.liquid {
        let dest_after = dest_after_move
            .checked_add(snap.liquid)
            .ok_or(QuorumError::Overflow)?;
        let total_after = total
            .checked_add(snap.liquid)
            .ok_or(QuorumError::Overflow)?;
        return Err(QuorumError::Short {
            dest_after,
            total_after,
            liquid: snap.liquid,
        });
    }
    let plan = QuorumPlan {
        redelegates,
        delegate: liquid_needed,
        dest_after: dest_after_move
            .checked_add(liquid_needed)
            .ok_or(QuorumError::Overflow)?,
        total_after: total
            .checked_add(liquid_needed)
            .ok_or(QuorumError::Overflow)?,
    };
    debug_assert!(plan.above_one_third());
    Ok(plan)
}

/// Explicit gas limit for the whole next-block transaction.
///
/// Doubles a per-message budget so a merged `MsgSend` plus redelegations
/// does not run out of gas. `--gas auto` on one generate-only body only
/// simulates that one message.
pub fn next_block_gas(send: bool, plan: &QuorumPlan) -> u64 {
    let mut raw = GAS_TX_OVERHEAD;
    if send {
        raw = raw.saturating_add(GAS_PER_SEND);
    }
    raw = raw.saturating_add(GAS_PER_REDELEGATE.saturating_mul(plan.redelegates.len() as u64));
    if plan.delegate > 0 {
        raw = raw.saturating_add(GAS_PER_DELEGATE);
    }
    raw.saturating_mul(2)
}

/// Fee coin for `gas` at `gas_prices` (`0.05uthiol`). Rounds up.
pub fn fee_for_gas(gas: u64, gas_prices: &str) -> Result<(u128, String), String> {
    let (coeff, scale, denom) = parse_gas_price(gas_prices)?;
    let product = u128::from(gas)
        .checked_mul(coeff)
        .ok_or_else(|| "fee overflow".to_string())?;
    let mut amount = product / scale;
    if product % scale != 0 {
        amount = amount.saturating_add(1);
    }
    Ok((amount, denom))
}

/// Unsigned tx the operator signs. Fee and gas cover every message.
pub fn unsigned_tx(
    messages: &[serde_json::Value],
    gas: u64,
    fee_amount: u128,
    fee_denom: &str,
) -> serde_json::Value {
    serde_json::json!({
        "body": {
            "messages": messages,
            "memo": "",
            "timeout_height": "0",
            "extension_options": [],
            "non_critical_extension_options": []
        },
        "auth_info": {
            "signer_infos": [],
            "fee": {
                "amount": [{"denom": fee_denom, "amount": fee_amount.to_string()}],
                "gas_limit": gas.to_string(),
                "payer": "",
                "granter": ""
            }
        },
        "signatures": []
    })
}

/// `terpd` arguments for the next-block transaction. No `--yes`, no node URL.
/// More than one message is `--generate-only` so the commands are not separate
/// broadcasts. Every command carries the combined gas limit. Sign `unsigned_tx`.
pub fn next_block_commands(
    chain_id: &str,
    gas_prices: &str,
    from: &str,
    dest: &str,
    denom: &str,
    send: Option<&BankSend>,
    plan: &QuorumPlan,
) -> Vec<Vec<String>> {
    let gas = next_block_gas(send.is_some(), plan).to_string();
    let mut cmds = Vec::new();
    if let Some(send) = send {
        cmds.push(bank_cmd(
            from,
            &send.to,
            &format!("{}{}", send.amount, send.denom),
            chain_id,
            gas_prices,
            &gas,
        ));
    }
    for redelegate in &plan.redelegates {
        cmds.push(staking_cmd(
            "redelegate",
            &[
                redelegate.src.as_str(),
                dest,
                &format!("{}{denom}", redelegate.amount),
            ],
            chain_id,
            gas_prices,
            from,
            &gas,
        ));
    }
    if plan.delegate > 0 {
        cmds.push(staking_cmd(
            "delegate",
            &[dest, &format!("{}{denom}", plan.delegate)],
            chain_id,
            gas_prices,
            from,
            &gas,
        ));
    }
    if cmds.len() > 1 {
        for cmd in &mut cmds {
            cmd.push("--generate-only".into());
        }
    }
    cmds
}

pub fn next_block_messages(
    delegator: &str,
    dest: &str,
    denom: &str,
    send: Option<&BankSend>,
    plan: &QuorumPlan,
) -> Vec<serde_json::Value> {
    let mut msgs = Vec::new();
    if let Some(send) = send {
        msgs.push(serde_json::json!({
            "@type": "/cosmos.bank.v1beta1.MsgSend",
            "from_address": delegator,
            "to_address": send.to,
            "amount": [{"denom": send.denom, "amount": send.amount.to_string()}],
        }));
    }
    for redelegate in &plan.redelegates {
        msgs.push(serde_json::json!({
            "@type": "/cosmos.staking.v1beta1.MsgBeginRedelegate",
            "delegator_address": delegator,
            "validator_src_address": redelegate.src,
            "validator_dst_address": dest,
            "amount": {"denom": denom, "amount": redelegate.amount.to_string()},
        }));
    }
    if plan.delegate > 0 {
        msgs.push(serde_json::json!({
            "@type": "/cosmos.staking.v1beta1.MsgDelegate",
            "delegator_address": delegator,
            "validator_address": dest,
            "amount": {"denom": denom, "amount": plan.delegate.to_string()},
        }));
    }
    msgs
}

/// Consensus pubkey of the mac node `cbbb3d490dc9300a92c12c7eef980ceb5d508747`.
pub const TERPZ_CONSENSUS_PUBKEY: &str = "46pUjSo5BbUrJyA+x1Rt+u45TfT1xFWh/QwJpO77lDc=";

pub const TERPZ_MONIKER: &str = "terpz";

/// terpz operator. Voting-power target of the foundation rebalance.
pub const TERPZ_OPERATOR: &str = "terpvaloper1fxccvvhhy43tvet2ah7jqwq4cwl9k3dx23tm5m";

/// AlxVoy. Receives every foundation share currently on SerGo.
pub const ALXVOY_OPERATOR: &str = "terpvaloper126jdyc74p70ahzrf3wvaka7hmeke8g2ntym70p";

/// SerGo. Jailed and unbonding; foundation stake here is redelegated away.
pub const SERGO_OPERATOR: &str = "terpvaloper1mzns2fp4urelqxyjxyjelef34j9eynu8ffd9vt";

/// 22.00% of bonded tokens.
pub const TERPZ_TARGET_BPS: u32 = 2_200;

pub const TERPZ_DETAILS: &str = "terpenes are volatile, so is this node";

/// P2P node id of the mac process. This is not an account key.
pub const TERPZ_NODE_ID: &str = "cbbb3d490dc9300a92c12c7eef980ceb5d508747";

/// Address written into validator net info and the create-validator memo.
/// Loopback, so the published endpoint is not this machine's LAN address.
pub const TERPZ_ADVERTISED_P2P: &str = "127.0.0.1:26656";

/// Account mnemonic that signs `MsgCreateValidator`. Not the consensus key.
pub const OPERATOR_MNEMONIC_ENV: &str = "TERPZ_OPERATOR_MNEMONIC";

/// Account mnemonic that signs the foundation send and redelegations.
pub const FOUNDATION_MNEMONIC_ENV: &str = "TERPZ_FOUNDATION_MNEMONIC";

/// Explicit gas for the single create-validator message.
pub const CREATE_VALIDATOR_GAS: u64 = 1_000_000;

/// `node-id@127.0.0.1:26656`. Peers and explorers that show the tx memo see loopback.
pub fn validator_advertised_memo(node_id: &str) -> String {
    format!("{node_id}@{TERPZ_ADVERTISED_P2P}")
}

/// `KEY=VALUE` lines from a dotenv file. Quoted values are unwrapped.
/// Blank lines and `#` comments are skipped.
pub fn dotenv_assignments(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_start_matches("export ").trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        let value = if value.len() >= 2 {
            let bytes = value.as_bytes();
            if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
            {
                &value[1..value.len() - 1]
            } else {
                value
            }
        } else {
            value
        };
        out.push((key.to_string(), value.to_string()));
    }
    out
}

pub fn dotenv_value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
}

/// Create-validator carries the operator valoper. The next-block tx carries
/// the foundation account on every message and the same operator as destination.
pub fn assert_phase_signers(
    create_unsigned: &serde_json::Value,
    next_unsigned: &serde_json::Value,
    operator_valoper: &str,
    foundation: &str,
) -> Result<(), String> {
    let create_msg = create_unsigned
        .pointer("/body/messages/0")
        .ok_or_else(|| "create-validator tx has no message".to_string())?;
    let got_op = create_msg
        .get("validator_address")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if got_op != operator_valoper {
        return Err(format!(
            "create-validator signer {operator_valoper} does not match tx validator {got_op}"
        ));
    }
    let memo = create_unsigned
        .pointer("/body/memo")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expected_memo = validator_advertised_memo(TERPZ_NODE_ID);
    if memo != expected_memo {
        return Err(format!(
            "create-validator memo must be {expected_memo} so net info stays on loopback"
        ));
    }
    let messages = next_unsigned
        .pointer("/body/messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "next-block tx has no messages".to_string())?;
    if messages.is_empty() {
        return Err("next-block tx has no messages".into());
    }
    for msg in messages {
        let kind = msg.get("@type").and_then(|v| v.as_str()).unwrap_or("");
        if kind.ends_with("MsgSend") {
            let from = msg
                .get("from_address")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if from != foundation {
                return Err(format!("bank send signer {from} is not {foundation}"));
            }
        } else if kind.ends_with("MsgBeginRedelegate") || kind.ends_with("MsgDelegate") {
            let from = msg
                .get("delegator_address")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if from != foundation {
                return Err(format!("{kind} signer {from} is not {foundation}"));
            }
            let dest = msg
                .get("validator_dst_address")
                .or_else(|| msg.get("validator_address"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if dest != operator_valoper {
                return Err(format!(
                    "{kind} destination {dest} is not operator {operator_valoper}"
                ));
            }
        } else {
            return Err(format!("unexpected message in the next-block tx: {kind}"));
        }
    }
    Ok(())
}

/// CLI file for `terpd tx staking create-validator`. Strict field set.
/// The signer is the new operator, not the foundation delegator.
pub fn create_validator_file(
    moniker: &str,
    details: &str,
    pubkey_base64: &str,
    self_bond: u128,
    denom: &str,
) -> serde_json::Value {
    serde_json::json!({
        "pubkey": {"@type": "/cosmos.crypto.ed25519.PubKey", "key": pubkey_base64},
        "amount": format!("{self_bond}{denom}"),
        "moniker": moniker,
        "identity": "",
        "website": "",
        "security": "",
        "details": details,
        "commission-rate": "0.100000000000000000",
        "commission-max-rate": "0.200000000000000000",
        "commission-max-change-rate": "0.010000000000000000",
        "min-self-delegation": "1",
    })
}

pub fn encode_msg_begin_redelegate(
    delegator: &str,
    src: &str,
    dest: &str,
    amount: u128,
    denom: &str,
) -> Vec<u8> {
    let mut coin = Vec::new();
    proto_string(&mut coin, 1, denom);
    proto_string(&mut coin, 2, &amount.to_string());
    let mut msg = Vec::new();
    proto_string(&mut msg, 1, delegator);
    proto_string(&mut msg, 2, src);
    proto_string(&mut msg, 3, dest);
    proto_ld(&mut msg, 4, &coin);
    msg
}

pub fn encode_msg_delegate(delegator: &str, validator: &str, amount: u128, denom: &str) -> Vec<u8> {
    let mut coin = Vec::new();
    proto_string(&mut coin, 1, denom);
    proto_string(&mut coin, 2, &amount.to_string());
    let mut msg = Vec::new();
    proto_string(&mut msg, 1, delegator);
    proto_string(&mut msg, 2, validator);
    proto_ld(&mut msg, 3, &coin);
    msg
}

fn bank_cmd(
    from: &str,
    to: &str,
    amount: &str,
    chain_id: &str,
    gas_prices: &str,
    gas: &str,
) -> Vec<String> {
    let mut cmd = vec![
        "terpd".into(),
        "tx".into(),
        "bank".into(),
        "send".into(),
        from.into(),
        to.into(),
        amount.into(),
    ];
    cmd.extend(tx_flags(from, chain_id, gas_prices, gas));
    cmd
}

fn staking_cmd(
    sub: &str,
    positional: &[&str],
    chain_id: &str,
    gas_prices: &str,
    from: &str,
    gas: &str,
) -> Vec<String> {
    let mut cmd = vec!["terpd".into(), "tx".into(), "staking".into(), sub.into()];
    cmd.extend(positional.iter().map(|s| (*s).to_string()));
    cmd.extend(tx_flags(from, chain_id, gas_prices, gas));
    cmd
}

fn tx_flags(from: &str, chain_id: &str, gas_prices: &str, gas: &str) -> Vec<String> {
    vec![
        "--from".into(),
        from.into(),
        "--chain-id".into(),
        chain_id.into(),
        "--gas".into(),
        gas.into(),
        "--gas-prices".into(),
        gas_prices.into(),
    ]
}

fn parse_gas_price(price: &str) -> Result<(u128, u128, String), String> {
    let split = price
        .find(|c: char| c.is_ascii_alphabetic())
        .ok_or_else(|| format!("gas price {price} has no denom"))?;
    let (coeff, denom) = price.split_at(split);
    if denom.is_empty() || coeff.is_empty() {
        return Err(format!("gas price {price} is incomplete"));
    }
    let (num, scale) = if let Some((whole, frac)) = coeff.split_once('.') {
        if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("gas price {price} has a bad fraction"));
        }
        let scale = 10u128
            .checked_pow(frac.len() as u32)
            .ok_or_else(|| "gas price scale overflow".to_string())?;
        let whole: u128 = whole
            .parse()
            .map_err(|_| format!("gas price {price} has a bad coefficient"))?;
        let frac: u128 = frac
            .parse()
            .map_err(|_| format!("gas price {price} has a bad fraction"))?;
        (
            whole
                .checked_mul(scale)
                .and_then(|w| w.checked_add(frac))
                .ok_or_else(|| "gas price overflow".to_string())?,
            scale,
        )
    } else {
        (
            coeff
                .parse()
                .map_err(|_| format!("gas price {price} has a bad coefficient"))?,
            1u128,
        )
    };
    Ok((num, scale, denom.to_string()))
}

/// Smallest `x` such that `3 * (base + x) > total` when `divisor` is 3
/// (redelegate: total stays put), or `3 * base + 2 * x > total` when
/// `divisor` is 2 (delegate: total grows by `x`). An exact third still
/// needs one unit.
fn amount_to_clear(base: u128, total: u128, divisor: u128) -> Result<u128, QuorumError> {
    let triple = base.checked_mul(3).ok_or(QuorumError::Overflow)?;
    if triple > total {
        return Ok(0);
    }
    let gap = total - triple;
    (gap / divisor).checked_add(1).ok_or(QuorumError::Overflow)
}

fn sum_amounts(amounts: impl Iterator<Item = u128>) -> Result<u128, QuorumError> {
    amounts.fold(Ok(0u128), |acc, n| {
        acc.and_then(|a| a.checked_add(n).ok_or(QuorumError::Overflow))
    })
}

fn coalesce_sources(dest: &str, sources: &[StakeSource]) -> Vec<StakeSource> {
    let mut merged: Vec<StakeSource> = Vec::new();
    for src in sources {
        if src.validator.is_empty() || src.validator == dest || src.redelegable == 0 {
            continue;
        }
        if let Some(row) = merged.iter_mut().find(|s| s.validator == src.validator) {
            row.redelegable = row.redelegable.saturating_add(src.redelegable);
        } else {
            merged.push(src.clone());
        }
    }
    merged.sort_by(|a, b| {
        b.redelegable
            .cmp(&a.redelegable)
            .then_with(|| a.validator.cmp(&b.validator))
    });
    merged
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorRow {
    pub operator: String,
    pub moniker: String,
    #[serde(with = "u128_json")]
    pub tokens: u128,
    pub bonded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveRedelegation {
    pub src: String,
    pub dst: String,
    #[serde(with = "u128_json")]
    pub balance: u128,
    pub completion_time: String,
}

/// Foundation delegations plus the bonded set they move across.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundationBook {
    pub delegator: String,
    pub denom: String,
    #[serde(with = "u128_json")]
    pub total_bonded: u128,
    pub operators: Vec<OperatorRow>,
    pub delegations: Vec<StakeSource>,
    pub redelegations: Vec<ActiveRedelegation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakeMove {
    pub src: String,
    pub dst: String,
    #[serde(with = "u128_json")]
    pub amount: u128,
}

/// `ready` can be signed on the next block. `deferred` waits until `deferred_until`,
/// because that source is still the destination of a redelegation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebalancePlan {
    pub ready: Vec<StakeMove>,
    pub deferred: Vec<StakeMove>,
    pub deferred_until: String,
    #[serde(with = "u128_json")]
    pub total_after_ready: u128,
    #[serde(with = "u128_json")]
    pub hub_before: u128,
    #[serde(with = "u128_json")]
    pub hub_after_ready: u128,
    #[serde(with = "u128_json")]
    pub hub_after: u128,
    #[serde(with = "u128_json")]
    pub total_after: u128,
    pub target_bps: u32,
}

/// Empty `empty_src` into `empty_dst`, then shed `hub` down to `target_bps`
/// of the bonded pool. Outflow is split evenly across the other bonded
/// operators, one extra uterp to the lowest address while the remainder lasts.
///
/// A redelegation from an unbonded source onto a bonded destination increases
/// `total_bonded` by that amount. Moves whose source still has a receiving
/// redelegation are deferred rather than dropped.
pub fn plan_share_rebalance(
    book: &FoundationBook,
    hub: &str,
    empty_src: &str,
    empty_dst: &str,
    target_bps: u32,
) -> Result<RebalancePlan, QuorumError> {
    if hub.is_empty() || empty_dst.is_empty() {
        return Err(QuorumError::EmptyDest);
    }
    if target_bps == 0 || target_bps > 10_000 {
        return Err(QuorumError::BadTarget);
    }
    let hub_before = operator_tokens(book, hub)?;
    let mut intended = Vec::new();
    let empty_amt = delegation_of(book, empty_src);
    if !empty_src.is_empty() && empty_src != empty_dst && empty_amt > 0 {
        intended.push(StakeMove {
            src: empty_src.to_string(),
            dst: empty_dst.to_string(),
            amount: empty_amt,
        });
    }
    let total_mid = projected_total(book, &intended)?;
    let hub_mid = apply_moves(hub_before, hub, &intended)?;
    let target = total_mid
        .checked_mul(u128::from(target_bps))
        .ok_or(QuorumError::Overflow)?
        / 10_000;
    let excess = hub_mid.saturating_sub(target);
    let on_hub = delegation_of(book, hub);
    if excess > on_hub {
        return Err(QuorumError::Short {
            dest_after: hub_mid.saturating_sub(on_hub),
            total_after: total_mid,
            liquid: 0,
        });
    }
    if excess > 0 {
        let dests = bonded_except(book, hub);
        if dests.is_empty() {
            return Err(QuorumError::Short {
                dest_after: hub_mid,
                total_after: total_mid,
                liquid: 0,
            });
        }
        for (dst, amount) in split_even(excess, &dests) {
            intended.push(StakeMove {
                src: hub.to_string(),
                dst,
                amount,
            });
        }
    }

    let mut ready = Vec::new();
    let mut deferred = Vec::new();
    let mut deferred_until = String::new();
    for mv in intended {
        if source_blocked(book, &mv.src) {
            if let Some(time) = latest_completion(book, &mv.src) {
                if time.as_str() > deferred_until.as_str() {
                    deferred_until = time;
                }
            }
            deferred.push(mv);
        } else {
            ready.push(mv);
        }
    }
    let hub_after_ready = apply_moves(hub_before, hub, &ready)?;
    let total_after_ready = projected_total(book, &ready)?;
    let mut all = ready.clone();
    all.extend(deferred.iter().cloned());
    Ok(RebalancePlan {
        ready,
        deferred,
        deferred_until,
        total_after_ready,
        hub_before,
        hub_after_ready,
        hub_after: apply_moves(hub_before, hub, &all)?,
        total_after: projected_total(book, &all)?,
        target_bps,
    })
}

/// Redelegations this cut will sign. The hub share design stays on the plan
/// and is not included, even when those moves are already unlocked.
pub fn executable_rebalance(plan: &RebalancePlan, empty_src: &str) -> Vec<StakeMove> {
    plan.ready
        .iter()
        .filter(|mv| mv.src == empty_src)
        .cloned()
        .collect()
}

/// Hub outflow kept for a later cut. Not an unsigned transaction.
pub fn hub_share_design(plan: &RebalancePlan, hub: &str) -> Vec<StakeMove> {
    plan.ready
        .iter()
        .chain(plan.deferred.iter())
        .filter(|mv| mv.src == hub)
        .cloned()
        .collect()
}

pub fn stake_move_messages(
    delegator: &str,
    denom: &str,
    moves: &[StakeMove],
) -> Vec<serde_json::Value> {
    moves
        .iter()
        .map(|mv| {
            serde_json::json!({
                "@type": "/cosmos.staking.v1beta1.MsgBeginRedelegate",
                "delegator_address": delegator,
                "validator_src_address": mv.src,
                "validator_dst_address": mv.dst,
                "amount": {"denom": denom, "amount": mv.amount.to_string()},
            })
        })
        .collect()
}

pub fn stake_move_commands(
    chain_id: &str,
    gas_prices: &str,
    from: &str,
    denom: &str,
    moves: &[StakeMove],
) -> Vec<Vec<String>> {
    let plan = QuorumPlan {
        redelegates: moves
            .iter()
            .map(|mv| Redelegate {
                src: mv.src.clone(),
                amount: mv.amount,
            })
            .collect(),
        delegate: 0,
        dest_after: 0,
        total_after: 0,
    };
    let gas = next_block_gas(false, &plan).to_string();
    let mut cmds = Vec::new();
    for mv in moves {
        cmds.push(staking_cmd(
            "redelegate",
            &[
                mv.src.as_str(),
                mv.dst.as_str(),
                &format!("{}{denom}", mv.amount),
            ],
            chain_id,
            gas_prices,
            from,
            &gas,
        ));
    }
    if cmds.len() > 1 {
        for cmd in &mut cmds {
            cmd.push("--generate-only".into());
        }
    }
    cmds
}

fn operator_tokens(book: &FoundationBook, operator: &str) -> Result<u128, QuorumError> {
    book.operators
        .iter()
        .find(|row| row.operator == operator)
        .map(|row| row.tokens)
        .ok_or_else(|| QuorumError::UnknownOperator(operator.to_string()))
}

fn delegation_of(book: &FoundationBook, operator: &str) -> u128 {
    book.delegations
        .iter()
        .filter(|row| row.validator == operator)
        .map(|row| row.redelegable)
        .fold(0u128, u128::saturating_add)
}

fn is_bonded(book: &FoundationBook, operator: &str) -> bool {
    book.operators
        .iter()
        .find(|row| row.operator == operator)
        .is_some_and(|row| row.bonded)
}

fn bonded_except(book: &FoundationBook, hub: &str) -> Vec<String> {
    let mut dests: Vec<String> = book
        .operators
        .iter()
        .filter(|row| row.bonded && row.operator != hub)
        .map(|row| row.operator.clone())
        .collect();
    dests.sort();
    dests.dedup();
    dests
}

fn source_blocked(book: &FoundationBook, operator: &str) -> bool {
    book.redelegations
        .iter()
        .any(|entry| entry.dst == operator && entry.balance > 0)
}

fn latest_completion(book: &FoundationBook, operator: &str) -> Option<String> {
    book.redelegations
        .iter()
        .filter(|entry| entry.dst == operator && entry.balance > 0)
        .map(|entry| entry.completion_time.clone())
        .max()
}

fn projected_total(book: &FoundationBook, moves: &[StakeMove]) -> Result<u128, QuorumError> {
    let mut total = book.total_bonded;
    for mv in moves {
        match (is_bonded(book, &mv.src), is_bonded(book, &mv.dst)) {
            (false, true) => {
                total = total.checked_add(mv.amount).ok_or(QuorumError::Overflow)?;
            }
            (true, false) => {
                total = total.checked_sub(mv.amount).ok_or(QuorumError::Overflow)?;
            }
            _ => {}
        }
    }
    Ok(total)
}

fn apply_moves(start: u128, operator: &str, moves: &[StakeMove]) -> Result<u128, QuorumError> {
    let mut tokens = start;
    for mv in moves {
        if mv.src == operator {
            tokens = tokens.checked_sub(mv.amount).ok_or(QuorumError::Overflow)?;
        }
        if mv.dst == operator {
            tokens = tokens.checked_add(mv.amount).ok_or(QuorumError::Overflow)?;
        }
    }
    Ok(tokens)
}

fn split_even(amount: u128, dests: &[String]) -> Vec<(String, u128)> {
    let n = dests.len() as u128;
    if n == 0 || amount == 0 {
        return Vec::new();
    }
    let base = amount / n;
    let mut rem = amount % n;
    let mut out = Vec::new();
    for dst in dests {
        let mut share = base;
        if rem > 0 {
            share += 1;
            rem -= 1;
        }
        if share > 0 {
            out.push((dst.clone(), share));
        }
    }
    out
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

mod u128_json {
    use super::*;

    pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        match value {
            serde_json::Value::String(text) => {
                text.parse::<u128>().map_err(serde::de::Error::custom)
            }
            serde_json::Value::Number(n) => n
                .to_string()
                .parse::<u128>()
                .map_err(serde::de::Error::custom),
            other => Err(serde::de::Error::custom(format!(
                "expected u128 string or number, got {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(
        dest_bonded: u128,
        total: u128,
        sources: Vec<(&str, u128)>,
        liquid: u128,
    ) -> QuorumSnapshot {
        QuorumSnapshot {
            delegator: FOUNDATION_DELEGATOR.into(),
            dest_operator: "dest".into(),
            dest_bonded,
            total_bonded: total,
            sources: sources
                .into_iter()
                .map(|(validator, redelegable)| StakeSource {
                    validator: validator.into(),
                    redelegable,
                })
                .collect(),
            liquid,
            denom: "uterp".into(),
        }
    }

    #[test]
    fn already_safe_is_empty() {
        let plan = plan_next_block(&snap(10, 20, vec![("a", 5)], 5)).unwrap();
        assert!(plan.redelegates.is_empty());
        assert_eq!(plan.delegate, 0);
        assert!(plan.above_one_third());
    }

    #[test]
    fn exact_third_needs_one_uterp() {
        let plan = plan_next_block(&snap(5, 15, vec![("a", 10)], 0)).unwrap();
        assert_eq!(plan.redelegates.len(), 1);
        assert_eq!(plan.redelegates[0].amount, 1);
        assert_eq!(plan.delegate, 0);
        assert_eq!(plan.dest_after, 6);
        assert_eq!(plan.total_after, 15);
        assert!(plan.above_one_third());
    }

    #[test]
    fn two_sources_when_the_largest_is_short() {
        // gap = 10, redelegate min = 4. Sources 3 and 3, largest-first then name.
        let plan = plan_next_block(&snap(0, 10, vec![("b", 3), ("a", 3)], 0)).unwrap();
        assert_eq!(plan.redelegates.len(), 2);
        assert_eq!(plan.redelegates[0].src, "a");
        assert_eq!(plan.redelegates[0].amount, 3);
        assert_eq!(plan.redelegates[1].src, "b");
        assert_eq!(plan.redelegates[1].amount, 1);
        assert_eq!(plan.delegate, 0);
        assert_eq!(plan.dest_after, 4);
        assert!(plan.above_one_third());
    }

    #[test]
    fn duplicate_source_is_one_message() {
        let plan = plan_next_block(&snap(5, 15, vec![("a", 1), ("a", 4)], 0)).unwrap();
        assert_eq!(plan.redelegates.len(), 1);
        assert_eq!(plan.redelegates[0].src, "a");
        assert_eq!(plan.redelegates[0].amount, 1);
    }

    #[test]
    fn liquid_only_for_the_remainder() {
        // Redelegate 2 of the 4 required, then liquid. 2*L > 10 - 6, so L = 3.
        let plan = plan_next_block(&snap(0, 10, vec![("a", 2)], 100)).unwrap();
        assert_eq!(plan.redelegates.len(), 1);
        assert_eq!(plan.redelegates[0].amount, 2);
        assert_eq!(plan.delegate, 3);
        assert_eq!(plan.dest_after, 5);
        assert_eq!(plan.total_after, 13);
        assert!(5 * 3 > 13);
    }

    #[test]
    fn shortfall_names_the_gap() {
        let err = plan_next_block(&snap(0, 10, vec![("a", 2)], 2)).unwrap_err();
        match err {
            QuorumError::Short {
                dest_after,
                total_after,
                liquid,
            } => {
                assert_eq!(dest_after, 4);
                assert_eq!(total_after, 12);
                assert_eq!(liquid, 2);
                assert!(dest_after * 3 <= total_after);
            }
            other => panic!("expected shortfall, got {other:?}"),
        }
    }

    #[test]
    fn dest_is_not_a_source() {
        let plan = plan_next_block(&snap(5, 15, vec![("dest", 100), ("a", 9)], 0)).unwrap();
        assert_eq!(plan.redelegates.len(), 1);
        assert_eq!(plan.redelegates[0].src, "a");
        assert_eq!(plan.redelegates[0].amount, 1);
    }

    #[test]
    fn empty_dest_fails() {
        let mut snapshot = snap(1, 3, vec![], 0);
        snapshot.dest_operator.clear();
        assert_eq!(
            plan_next_block(&snapshot).unwrap_err(),
            QuorumError::EmptyDest
        );
    }

    #[test]
    fn self_bond_counts_on_dest_and_total() {
        let snapshot = snap(0, 15, vec![("a", 10)], 0).with_new_validator_self_bond(5);
        assert_eq!(snapshot.dest_bonded, 5);
        assert_eq!(snapshot.total_bonded, 20);
        // 5 of 20 is under a third. One uterp of redelegate is not enough:
        // (5+R)*3 > 20 => R > 5/3 => R = 2.
        let plan = plan_next_block(&snapshot).unwrap();
        assert_eq!(plan.redelegates[0].amount, 2);
        assert_eq!(plan.dest_after, 7);
        assert_eq!(plan.total_after, 20);
        assert!(plan.above_one_third());
    }

    #[test]
    fn six_million_redelegate_clears_without_liquid() {
        let six_m = 6_000_000 * MICRO;
        let one_terp = MICRO;
        let total = 10_000_000 * MICRO + one_terp;
        let snapshot = QuorumSnapshot {
            delegator: FOUNDATION_DELEGATOR.into(),
            dest_operator: "terpvaloper1patched".into(),
            dest_bonded: 0,
            total_bonded: 10_000_000 * MICRO,
            sources: vec![StakeSource {
                validator: "terpvaloper1source".into(),
                redelegable: six_m,
            }],
            liquid: 1_000_000 * MICRO,
            denom: "uterp".into(),
        }
        .with_new_validator_self_bond(one_terp);
        assert_eq!(snapshot.total_bonded, total);
        let plan = plan_next_block(&snapshot).unwrap();
        assert_eq!(plan.delegate, 0);
        assert_eq!(plan.redelegates.len(), 1);
        assert!(plan.redelegates[0].amount < six_m);
        assert!(plan.above_one_third());
        assert!(plan.dest_after * 3 > plan.total_after);
    }

    #[test]
    fn commands_do_not_auto_confirm() {
        let plan = plan_next_block(&snap(0, 10, vec![("b", 3), ("a", 3)], 1)).unwrap();
        assert!(plan.delegate > 0 || plan.redelegates.len() > 1);
        let send = BankSend {
            to: FOUNDATION_DAO.into(),
            amount: DAO_SEND_TERP * MICRO,
            denom: "uterp".into(),
        };
        let gas = next_block_gas(true, &plan);
        let (fee, fee_denom) = fee_for_gas(gas, "0.05uthiol").unwrap();
        assert_eq!(fee_denom, "uthiol");
        assert_eq!(fee, u128::from(gas) * 5 / 100);
        let cmds = next_block_commands(
            "morocco-1",
            "0.05uthiol",
            FOUNDATION_DELEGATOR,
            "dest",
            "uterp",
            Some(&send),
            &plan,
        );
        assert!(cmds.len() > 1);
        assert_eq!(cmds[0][2], "bank");
        assert_eq!(cmds[0][5], FOUNDATION_DAO);
        for cmd in &cmds {
            assert!(cmd
                .iter()
                .all(|c| c != "--yes" && c != "-y" && c != "--node" && c != "auto"));
            assert!(cmd
                .windows(2)
                .any(|w| w[0] == "--gas" && w[1] == gas.to_string()));
            assert!(cmd.iter().any(|c| c == "--generate-only"));
        }
        let msgs = next_block_messages(FOUNDATION_DELEGATOR, "dest", "uterp", Some(&send), &plan);
        assert_eq!(msgs.len(), cmds.len());
        assert_eq!(msgs[0]["@type"], "/cosmos.bank.v1beta1.MsgSend");
        assert_eq!(msgs[0]["to_address"], FOUNDATION_DAO);
        assert_eq!(msgs[0]["amount"][0]["amount"], send.amount.to_string());
        let tx = unsigned_tx(&msgs, gas, fee, &fee_denom);
        assert_eq!(tx["auth_info"]["fee"]["gas_limit"], gas.to_string());
        assert_eq!(tx["body"]["messages"][0]["@type"], msgs[0]["@type"]);
    }

    #[test]
    fn bank_send_is_held_out_of_liquid_delegation() {
        let mut snapshot = snap(0, 10, vec![("a", 1)], 6);
        snapshot.reserve_bank_send(4).unwrap();
        assert_eq!(snapshot.liquid, 2);
        let err = plan_next_block(&snapshot).unwrap_err();
        assert!(matches!(err, QuorumError::Short { liquid: 2, .. }));
        let mut short = snap(0, 10, vec![], 3);
        assert!(matches!(
            short.reserve_bank_send(4),
            Err(QuorumError::SendExceedsLiquid { .. })
        ));
    }

    #[test]
    fn begin_redelegate_proto_carries_both_validators() {
        let raw = encode_msg_begin_redelegate("delegator", "src-val", "dst-val", 7, "uterp");
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("delegator"));
        assert!(text.contains("src-val"));
        assert!(text.contains("dst-val"));
        assert!(text.contains("uterp"));
        assert!(text.contains('7'));
        let delegate = encode_msg_delegate("delegator", "dst-val", 7, "uterp");
        assert!(!String::from_utf8_lossy(&delegate).contains("src-val"));
    }

    #[test]
    fn terpz_create_validator_file_uses_the_mac_consensus_key() {
        let file = create_validator_file(
            TERPZ_MONIKER,
            TERPZ_DETAILS,
            TERPZ_CONSENSUS_PUBKEY,
            DEFAULT_SELF_BOND,
            "uterp",
        );
        assert_eq!(file["moniker"], "terpz");
        assert_eq!(file["details"], TERPZ_DETAILS);
        assert_eq!(file["amount"], "1000000uterp");
        assert_eq!(file["pubkey"]["key"], TERPZ_CONSENSUS_PUBKEY);
        assert_eq!(file["pubkey"]["@type"], "/cosmos.crypto.ed25519.PubKey");
        assert_eq!(file["min-self-delegation"], "1");
    }

    #[test]
    fn advertised_validator_memo_is_loopback() {
        let memo = validator_advertised_memo(TERPZ_NODE_ID);
        assert_eq!(
            memo,
            "cbbb3d490dc9300a92c12c7eef980ceb5d508747@127.0.0.1:26656"
        );
        assert!(!memo.contains("192.168."));
    }

    #[test]
    fn dotenv_reads_quoted_mnemonics_without_keeping_comments() {
        let pairs = dotenv_assignments(
            "# comment\nTERPZ_OPERATOR_MNEMONIC=\"one two\"\nexport TERPZ_FOUNDATION_MNEMONIC='three four'\n\n",
        );
        assert_eq!(dotenv_value(&pairs, OPERATOR_MNEMONIC_ENV), Some("one two"));
        assert_eq!(
            dotenv_value(&pairs, FOUNDATION_MNEMONIC_ENV),
            Some("three four")
        );
    }

    #[test]
    fn phase_signers_must_match_the_two_keys() {
        let memo = validator_advertised_memo(TERPZ_NODE_ID);
        let create = serde_json::json!({
            "body": {"memo": memo, "messages": [{
                "@type": "/cosmos.staking.v1beta1.MsgCreateValidator",
                "validator_address": "terpvaloper1operator"
            }]}
        });
        let next = serde_json::json!({
            "body": {"messages": [
                {"@type": "/cosmos.bank.v1beta1.MsgSend", "from_address": FOUNDATION_DELEGATOR},
                {"@type": "/cosmos.staking.v1beta1.MsgBeginRedelegate", "delegator_address": FOUNDATION_DELEGATOR, "validator_dst_address": "terpvaloper1operator"}
            ]}
        });
        assert_phase_signers(&create, &next, "terpvaloper1operator", FOUNDATION_DELEGATOR).unwrap();
        let err = assert_phase_signers(&create, &next, "terpvaloper1other", FOUNDATION_DELEGATOR)
            .unwrap_err();
        assert!(err.contains("does not match"));
    }

    fn live_book() -> FoundationBook {
        FoundationBook {
            delegator: FOUNDATION_DELEGATOR.into(),
            denom: "uterp".into(),
            total_bonded: 6_141_020_723_516,
            operators: vec![
                row(TERPZ_OPERATOR, "terpz", 2_229_439_911_061, true),
                row(
                    "terpvaloper1fak8qed3zu86d8hlmkdsee9h2xtutawlqm660j",
                    "Apollo",
                    1_385_263_302_675,
                    true,
                ),
                row(
                    "terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw",
                    "Blackhox",
                    1_224_574_403_280,
                    true,
                ),
                row(
                    "terpvaloper1dmyu7jdu7pgzgsxucnav6sanfk8s7jfsw0y60a",
                    "ChainTools",
                    1_099_001_499_000,
                    true,
                ),
                row(ALXVOY_OPERATOR, "AlxVoy", 102_740_607_500, true),
                row(
                    "terpvaloper1gr67fce5ymhyz93qe0flqlgv2p424a48hccllk",
                    "BlockPro",
                    100_001_000_000,
                    true,
                ),
                row(SERGO_OPERATOR, "SerGo", 1_086_211_710_667, false),
            ],
            delegations: vec![
                StakeSource {
                    validator: TERPZ_OPERATOR.into(),
                    redelegable: 2_229_438_911_061,
                },
                StakeSource {
                    validator: SERGO_OPERATOR.into(),
                    redelegable: 986_088_933_666,
                },
            ],
            redelegations: vec![
                entry(
                    ALXVOY_OPERATOR,
                    TERPZ_OPERATOR,
                    1_746_926_009_669,
                    "2026-10-22T18:44:39.580302514Z",
                ),
                entry(
                    "terpvaloper1g80p4suzrz4vqsvs3za0yqhqcphlvnjy7snfnw",
                    TERPZ_OPERATOR,
                    462_512_901_393,
                    "2026-10-22T18:44:39.580302514Z",
                ),
                entry(
                    "terpvaloper1fak8qed3zu86d8hlmkdsee9h2xtutawlqm660j",
                    TERPZ_OPERATOR,
                    19_999_999_999,
                    "2026-10-22T18:49:31.326921063Z",
                ),
            ],
        }
    }

    fn row(operator: &str, moniker: &str, tokens: u128, bonded: bool) -> OperatorRow {
        OperatorRow {
            operator: operator.into(),
            moniker: moniker.into(),
            tokens,
            bonded,
        }
    }

    fn entry(src: &str, dst: &str, balance: u128, completion_time: &str) -> ActiveRedelegation {
        ActiveRedelegation {
            src: src.into(),
            dst: dst.into(),
            balance,
            completion_time: completion_time.into(),
        }
    }

    #[test]
    fn sergo_moves_now_and_terpz_waits_for_22_percent() {
        let plan = plan_share_rebalance(
            &live_book(),
            TERPZ_OPERATOR,
            SERGO_OPERATOR,
            ALXVOY_OPERATOR,
            TERPZ_TARGET_BPS,
        )
        .unwrap();
        assert_eq!(plan.ready.len(), 1);
        assert_eq!(plan.ready[0].src, SERGO_OPERATOR);
        assert_eq!(plan.ready[0].dst, ALXVOY_OPERATOR);
        assert_eq!(plan.ready[0].amount, 986_088_933_666);
        assert_eq!(plan.hub_after_ready, plan.hub_before);
        let total = plan.total_after_ready;
        assert_eq!(total, 6_141_020_723_516 + 986_088_933_666);
        let target = total * 2_200 / 10_000;
        let excess = plan.hub_before - target;
        assert_eq!(plan.deferred.len(), 5);
        let moved: u128 = plan.deferred.iter().map(|mv| mv.amount).sum();
        assert_eq!(moved, excess);
        assert!(plan.deferred.iter().all(|mv| mv.src == TERPZ_OPERATOR));
        assert_eq!(plan.deferred[0].dst, ALXVOY_OPERATOR);
        assert_eq!(plan.deferred[0].amount, plan.deferred[1].amount + 1);
        assert_eq!(plan.hub_after, target);
        assert!(plan.hub_after * 10_000 / plan.total_after <= 2_200);
        assert_eq!(plan.deferred_until, "2026-10-22T18:49:31.326921063Z");
        let executable = executable_rebalance(&plan, SERGO_OPERATOR);
        assert_eq!(executable, plan.ready);
        let design = hub_share_design(&plan, TERPZ_OPERATOR);
        assert_eq!(design, plan.deferred);
        let msgs = stake_move_messages(FOUNDATION_DELEGATOR, "uterp", &executable);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["validator_dst_address"], ALXVOY_OPERATOR);
        let cmds = stake_move_commands(
            "morocco-1",
            "0.05uthiol",
            FOUNDATION_DELEGATOR,
            "uterp",
            &plan.ready,
        );
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].iter().all(|c| c != "--yes" && c != "--node"));
    }

    #[test]
    fn unlocked_hub_redelegate_is_ready() {
        let mut book = live_book();
        book.redelegations.clear();
        let plan = plan_share_rebalance(
            &book,
            TERPZ_OPERATOR,
            SERGO_OPERATOR,
            ALXVOY_OPERATOR,
            TERPZ_TARGET_BPS,
        )
        .unwrap();
        assert!(plan.deferred.is_empty());
        assert_eq!(plan.deferred_until, "");
        assert_eq!(plan.ready.len(), 6);
        let executable = executable_rebalance(&plan, SERGO_OPERATOR);
        assert_eq!(executable.len(), 1);
        assert_eq!(executable[0].src, SERGO_OPERATOR);
        assert_eq!(hub_share_design(&plan, TERPZ_OPERATOR).len(), 5);
        assert_eq!(plan.ready[0].src, SERGO_OPERATOR);
        assert!(plan.ready[1..].iter().all(|mv| mv.src == TERPZ_OPERATOR));
        assert_eq!(plan.hub_after, plan.total_after * 2_200 / 10_000);
    }

    #[test]
    fn locks_reduce_redelegable_balance() {
        let sources = sources_after_locks(
            vec![StakeSource {
                validator: "a".into(),
                redelegable: 10,
            }],
            [("a".into(), 4), ("a".into(), 6)],
        );
        assert!(sources.is_empty());
    }
}
