//! 07-tendermint client construction for morocco-1 and its counterparties.
//!
//! Builds `MsgCreateClient` and gov `MsgRecoverClient` as protobuf `Any` for
//! the cw-orch Daemon. Penumbra gets its JMT ics23 proof specs; cosmos-sdk
//! chains get IAVL + tendermint specs.

use anyhow::{anyhow, bail, Context, Result};
use cosmos_sdk_proto::prost::Message;
use cosmrs::Any;
use ibc_proto::ics23::{HashOp, InnerSpec, LeafOp, LengthOp, ProofSpec};
use ibc_proto::ibc::core::client::v1::{Height, MsgCreateClient, MsgRecoverClient};
use ibc_proto::ibc::core::commitment::v1::MerkleRoot;
use ibc_proto::ibc::lightclients::tendermint::v1::{ClientState, ConsensusState, Fraction};

pub const MSG_CREATE_CLIENT: &str = "/ibc.core.client.v1.MsgCreateClient";
pub const MSG_RECOVER_CLIENT: &str = "/ibc.core.client.v1.MsgRecoverClient";
pub const MSG_SUBMIT_PROPOSAL: &str = "/cosmos.gov.v1.MsgSubmitProposal";
pub const TM_CLIENT_STATE: &str = "/ibc.lightclients.tendermint.v1.ClientState";
pub const TM_CONSENSUS_STATE: &str = "/ibc.lightclients.tendermint.v1.ConsensusState";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    /// cosmos-sdk IAVL + tendermint simple merkle.
    Iavl,
    /// Penumbra: two JMT specs (main store and its app-hash wrapper).
    PenumbraJmt,
}

/// Header facts the client is anchored to.
#[derive(Clone, Debug)]
pub struct Anchor {
    pub chain_id: String,
    pub height: u64,
    pub time_unix: i64,
    pub time_nanos: i32,
    pub app_hash: Vec<u8>,
    pub next_validators_hash: Vec<u8>,
    pub unbonding_secs: u64,
    pub store: Store,
}

/// Revision number from an IBC chain-id like `osmosis-1` or `penumbra-1`.
pub fn revision_number(chain_id: &str) -> u64 {
    chain_id
        .rsplit_once('-')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(0)
}

fn iavl_spec() -> ProofSpec {
    ProofSpec {
        leaf_spec: Some(LeafOp {
            hash: HashOp::Sha256 as i32,
            prehash_key: HashOp::NoHash as i32,
            prehash_value: HashOp::Sha256 as i32,
            length: LengthOp::VarProto as i32,
            prefix: vec![0],
        }),
        inner_spec: Some(InnerSpec {
            child_order: vec![0, 1],
            child_size: 33,
            min_prefix_length: 4,
            max_prefix_length: 12,
            empty_child: vec![],
            hash: HashOp::Sha256 as i32,
        }),
        max_depth: 0,
        min_depth: 0,
        prehash_key_before_comparison: false,
    }
}

fn tendermint_spec() -> ProofSpec {
    ProofSpec {
        leaf_spec: Some(LeafOp {
            hash: HashOp::Sha256 as i32,
            prehash_key: HashOp::NoHash as i32,
            prehash_value: HashOp::Sha256 as i32,
            length: LengthOp::VarProto as i32,
            prefix: vec![0],
        }),
        inner_spec: Some(InnerSpec {
            child_order: vec![0, 1],
            child_size: 32,
            min_prefix_length: 1,
            max_prefix_length: 1,
            empty_child: vec![],
            hash: HashOp::Sha256 as i32,
        }),
        max_depth: 0,
        min_depth: 0,
        prehash_key_before_comparison: false,
    }
}

