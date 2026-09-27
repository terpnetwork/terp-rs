//! Native staking + bank actions a DAO executes from its treasury.
//!
//! Complements `DaoVoteDelegation` (voting-power delegates). This module
//! builds CosmosMsgs for `MsgDelegate` / `MsgSend` so Foundation-style
//! proposals (spread stake, gas a new operator, instantiate a subDAO) share
//! one builder.

use cosmwasm_std::{AnyMsg, Binary, Coin, CosmosMsg, Uint256, WasmMsg};

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

/// morocco-1 DAO DAO codes used to instantiate FIAB.
pub const CODE_CORE: u64 = 23;
pub const CODE_PROPOSAL_SINGLE: u64 = 24;
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
    let (hrp, data) = addr.split_once('1').ok_or_else(|| format!("not bech32: {addr}"))?;
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
        for i in 0..5 {
            if (b >> i) & 1 == 1 {
                chk ^= GEN[i];
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
    for i in 0..6 {
        checksum[i] = ((polymod >> (5 * (5 - i))) & 31) as u8;
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
    fn million_tokens_is_one_million_whole() {
        assert_eq!(million_tokens(), Uint256::from(1_000_000_000_000u128));
        assert_eq!(terp_100k(), Uint256::from(100_000_000_000u128));
    }
}
