//! cw-orch suite for `terp-bounty-escrow` (DEV/TEST ONLY).
//!
//! Runs on any `CwEnv`; the tests use `MockBech32`. There is intentionally no deploy binary:
//! the contract is not audited and must not be stored on a public chain (the contract also
//! refuses `morocco-1` through its default `dev-chain-guard` feature).
use cosmwasm_std::{Timestamp, Uint256};
use cw_orch::prelude::*;

pub use terp_bounty_escrow::interface::TerpBountyEscrow;
pub use terp_bounty_escrow::msg::{
    ExecuteMsgFns as _, InstantiateMsg as BountyEscrowInitMsg, PreProposeInfo,
    ProposalCreationPolicy, ProposerGateMsg, QueryMsgFns as _,
};
pub use terp_bounty_escrow::state::CreationGate;
pub use terp_bounty_escrow::state::DenomRule;

/// Placeholder governance parameters (same file the contract tests and the TS client read).
pub const PARAMS_JSON: &str =
    include_str!("../../../contracts/revenue/bounty-escrow/params/escrow.params.json");

fn params() -> serde_json::Value {
    serde_json::from_str(PARAMS_JSON).expect("escrow.params.json")
}

fn strip_labels(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => serde_json::Value::Object(
            m.iter()
                .filter(|(k, _)| !k.starts_with('_'))
                .map(|(k, v)| (k.clone(), strip_labels(v)))
                .collect(),
        ),
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(strip_labels).collect())
        }
        x => x.clone(),
    }
}

/// Denom caps from the params file.
pub fn param_denoms() -> Vec<DenomRule> {
    serde_json::from_value(strip_labels(&params()["instantiate"]["denoms"])).expect("denoms")
}

/// `min_submit_stake` from the params file (micro units of the bond denom).
pub fn param_min_submit_stake() -> Uint256 {
    let s = params()["instantiate"]["min_submit_stake"]
        .as_str()
        .expect("min_submit_stake")
        .to_string();
    Uint256::from(s.parse::<u128>().expect("u128"))
}

/// `min_upload_stake` from the params file (micro units of the bond denom); the CID storage
/// service checks it off chain through the `can_upload` query.
pub fn param_min_upload_stake() -> Uint256 {
    let s = params()["instantiate"]["min_upload_stake"]
        .as_str()
        .expect("min_upload_stake")
        .to_string();
    Uint256::from(s.parse::<u128>().expect("u128"))
}

/// Build an instantiate msg from the placeholder params.
/// `min_upload_stake` comes from the params file.
pub fn init_msg(
    owner: &Addr,
    approver: &Addr,
    allowlist: Vec<String>,
    min_submit_stake: Uint256,
) -> BountyEscrowInitMsg {
    let p = params();
    let i = &p["instantiate"];
    BountyEscrowInitMsg {
        owner: owner.to_string(),
        approver: approver.to_string(),
        gate: ProposerGateMsg {
            use_allowlist: true,
            min_stake: None,
            cw4_group: None,
        },
        denoms: param_denoms(),
        allow_partial_payouts: false,
        fee_bps: 0,
        fee_collector: None,
        min_duration_secs: i["min_duration_secs"].as_u64().expect("min_duration_secs"),
        max_duration_secs: i["max_duration_secs"].as_u64().expect("max_duration_secs"),
        params_version: p["params_version"]
            .as_str()
            .expect("params_version")
            .to_string(),
        params_placeholder: true,
        min_submit_stake,
        min_upload_stake: param_min_upload_stake(),
        initial_allowlist: allowlist,
        dao: None,
        pre_propose_info: None,
        creation_gate: None,
    }
}

/// Deploy bag for the escrow.
pub struct BountyEscrowSuite<Chain> {
    pub chain: Chain,
    pub escrow: TerpBountyEscrow<Chain>,
}

impl<Chain: CwEnv> BountyEscrowSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            escrow: TerpBountyEscrow::new(chain.clone()),
            chain,
        }
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for BountyEscrowSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = BountyEscrowInitMsg;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.escrow.upload_if_needed()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.escrow)]
    }

    fn load_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self::new(chain))
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain)?;
        let admin = Addr::unchecked(data.owner.clone());
        suite.escrow.instantiate(&data, Some(&admin), &[])?;
        Ok(suite)
    }
}

/// Default expiry used by the tests: block time + the params' minimum window + 1 day.
pub fn expiry(now: Timestamp) -> Timestamp {
    let min = params()["instantiate"]["min_duration_secs"]
        .as_u64()
        .unwrap_or(0);
    now.plus_seconds(min + 86_400)
}