/// Penumbra JMT spec. Matches the live penumbra-1 clients on osmosis-1
/// (07-tendermint-3242/3717/3754): leaf prefix "JMT::LeafNode", 32-byte
/// children, 16-byte prefixes, SPARSE_MERKLE_PLACEHOLDER_HASH__ empty child.
pub fn penumbra_jmt_spec() -> ProofSpec {
    ProofSpec {
        leaf_spec: Some(LeafOp {
            hash: HashOp::Sha256 as i32,
            prehash_key: HashOp::Sha256 as i32,
            prehash_value: HashOp::Sha256 as i32,
            length: LengthOp::NoPrefix as i32,
            prefix: b"JMT::LeafNode".to_vec(),
        }),
        inner_spec: Some(InnerSpec {
            child_order: vec![0, 1],
            child_size: 32,
            min_prefix_length: 16,
            max_prefix_length: 16,
            empty_child: b"SPARSE_MERKLE_PLACEHOLDER_HASH__".to_vec(),
            hash: HashOp::Sha256 as i32,
        }),
        max_depth: 64,
        min_depth: 0,
        prehash_key_before_comparison: true,
    }
}

pub fn proof_specs(store: Store) -> Vec<ProofSpec> {
    match store {
        Store::Iavl => vec![iavl_spec(), tendermint_spec()],
        Store::PenumbraJmt => vec![penumbra_jmt_spec(), penumbra_jmt_spec()],
    }
}

fn duration(secs: u64) -> ibc_proto::google::protobuf::Duration {
    ibc_proto::google::protobuf::Duration { seconds: secs as i64, nanos: 0 }
}

/// Trusting period is 2/3 of unbonding (hermes default), rounded down.
pub fn trusting_secs(unbonding_secs: u64) -> u64 {
    unbonding_secs * 2 / 3
}

pub fn client_state(a: &Anchor, max_clock_drift_secs: u64) -> Result<ClientState> {
    if a.unbonding_secs < 3600 {
        bail!("{}: unbonding {}s looks wrong", a.chain_id, a.unbonding_secs);
    }
    if a.app_hash.is_empty() || a.next_validators_hash.len() != 32 {
        bail!("{}: header is missing app_hash or next_validators_hash", a.chain_id);
    }
    #[allow(deprecated)]
    Ok(ClientState {
        chain_id: a.chain_id.clone(),
        trust_level: Some(Fraction { numerator: 1, denominator: 3 }),
        trusting_period: Some(duration(trusting_secs(a.unbonding_secs))),
        unbonding_period: Some(duration(a.unbonding_secs)),
        max_clock_drift: Some(duration(max_clock_drift_secs)),
        frozen_height: Some(Height { revision_number: 0, revision_height: 0 }),
        latest_height: Some(Height {
            revision_number: revision_number(&a.chain_id),
            revision_height: a.height,
        }),
        proof_specs: proof_specs(a.store),
        upgrade_path: vec!["upgrade".into(), "upgradedIBCState".into()],
        allow_update_after_expiry: false,
        allow_update_after_misbehaviour: false,
    })
}

pub fn consensus_state(a: &Anchor) -> ConsensusState {
    ConsensusState {
        timestamp: Some(ibc_proto::google::protobuf::Timestamp {
            seconds: a.time_unix,
            nanos: a.time_nanos,
        }),
        root: Some(MerkleRoot { hash: a.app_hash.clone() }),
        next_validators_hash: a.next_validators_hash.clone(),
    }
}

pub fn create_client_any(a: &Anchor, signer: &str, max_clock_drift_secs: u64) -> Result<Any> {
    let cs = client_state(a, max_clock_drift_secs)?;
    let msg = MsgCreateClient {
        client_state: Some(ibc_proto::google::protobuf::Any {
            type_url: TM_CLIENT_STATE.into(),
            value: cs.encode_to_vec(),
        }),
        consensus_state: Some(ibc_proto::google::protobuf::Any {
            type_url: TM_CONSENSUS_STATE.into(),
            value: consensus_state(a).encode_to_vec(),
        }),
        signer: signer.into(),
    };
    Ok(Any { type_url: MSG_CREATE_CLIENT.into(), value: msg.encode_to_vec() })
}

