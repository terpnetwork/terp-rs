//! Terp (morocco-1) <-> Osmosis (osmosis-1) Polytone wiring.
//!
//! Source of the Osmosis side: DAO-DAO's UI config,
//! github.com/DA0-DA0/dao-dao-ui `packages/utils/constants/polytone.json`
//! (last change e69e4cdfb538ee2f7bb662972e6a62ab90871401), loaded by
//! `packages/utils/constants/chains.ts` (3fbec83541513aae65868da481b87f44533b12c8).
//! DAO-DAO has no Terp entry, so there is no existing Terp<->Osmosis pair.
//! Every Osmosis voice DAO-DAO lists (11 pairs) is code 128 with
//! `{"proxy_code_id":"127","block_max_gas":"120000000"}`; every Osmosis note
//! is code 126; listeners are 125. Checked read-only against
//! lcd.osmosis.zone (code creator osmo1559zgk3mxm00qtr0zu2x5n4rh5vw704q4mp2tn).
//!
//! morocco-1 already stores the same four binaries (same checksums), uploaded
//! by gov with instantiate permission `Nobody`: note 4, listener 5, proxy 6,
//! voice 7. Those are the legacy `--via-gov` path only. The default is the
//! cw-v3 Polytone codes further down (Everybody, ids from cw-orch state).

use anyhow::{anyhow, Result};
use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgInstantiateContract2;
use cosmrs::Any;

pub const DAO_DAO_POLYTONE_JSON: &str = "https://github.com/DA0-DA0/dao-dao-ui/blob/e69e4cdfb538ee2f7bb662972e6a62ab90871401/packages/utils/constants/polytone.json";
pub const DAO_DAO_CHAINS_TS: &str = "https://github.com/DA0-DA0/dao-dao-ui/blob/3fbec83541513aae65868da481b87f44533b12c8/packages/utils/constants/chains.ts";

pub const BLOCK_MAX_GAS: &str = "120000000";
pub const CHANNEL_VERSION: &str = "polytone-1";

/// osmosis-1 code ids used by every DAO-DAO polytone pair.
pub const OSMO_LISTENER_CODE: u64 = 125;
pub const OSMO_NOTE_CODE: u64 = 126;
pub const OSMO_PROXY_CODE: u64 = 127;
pub const OSMO_VOICE_CODE: u64 = 128;
/// Reference pair from DAO-DAO (juno-1 note -> osmosis-1 voice), shape only.
pub const OSMO_REFERENCE_VOICE: &str = "osmo1af93h8xcszszes2a0kjms5zpm5ns3fys4aez2f40fgz428hc8aws28klzs";

/// morocco-1 code ids (gov-stored, instantiate permission Nobody).
pub const TERP_NOTE_CODE: u64 = 4;
pub const TERP_LISTENER_CODE: u64 = 5;
pub const TERP_PROXY_CODE: u64 = 6;
pub const TERP_VOICE_CODE: u64 = 7;

pub const NOTE_CHECKSUM: &str = "6284cfad75227c84ae407a57072bd7c3a599b31dd6c67d0a5e0a72fa9447a33f";
pub const LISTENER_CHECKSUM: &str = "7e4f66e87570d887a5ff8b2b1dcb5c5e254abeadef11a9f1ed517d90fa015481";
pub const PROXY_CHECKSUM: &str = "5094551145bcd256538775a0611ce9e88f8d7a182a06f082f901ffed9184bb5a";
pub const VOICE_CHECKSUM: &str = "7ae233f2f3a39d2d39828ca47e981a38a073435551646f8f6320c0fbb23e8c96";

pub const NOTE_SALT: &str = "polytone-note-to-osmosis-1";
pub const LISTENER_SALT: &str = "polytone-listener-from-osmosis-1";
pub const NOTE_LABEL: &str = "polytone_note_to_osmosis-1";
pub const LISTENER_LABEL: &str = "polytone_listener_from_osmosis-1";
pub const VOICE_LABEL: &str = "polytone_voice_from_morocco-1";

fn hex32(h: &str) -> Result<Vec<u8>> {
    let v = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()?;
    if v.len() != 32 {
        return Err(anyhow!("checksum must be 32 bytes"));
    }
    Ok(v)
}

/// instantiate2 address of a gov-created contract on morocco-1.
pub fn predict(gov: &str, checksum_hex: &str, salt: &str) -> Result<String> {
    let (_, creator) = bech32::decode(gov).map_err(|e| anyhow!("{e}"))?;
    cw_orch_kms::instantiate2_bech32("terp", &hex32(checksum_hex)?, &creator, salt.as_bytes())
        .map_err(|e| anyhow!(e))
}

