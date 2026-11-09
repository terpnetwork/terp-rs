use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin, Empty, Timestamp, Uint256};
use cw_storage_plus::{Index, IndexList, IndexedMap, Item, Map, MultiIndex};

/// Who may propose (create) a bounty. A sender is eligible if ANY enabled gate passes.
#[cw_serde]
pub struct ProposerGate {
    /// Admin-managed allowlist (ALLOWLIST map) is consulted.
    pub use_allowlist: bool,
    /// Sum of the sender's delegations in the chain bond denom
    /// (StakingQuery::AllDelegations + BondedDenom) must be >= this. Must be > 0 when set.
    pub min_stake: Option<Uint256>,
    /// cw4 group contract; sender must be a member with weight > 0.
    pub cw4_group: Option<Addr>,
}

/// Who may create a bounty while the creation policy is `Anyone {}` (no pre-propose module).
/// When a pre-propose module is installed (`ProposalCreationPolicy::Module`), that module is the gate
/// and this setting is not consulted.
#[cw_serde]
pub enum CreationGate {
    /// Legacy behaviour: the `ProposerGate` (allowlist / min_stake / cw4, any passes).
    ProposerGate {},
    /// (a) Anyone may deposit and create a bounty. Listing still needs the approver.
    Anyone {},
    /// (b) The sender's bonded stake in the chain bond denom (same `bonded()` check as
    /// `ProposerGate.min_stake` and the submit/upload gates) must be >= `amount`. `amount` > 0.
    MinStake { amount: Uint256 },
    /// (c) TODO, NOT IMPLEMENTED: a sound, private proof that the proposer deposited into an escrow on a
    /// counterparty chain (e.g. Zcash shielded) without revealing the deposit details. See DESIGN.md §9.
    /// Instantiate/update_config reject it and create would fail with `Unimplemented`.
    CounterpartyEscrowProof {
        /// Counterparty chain id, e.g. "zcash-mainnet".
        chain: String,
    },
}

impl Default for CreationGate {
    fn default() -> Self {
        CreationGate::ProposerGate {}
    }
}

#[cw_serde]
pub struct DenomRule {
    pub denom: String,
    pub min_amount: Uint256,
    pub max_amount: Uint256,
}

#[cw_serde]
pub struct Config {
    /// Can change config and the allowlist. DAO core address in prod.
    pub owner: Addr,
    /// The only address that can approve listings, accept work and release payouts.
    /// A human admin key in dev; a cw3 multisig or DAO-DAO core in prod. Never automatic.
    pub approver: Addr,
    pub gate: ProposerGate,
    pub denoms: Vec<DenomRule>,
    pub allow_partial_payouts: bool,
    /// Fee taken from each payout, in basis points. Hard-capped at 1000 (10%).
    pub fee_bps: u16,
    pub fee_collector: Option<Addr>,
    /// expires_at must be within [now + min_duration_secs, now + max_duration_secs].
    pub min_duration_secs: u64,
    pub max_duration_secs: u64,
    /// Label of the governance parameter set in force (e.g. escrow/params/escrow.params.json
    /// `params_version`). Stamped on every bounty and on create/update_config events.
    pub params_version: String,
    /// true while the parameters are PLACEHOLDERS that governance has not voted.
    pub params_placeholder: bool,
    /// Submission stake gate: the assignee's bonded-denom delegations must be >= this to `submit`.
    /// 0 disables the gate. Governance parameter (placeholder until voted).
    pub min_submit_stake: Uint256,
    /// Upload stake gate for the off-chain CID storage service (terp-bounty-details/v1 and
    /// terp-bounty-submission/v1 documents): the uploader's bonded-denom delegations must be >= this.
    /// The contract does not see uploads; it is the governance source of the threshold and of the
    /// `can_upload` check the service calls. 0 disables. Placeholder until voted.
    /// `serde(default)` keeps configs stored by older code readable after a migrate.
    #[serde(default)]
    pub min_upload_stake: Uint256,
    /// Creation gate applied when no pre-propose module is installed. `serde(default)` = ProposerGate {}.
    #[serde(default)]
    pub creation_gate: CreationGate,
}

#[cw_serde]
pub enum BountyStatus {
    /// Funded, waiting for the approver to list it.
    Pending,
    /// Listed (approved); accepting applications off-chain.
    Open,
    Assigned,
    Submitted,
    /// Work accepted by the approver; awaiting (or partially through) payout.
    Accepted,
    Paid,
    Cancelled,
    Refunded,
    Expired,
}