pub fn is_tm_client_id(id: &str) -> bool {
    id.strip_prefix("07-tendermint-")
        .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(false)
}

/// Gov proposal: recover each `(subject, substitute)` pair. `authority` is
/// the gov module account; `proposer` is the cw-orch-kms granter.
pub fn recover_proposal_any(
    pairs: &[(String, String)],
    authority: &str,
    proposer: &str,
    deposit: (u128, &str),
    title: &str,
    summary: &str,
) -> Result<Any> {
    use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
    use cosmos_sdk_proto::cosmos::gov::v1::MsgSubmitProposal;
    if pairs.is_empty() {
        bail!("no subject:substitute pairs");
    }
    if title.is_empty() || title.len() > 255 || summary.is_empty() {
        bail!("title must be 1..=255 bytes and summary non-empty");
    }
    let mut messages = Vec::new();
    for (subject, substitute) in pairs {
        if !is_tm_client_id(subject) || !is_tm_client_id(substitute) || subject == substitute {
            bail!("bad pair {subject}:{substitute}");
        }
        let m = MsgRecoverClient {
            subject_client_id: subject.clone(),
            substitute_client_id: substitute.clone(),
            signer: authority.into(),
        };
        messages.push(cosmos_sdk_proto::Any { type_url: MSG_RECOVER_CLIENT.into(), value: m.encode_to_vec() });
    }
    let submit = MsgSubmitProposal {
        messages,
        initial_deposit: vec![Coin { denom: deposit.1.into(), amount: deposit.0.to_string() }],
        proposer: proposer.into(),
        metadata: String::new(),
        title: title.into(),
        summary: summary.into(),
        expedited: false,
    };
    Ok(Any { type_url: MSG_SUBMIT_PROPOSAL.into(), value: submit.encode_to_vec() })
}

/// Latest header of a cosmos-sdk chain through cw-orch's read-only gRPC daemon.
/// The connect step refuses a node whose chain-id differs.
pub fn cosmos_anchor(info: &cw_orch::environment::ChainInfoOwned) -> Result<Anchor> {
    use cw_orch::daemon::queriers::{Node, Staking};
    let daemon = crate::kms_gate::query_only(info)?;
    let channel = daemon.channel();
    let rt = &daemon.rt_handle;
    let block = rt
        .block_on(Node::new_async(channel.clone())._latest_block())
        .map_err(|e| anyhow!("{}: latest block: {e}", info.chain_id))?;
    let header = block.header.context("block without header")?;
    let params = rt
        .block_on(Staking::new_async(channel.clone())._params())
        .map_err(|e| anyhow!("{}: staking params: {e}", info.chain_id))?
        .params
        .context("no staking params")?;
    let unbonding = params.unbonding_time.context("no unbonding_time")?;
    let time = header.time.context("header without time")?;
    Ok(Anchor {
        chain_id: header.chain_id.clone(),
        height: header.height as u64,
        time_unix: time.seconds,
        time_nanos: time.nanos,
        app_hash: header.app_hash.clone(),
        next_validators_hash: header.next_validators_hash.clone(),
        unbonding_secs: unbonding.seconds as u64,
        store: Store::Iavl,
    })
}

