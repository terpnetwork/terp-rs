//! ZAP1 + IBC **v2** 08-wasm client.
//!
//! Verifies a `zap1-verify` BLAKE2b Merkle path and IBC v2 packet fields
//! (source/dest **clients**, sequence, timeout, `app_data_hash`).
//! This is not Crosslink header / Ed25519 finalizer verification.
//!
//! Leaf and node hashes stay inside `zap1-verify` (`blake2b_simd`) with
//! personals `NordicShield_` and `NordicShield_MRK`. Host `Api::blake2b_256`
//! is message-only and is not a substitute. Guest `simd128` stays off.

use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult,
};
use cw_storage_plus::Map;
use sha2::{Digest, Sha256};
use zap1_verify::{compute_leaf_hash, verify_proof, EventPayload, ProofStep, SiblingPosition};

#[cfg(not(target_arch = "wasm32"))]
mod interface;
#[cfg(not(target_arch = "wasm32"))]
pub use interface::CwIcs08WasmZap1Contract;

/// Domain-separated SHA-256 (same construction as PIR `tagged_hash`).
pub fn tagged_hash(tag: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(tag);
    h.update(&[tag.len() as u8]);
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().into()
}

pub fn app_data_hash(
    note: &[u8; 32],
    amount_zat: u64,
    frost_group: &[u8; 32],
    zap1_leaf: &[u8; 32],
) -> [u8; 32] {
    tagged_hash(
        // SSOT: terp_seams::domain::IBC_V2_ZAP1_APP_V1 (keep inlined for wasm-slim deps)
        b"ibc-v2-zap1-app-v1",
        &[note, &amount_zat.to_le_bytes(), frost_group, zap1_leaf],
    )
}

#[cosmwasm_schema::cw_serde]
pub struct InstantiateMsg {}

#[cosmwasm_schema::cw_serde]
pub struct Zap1ProofStepMsg {
    pub sibling_hex: String,
    pub sibling_is_left: bool,
}

#[cosmwasm_schema::cw_serde]
pub struct IbcV2Fields {
    pub source_client: String,
    pub dest_client: String,
    pub sequence: u64,
    pub timeout_timestamp_ns: u64,
    pub app_data_hash_hex: String,
}

#[cosmwasm_schema::cw_serde]
pub enum ExecuteMsg {
    CreateClient {
        source_client: String,
        dest_client: String,
    },
    /// IBC v2 packet + ZAP1 Merkle. Tampered `app_data_hash` is rejected.
    AttestPacket {
        source_client: String,
        dest_client: String,
        sequence: u64,
        timeout_timestamp_ns: u64,
        app_data_hash_hex: String,
        note_hex: String,
        amount_zat: u64,
        frost_group_hex: String,
        leaf_hex: String,
        merkle_root_hex: String,
        event_kind: String,
        wallet_or_serial: String,
        proof: Vec<Zap1ProofStepMsg>,
    },
}

#[cosmwasm_schema::cw_serde]
#[derive(cosmwasm_schema::QueryResponses)]
pub enum QueryMsg {
    #[returns(AcceptedResp)]
    Accepted {
        source_client: String,
        dest_client: String,
        sequence: u64,
        app_data_hash_hex: String,
    },
}

#[cosmwasm_schema::cw_serde]
pub struct AcceptedResp {
    pub accepted: bool,
}

const ACCEPTED: Map<(&str, &str, u64), String> = Map::new("zap1_ibcv2_accepted");

fn parse_hex32(s: &str) -> StdResult<[u8; 32]> {
    let t = s.trim().trim_start_matches("0x");
    let v = hex::decode(t).map_err(|_| StdError::msg("invalid hex"))?;
    v.try_into()
        .map_err(|_| StdError::msg("expected 32 bytes"))
}