pub struct TerpNote {
    pub note: String,
    pub listener: String,
}

pub fn predicted_terp_note(gov: &str) -> Result<TerpNote> {
    Ok(TerpNote {
        note: predict(gov, NOTE_CHECKSUM, NOTE_SALT)?,
        listener: predict(gov, LISTENER_CHECKSUM, LISTENER_SALT)?,
    })
}

/// Note init. `pair` = (morocco connection to osmosis-1, `wasm.<osmosis voice>`)
/// pins the note to that voice; without it the first channel handshake wins.
pub fn note_init(pair: Option<(&str, &str)>) -> serde_json::Value {
    let pair = pair.map(|(c, p)| serde_json::json!({"connection_id": c, "remote_port": p}));
    serde_json::json!({"pair": pair, "block_max_gas": BLOCK_MAX_GAS})
}

pub fn voice_init() -> serde_json::Value {
    serde_json::json!({"proxy_code_id": OSMO_PROXY_CODE.to_string(), "block_max_gas": BLOCK_MAX_GAS})
}

/// The two gov-sent instantiate2 messages (note, listener). Admin = gov module.
pub fn terp_note_msgs(gov: &str, pair: Option<(&str, &str)>) -> Result<Vec<Any>> {
    let p = predicted_terp_note(gov)?;
    let mk = |code_id, label: &str, msg: serde_json::Value, salt: &str| -> Result<Any> {
        Ok(crate::kms_gate::any(
            "/cosmwasm.wasm.v1.MsgInstantiateContract2",
            &MsgInstantiateContract2 {
                sender: gov.into(),
                admin: gov.into(),
                code_id,
                label: label.into(),
                msg: serde_json::to_vec(&msg)?,
                funds: vec![],
                salt: salt.as_bytes().to_vec(),
                fix_msg: false,
            },
        ))
    };
    Ok(vec![
        mk(TERP_NOTE_CODE, NOTE_LABEL, note_init(pair), NOTE_SALT)?,
        mk(TERP_LISTENER_CODE, LISTENER_LABEL, serde_json::json!({"note": p.note}), LISTENER_SALT)?,
    ])
}