/// Penumbra exposes no cosmos `GetNodeInfo`, so the header comes from the
/// CometBFT RPC `/commit` (read-only). Unbonding is passed in seconds
/// (penumbra-1 stakeParams.unbondingDelay = 120960 blocks; the live osmosis
/// clients use 604800s).
pub fn penumbra_anchor(rpc: &str, expected_chain_id: &str, unbonding_secs: u64) -> Result<Anchor> {
    let url = format!("{}/commit", rpc.trim_end_matches('/'));
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "20", &url])
        .output()
        .context("curl penumbra /commit")?;
    if !out.status.success() {
        bail!("penumbra /commit failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let h = &v["result"]["signed_header"]["header"];
    let chain_id = h["chain_id"].as_str().context("no chain_id")?.to_string();
    if chain_id != expected_chain_id {
        bail!("penumbra rpc reports {chain_id}, expected {expected_chain_id}");
    }
    let height: u64 = h["height"].as_str().context("no height")?.parse()?;
    let time = h["time"].as_str().context("no time")?;
    let (time_unix, time_nanos) = parse_rfc3339(time)?;
    Ok(Anchor {
        chain_id,
        height,
        time_unix,
        time_nanos,
        app_hash: hex(h["app_hash"].as_str().context("no app_hash")?)?,
        next_validators_hash: hex(h["next_validators_hash"].as_str().context("no nvh")?)?,
        unbonding_secs,
        store: Store::PenumbraJmt,
    })
}

fn hex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("odd hex");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!(e)))
        .collect()
}

/// Minimal RFC3339 (UTC, `Z`) parser for CometBFT header times.
pub fn parse_rfc3339(s: &str) -> Result<(i64, i32)> {
    let s = s.strip_suffix('Z').context("time is not UTC")?;
    let (date, time) = s.split_once('T').context("bad time")?;
    let d: Vec<i64> = date.split('-').map(|x| x.parse()).collect::<Result<_, _>>()?;
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let t: Vec<i64> = hms.split(':').map(|x| x.parse()).collect::<Result<_, _>>()?;
    if d.len() != 3 || t.len() != 3 {
        bail!("bad time {s}");
    }
    let (y, m, day) = (d[0], d[1], d[2]);
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut f = frac.to_string();
    f.truncate(9);
    while f.len() < 9 {
        f.push('0');
    }
    Ok((days * 86400 + t[0] * 3600 + t[1] * 60 + t[2], f.parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(store: Store) -> Anchor {
        Anchor {
            chain_id: "penumbra-1".into(),
            height: 13350993,
            time_unix: 1_790_000_000,
            time_nanos: 0,
            app_hash: vec![1; 32],
            next_validators_hash: vec![2; 32],
            unbonding_secs: 604800,
            store,
        }
    }

    #[test]
    fn penumbra_client_matches_live_osmosis_client_shape() {
        let cs = client_state(&anchor(Store::PenumbraJmt), 7235).unwrap();
        assert_eq!(cs.trusting_period.unwrap().seconds, 403200);
        assert_eq!(cs.latest_height.unwrap().revision_number, 1);
        assert_eq!(cs.proof_specs.len(), 2);
        let leaf = cs.proof_specs[0].leaf_spec.clone().unwrap();
        assert_eq!(leaf.prefix, b"JMT::LeafNode");
        assert_eq!(cs.proof_specs[0].inner_spec.clone().unwrap().empty_child, b"SPARSE_MERKLE_PLACEHOLDER_HASH__");
        assert!(cs.proof_specs[0].prehash_key_before_comparison);
    }

    #[test]
    fn recover_rejects_bad_pairs_and_uses_gov_authority() {
        let auth = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";
        assert!(recover_proposal_any(&[], auth, "p", (1, "uterp"), "t", "s").is_err());
        assert!(recover_proposal_any(&[("x".into(), "07-tendermint-1".into())], auth, "p", (1, "uterp"), "t", "s").is_err());
        let a = recover_proposal_any(&[("07-tendermint-1".into(), "07-tendermint-2".into())], auth, "p", (1, "uterp"), "t", "s").unwrap();
        let m = cosmos_sdk_proto::cosmos::gov::v1::MsgSubmitProposal::decode(a.value.as_slice()).unwrap();
        let r = MsgRecoverClient::decode(m.messages[0].value.as_slice()).unwrap();
        assert_eq!(r.signer, auth);
    }

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339("1970-01-02T00:00:01.5Z").unwrap(), (86401, 500_000_000));
        assert_eq!(revision_number("osmosis-1"), 1);
        assert_eq!(revision_number("morocco-1"), 1);
    }
}