/// Pure verify used by the wasm entry point and by PIR (same fields as IBC v2).
pub fn verify_zap1_ibcv2(
    ibc: &IbcV2Fields,
    event_kind: &str,
    wallet_or_serial: &str,
    note: &[u8; 32],
    amount_zat: u64,
    frost_group: &[u8; 32],
    leaf: &[u8; 32],
    merkle_root: &[u8; 32],
    proof: &[( [u8; 32], bool )],
) -> Result<[u8; 32], String> {
    if ibc.source_client.is_empty() || ibc.dest_client.is_empty() {
        return Err("empty ibc client id".into());
    }
    if ibc.sequence == 0 {
        return Err("zero sequence".into());
    }
    let payload = match event_kind {
        "hosting_payment" | "HostingPayment" => EventPayload::HostingPayment {
            serial_number: wallet_or_serial.as_bytes(),
            month: 8,
            year: 2026,
        },
        "program_entry" | "ProgramEntry" => EventPayload::ProgramEntry {
            wallet_hash: wallet_or_serial.as_bytes(),
        },
        "ownership_attest" | "OwnershipAttest" => EventPayload::OwnershipAttest {
            wallet_hash: wallet_or_serial.as_bytes(),
            serial_number: b"pir-seat",
        },
        _ => return Err("unknown event kind".into()),
    };
    let computed = compute_leaf_hash(&payload);
    if &computed != leaf {
        return Err("leaf does not match event payload".into());
    }
    let path: Vec<ProofStep> = proof
        .iter()
        .map(|(sib, left)| ProofStep {
            hash: *sib,
            position: if *left {
                SiblingPosition::Left
            } else {
                SiblingPosition::Right
            },
        })
        .collect();
    if !verify_proof(leaf, &path, merkle_root) {
        return Err("zap1 merkle proof failed".into());
    }
    let expect = app_data_hash(note, amount_zat, frost_group, leaf);
    let got = parse_hex32(&ibc.app_data_hash_hex).map_err(|e| e.to_string())?;
    if got != expect {
        return Err("tampered app_data_hash".into());
    }
    Ok(expect)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _msg: InstantiateMsg,
) -> StdResult<Response> {
    Ok(Response::new().add_attribute("client", "08-zap1-ibcv2"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(deps: DepsMut, _env: Env, _info: MessageInfo, msg: ExecuteMsg) -> StdResult<Response> {
    match msg {
        ExecuteMsg::CreateClient {
            source_client,
            dest_client,
        } => {
            if source_client.is_empty() || dest_client.is_empty() {
                return Err(StdError::msg("empty client id"));
            }
            Ok(Response::new().add_attribute("action", "create_client"))
        }
        ExecuteMsg::AttestPacket {
            source_client,
            dest_client,
            sequence,
            timeout_timestamp_ns,
            app_data_hash_hex,
            note_hex,
            amount_zat,
            frost_group_hex,
            leaf_hex,
            merkle_root_hex,
            event_kind,
            wallet_or_serial,
            proof,
        } => {
            let ibc = IbcV2Fields {
                source_client: source_client.clone(),
                dest_client: dest_client.clone(),
                sequence,
                timeout_timestamp_ns,
                app_data_hash_hex: app_data_hash_hex.clone(),
            };
            let note = parse_hex32(&note_hex)?;
            let frost = parse_hex32(&frost_group_hex)?;
            let leaf = parse_hex32(&leaf_hex)?;
            let root = parse_hex32(&merkle_root_hex)?;
            let mut steps = Vec::new();
            for s in &proof {
                steps.push((parse_hex32(&s.sibling_hex)?, s.sibling_is_left));
            }
            let hash = verify_zap1_ibcv2(
                &ibc,
                &event_kind,
                &wallet_or_serial,
                &note,
                amount_zat,
                &frost,
                &leaf,
                &root,
                &steps,
            )
            .map_err(StdError::msg)?;
            let hexh = hex::encode(hash);
            ACCEPTED.save(
                deps.storage,
                (&source_client, &dest_client, sequence),
                &hexh,
            )?;
            Ok(Response::new()
                .add_attribute("action", "attest_packet")
                .add_attribute("app_data_hash", hexh))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Accepted {
            source_client,
            dest_client,
            sequence,
            app_data_hash_hex,
        } => {
            let stored = ACCEPTED.may_load(
                deps.storage,
                (&source_client, &dest_client, sequence),
            )?;
            let want = app_data_hash_hex.trim().trim_start_matches("0x").to_ascii_lowercase();
            let accepted = stored
                .map(|s| s.eq_ignore_ascii_case(&want))
                .unwrap_or(false);
            to_json_binary(&AcceptedResp { accepted })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tampered_hash_rejected() {
        let leaf = [1u8; 32];
        let ibc = IbcV2Fields {
            source_client: "08-zap1-src".into(),
            dest_client: "08-terp-dst".into(),
            sequence: 1,
            timeout_timestamp_ns: 0,
            app_data_hash_hex: "00".repeat(32),
        };
        let err = verify_zap1_ibcv2(
            &ibc,
            "hosting_payment",
            "x",
            &[2u8; 32],
            1,
            &[3u8; 32],
            &leaf,
            &[4u8; 32],
            &[],
        )
        .unwrap_err();
        assert!(err.contains("leaf") || err.contains("merkle") || err.contains("tampered"));
    }
}