/// `MsgSubmitProposal` (gov v1) carrying `messages`, deposit paid by `proposer`.
pub fn gov_submit_any(messages: Vec<Any>, proposer: &str, deposit: (u128, &str), title: &str, summary: &str) -> Result<Any> {
    use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
    use cosmos_sdk_proto::cosmos::gov::v1::MsgSubmitProposal;
    if title.is_empty() || title.len() > 255 || summary.is_empty() || messages.is_empty() {
        return Err(anyhow!("proposal needs a 1..=255 byte title, a summary and messages"));
    }
    Ok(crate::kms_gate::any(
        "/cosmos.gov.v1.MsgSubmitProposal",
        &MsgSubmitProposal {
            messages: messages.into_iter().map(|a| cosmos_sdk_proto::Any { type_url: a.type_url, value: a.value }).collect(),
            initial_deposit: vec![Coin { denom: deposit.1.into(), amount: deposit.0.to_string() }],
            proposer: proposer.into(),
            metadata: String::new(),
            title: title.into(),
            summary: summary.into(),
            expedited: false,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const GOV: &str = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";

    #[test]
    fn note_address_is_stable_and_listener_points_at_it() {
        let a = predicted_terp_note(GOV).unwrap();
        assert_eq!(a.note, predicted_terp_note(GOV).unwrap().note);
        assert!(a.note.starts_with("terp1") && a.note != a.listener);
        let msgs = terp_note_msgs(GOV, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|m| m.type_url == "/cosmwasm.wasm.v1.MsgInstantiateContract2"));
    }

    #[test]
    fn voice_init_matches_dao_dao_osmosis_voices() {
        assert_eq!(voice_init(), serde_json::json!({"proxy_code_id": "127", "block_max_gas": "120000000"}));
        assert_eq!(note_init(None)["pair"], serde_json::Value::Null);
    }
}

// ---------------------------------------------------------------------------
// cw-v3 Polytone (default). Source: crates/polytone-evm
// (github.com/permissionlessweb/polytone @ fac39bb2, "v3 wasm", cw2 1.0.2).
// Already on morocco-1 with instantiate permission Everybody, so no gov and no
// upload: the code ids come from cw-orch state (`morocco-1.code_ids`).

pub const V3_NOTE_CHECKSUM: &str = "4d83a8b17dada766fd0299b6dff08565127ba9e9f4347f8dafdf54e50d71ebfe";
pub const V3_VOICE_CHECKSUM: &str = "58608621b91adbb5e911188435945e85e8e9495f0301eeea03cb1818a662c33e";
pub const V3_PROXY_CHECKSUM: &str = "702805bd1c6300d2ea17ed74792df8bb2d233b0da68449a9f5d8e70d8e4e4724";
/// Built in polytone-evm/artifacts, not stored on morocco-1.
pub const V3_LISTENER_CHECKSUM: &str = "178338209f18a29a2432d141488433607d2e7ae2151b904bf00405b01c0be93c";
pub const V3_NOTE_SALT: &str = "polytone-v3-note-to-osmosis-1";
pub const V3_LISTENER_SALT: &str = "polytone-v3-listener-from-osmosis-1";

#[derive(Debug, Clone)]
pub struct StateCodes {
    pub source: String,
    pub note: u64,
    pub voice: Option<u64>,
    pub proxy: Option<u64>,
    pub listener: Option<u64>,
}

/// cw-orch state file: `$STATE_FILE`, then `~/.cw-orch/state.json`, then the
/// repo's `public/state.json`.
pub fn state_candidates() -> Vec<std::path::PathBuf> {
    let mut v = vec![];
    if let Ok(p) = std::env::var("STATE_FILE") {
        v.push(p.into());
    }
    if let Some(h) = std::env::var_os("HOME") {
        v.push(std::path::PathBuf::from(h).join(".cw-orch/state.json"));
    }
    v.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/state.json"));
    v
}

pub fn codes_from_state_json(state: &serde_json::Value, chain_id: &str, source: &str) -> Result<StateCodes> {
    let ids = &state[chain_id]["code_ids"];
    let id = |k: &str| ids[k].as_u64();
    Ok(StateCodes {
        source: source.into(),
        note: id("polytone:note").ok_or_else(|| anyhow!("{source}: no {chain_id}.code_ids.polytone:note"))?,
        voice: id("polytone:voice"),
        proxy: id("polytone:proxy"),
        listener: id("polytone:listener"),
    })
}

pub fn codes_from_state(chain_id: &str, explicit: Option<&std::path::Path>) -> Result<StateCodes> {
    let paths = match explicit {
        Some(p) => vec![p.to_path_buf()],
        None => state_candidates(),
    };
    let mut errs = vec![];
    for p in paths {
        match std::fs::read(&p).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<serde_json::Value>(&b)?)) {
            Ok(v) => match codes_from_state_json(&v, chain_id, &p.display().to_string()) {
                Ok(c) => return Ok(c),
                Err(e) => errs.push(e.to_string()),
            },
            Err(e) => errs.push(format!("{}: {e}", p.display())),
        }
    }
    Err(anyhow!("no polytone code ids for {chain_id}: {}", errs.join("; ")))
}

/// instantiate2 address for any creator.
pub fn predict_for(creator: &str, checksum_hex: &str, salt: &str) -> Result<String> {
    let (hrp, bytes) = bech32::decode(creator).map_err(|e| anyhow!("{e}"))?;
    cw_orch_kms::instantiate2_bech32(hrp.as_str(), &hex32(checksum_hex)?, &bytes, salt.as_bytes()).map_err(|e| anyhow!(e))
}

/// `MsgInstantiateContract2` sent by `sender` (the kms granter), admin = DAO.
pub fn instantiate2_any(sender: &str, admin: &str, code_id: u64, label: &str, msg: &serde_json::Value, salt: &str) -> Result<Any> {
    Ok(crate::kms_gate::any(
        "/cosmwasm.wasm.v1.MsgInstantiateContract2",
        &MsgInstantiateContract2 {
            sender: sender.into(),
            admin: admin.into(),
            code_id,
            label: label.into(),
            msg: serde_json::to_vec(msg)?,
            funds: vec![],
            salt: salt.as_bytes().to_vec(),
            fix_msg: false,
        },
    ))
}

#[cfg(test)]
mod v3_tests {
    use super::*;

    #[test]
    fn reads_code_ids_from_cw_orch_state() {
        let s = serde_json::json!({"morocco-1": {"code_ids": {"polytone:note": 78, "polytone:voice": 79, "polytone:proxy": 80}}});
        let c = codes_from_state_json(&s, "morocco-1", "t").unwrap();
        assert_eq!((c.note, c.voice, c.proxy, c.listener), (78, Some(79), Some(80), None));
        assert!(codes_from_state_json(&s, "osmosis-1", "t").is_err());
    }

    #[test]
    fn note_prediction_depends_on_creator() {
        let gov = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";
        let a = predict_for(gov, V3_NOTE_CHECKSUM, V3_NOTE_SALT).unwrap();
        assert!(a.starts_with("terp1"));
        assert_ne!(a, predict_for(gov, V3_NOTE_CHECKSUM, "other").unwrap());
    }
}