impl BountyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BountyStatus::Pending => "pending",
            BountyStatus::Open => "open",
            BountyStatus::Assigned => "assigned",
            BountyStatus::Submitted => "submitted",
            BountyStatus::Accepted => "accepted",
            BountyStatus::Paid => "paid",
            BountyStatus::Cancelled => "cancelled",
            BountyStatus::Refunded => "refunded",
            BountyStatus::Expired => "expired",
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            BountyStatus::Paid
                | BountyStatus::Cancelled
                | BountyStatus::Refunded
                | BountyStatus::Expired
        )
    }
}

#[cw_serde]
pub struct Bounty {
    pub id: u64,
    /// Off-chain board id (zec-bounties DB Bounty.id). Unique.
    pub bounty_ref: String,
    pub title: String,
    /// Short public summary, at most SUMMARY_MAX_BYTES (280) bytes of UTF-8. Full details live off chain.
    pub summary: String,
    /// sha256 (hex) of the canonical terp-bounty-details/v1 JSON document (the full details).
    pub details_hash: String,
    /// Optional IPFS CID of that document (CIDv0 Qm… or CIDv1 base32 b…).
    pub details_cid: Option<String>,
    pub proposer: Addr,
    pub reward: Coin,
    /// Gross amount released so far (fees included).
    pub paid: Uint256,
    pub refunded: Uint256,
    pub status: BountyStatus,
    pub assignee: Option<Addr>,
    pub created_at: Timestamp,
    pub created_height: u64,
    pub expires_at: Timestamp,
    pub submission_count: u32,
    pub updated_at: Timestamp,
    /// Config.params_version in force when the bounty was created.
    pub params_version: String,
}

impl Bounty {
    pub fn remaining(&self) -> Uint256 {
        self.reward.amount - self.paid - self.refunded
    }
}

#[cw_serde]
pub struct Submission {
    pub bounty_id: u64,
    pub seq: u32,
    pub submitter: Addr,
    /// sha256 (hex) of the exact bytes of the terp-bounty-submission/v1 JSON doc; the backend re-hashes to verify.
    pub hash: String,
    /// IPFS CID of that doc (CIDv0 Qm… or CIDv1 base32 b…), validated on chain (cid.rs).
    pub cid: String,
    pub submitted_at: Timestamp,
    pub height: u64,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const BOUNTY_SEQ: Item<u64> = Item::new("bounty_seq");
pub const BOUNTY_REFS: Map<&str, u64> = Map::new("bounty_refs");
pub const SUBMISSIONS: Map<(u64, u32), Submission> = Map::new("submissions");
pub const ALLOWLIST: Map<&Addr, Empty> = Map::new("allowlist");
/// DAO DAO core contract this module belongs to (proposal-module `Dao {}` query).
pub const DAO: Item<Addr> = Item::new("dao");
/// dao-voting `ProposalCreationPolicy`: Anyone, or a pre-propose Module with exclusive creation rights.
pub const CREATION_POLICY: Item<dao_voting::pre_propose::ProposalCreationPolicy> =
    Item::new("creation_policy");
/// cw-hooks receivers of dao-hooks `ProposalHookMsg` (NewProposal / ProposalStatusChanged).
pub const PROPOSAL_HOOKS: cw_hooks::Hooks = cw_hooks::Hooks::new("proposal_hooks");

pub struct BountyIndexes<'a> {
    pub status: MultiIndex<'a, String, Bounty, u64>,
    pub proposer: MultiIndex<'a, Addr, Bounty, u64>,
}

impl IndexList<Bounty> for BountyIndexes<'_> {
    fn get_indexes(&'_ self) -> Box<dyn Iterator<Item = &'_ dyn Index<Bounty>> + '_> {
        let v: Vec<&dyn Index<Bounty>> = vec![&self.status, &self.proposer];
        Box::new(v.into_iter())
    }
}

pub fn bounties<'a>() -> IndexedMap<u64, Bounty, BountyIndexes<'a>> {
    IndexedMap::new(
        "bounties",
        BountyIndexes {
            status: MultiIndex::new(
                |_pk, b: &Bounty| b.status.as_str().to_string(),
                "bounties",
                "bounties__status",
            ),
            proposer: MultiIndex::new(
                |_pk, b: &Bounty| b.proposer.clone(),
                "bounties",
                "bounties__proposer",
            ),
        },
    )
}
