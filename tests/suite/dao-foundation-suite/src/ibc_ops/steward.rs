//! Liquidity-steward DAOs.
//!
//! Foundation proposes four TERP DAOs and is not the admin of any of them.
//! `steward-akash` and `steward-juno` are the oversight DAOs. `custody-akash`
//! and `custody-juno` are vetoed by the matching steward contract. Each
//! custody later proposes one TERP subDAO. The Osmosis subDAO is a polytone
//! payload: the custody's remote account does not exist until the note
//! returns, so that instantiate is not placed in the same proposal.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use bech32::{Bech32, Hrp};
use cosmwasm_std::{
    Binary, CanonicalAddr, CosmosMsg, Uint256, WasmMsg, instantiate2_address, to_json_binary,
};
use cw_utils::Duration;
use cw4::Member;
use dao_interface::state::{Admin, ModuleInstantiateInfo};
use dao_voting::pre_propose::PreProposeInfo;
use dao_voting::threshold::Threshold;
use dao_voting::veto::VetoConfig;

use super::{
    CODE_CORE, CODE_CW4_GROUP, CODE_PROPOSAL_SINGLE, CODE_VOTING_CW4, foundation_proposal_wrapper,
};

/// 48 hours. Veto timelock and max voting period share this time unit.
pub const VETO_SECONDS: u64 = 48 * 60 * 60;

const VOTE_PERIOD: Duration = Duration::Time(VETO_SECONDS);

/// Polytone note execute that only opens the remote account.
pub const POLYTONE_TIMEOUT_SECONDS: u64 = 3600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StewardAccounts {
    pub custody_akash: String,
    pub custody_juno: String,
    pub steward_akash: String,
    pub steward_juno: String,
    pub sub_custody_akash_osmosis: String,
    pub sub_custody_akash_terp: String,
    pub sub_custody_juno_osmosis: String,
    pub sub_custody_juno_terp: String,
}

#[derive(Clone, Debug)]
pub struct RemoteCodeIds {
    pub core: u64,
    pub voting_cw4: u64,
    pub cw4_group: u64,
    pub proposal_single: u64,
}

#[derive(Clone, Debug)]
struct DaoInit {
    name: String,
    description: String,
    member: String,
    /// dao-dao-core admin. None means the contract becomes its own admin.
    dao_admin: Option<String>,
    /// CosmWasm migrate admin. None keeps the instantiator from owning it.
    wasm_admin: Option<String>,
    /// Proposal-module vetoer. None leaves veto off.
    vetoer: Option<String>,
    salt: String,
}

pub struct StewardPlan {
    pub checksum_hex: String,
    pub creator: String,
    /// Predicted core addresses, keyed by DAO name.
    pub addresses: BTreeMap<String, String>,
    pub foundation_proposal: serde_json::Value,
    pub subdaos: Vec<SubdaoFile>,
}

pub struct SubdaoFile {
    pub file_stem: String,
    pub body: serde_json::Value,
}

pub fn load_steward_accounts(path: &Path) -> Result<StewardAccounts, String> {
    let raw = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let mut by_name = BTreeMap::new();
    for row in rows {
        let name = row
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("steward account row missing name")?
            .to_string();
        let address = row
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("steward account {name} missing address"))?
            .to_string();
        if by_name.insert(name.clone(), address).is_some() {
            return Err(format!("duplicate steward account {name}"));
        }
    }
    let take = |name: &str| -> Result<String, String> {
        by_name
            .get(name)
            .cloned()
            .ok_or_else(|| format!("steward-accounts.json missing {name}"))
    };
    Ok(StewardAccounts {
        custody_akash: take("custody-akash")?,
        custody_juno: take("custody-juno")?,
        steward_akash: take("steward-akash")?,
        steward_juno: take("steward-juno")?,
        sub_custody_akash_osmosis: take("sub-custody-akash-osmosis")?,
        sub_custody_akash_terp: take("sub-custody-akash-terp")?,
        sub_custody_juno_osmosis: take("sub-custody-juno-osmosis")?,
        sub_custody_juno_terp: take("sub-custody-juno-terp")?,
    })
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim().trim_start_matches("0x");
    if s.len() % 2 != 0 {
        return Err("odd hex length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

pub fn decode_bech32(addr: &str) -> Result<(String, Vec<u8>), String> {
    let (hrp, data) = bech32::decode(addr).map_err(|e| format!("bech32 {addr}: {e}"))?;
    Ok((hrp.to_string(), data))
}

/// wasmd `BuildContractAddressPredictable` with `ContractAddrLen = 32`.
pub fn predictable_contract_address(
    checksum: &[u8],
    creator_canonical: &[u8],
    salt: &[u8],
    prefix: &str,
) -> Result<String, String> {
    let canon = instantiate2_address(checksum, &CanonicalAddr::from(creator_canonical), salt)
        .map_err(|e| format!("instantiate2 address: {e}"))?;
    let hrp = Hrp::parse(prefix).map_err(|e| e.to_string())?;
    bech32::encode::<Bech32>(hrp, canon.as_slice()).map_err(|e| e.to_string())
}

fn one_member(addr: &str) -> Vec<Member> {
    vec![Member {
        addr: addr.to_string(),
        weight: 1,
    }]
}

fn veto_config(vetoer: &str) -> VetoConfig {
    VetoConfig {
        timelock_duration: Duration::Time(VETO_SECONDS),
        vetoer: vetoer.to_string(),
        early_execute: false,
        veto_before_passed: true,
    }
}

fn dao_init_msg(
    spec: &DaoInit,
    voting_code: u64,
    group_code: u64,
    proposal_code: u64,
) -> dao_interface::msg::InstantiateMsg {
    let voting = ModuleInstantiateInfo {
        code_id: voting_code,
        msg: to_json_binary(&dao_voting_cw4::msg::InstantiateMsg {
            group_contract: dao_voting_cw4::msg::GroupContract::New {
                cw4_group_code_id: group_code,
                cw4_group_salt: None,
                initial_members: one_member(&spec.member),
            },
        })
        .expect("voting msg"),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: format!("{}-voting", spec.name),
        salt: None,
    };
    let proposal = ModuleInstantiateInfo {
        code_id: proposal_code,
        msg: to_json_binary(&dao_proposal_single::msg::InstantiateMsg {
            threshold: Threshold::AbsoluteCount {
                threshold: Uint256::from(1u64),
            },
            max_voting_period: VOTE_PERIOD,
            min_voting_period: None,
            only_members_execute: true,
            allow_revoting: false,
            pre_propose_info: PreProposeInfo::AnyoneMayPropose {},
            close_proposal_on_execution_failure: true,
            veto: spec.vetoer.as_deref().map(veto_config),
            delegation_module: None,
        })
        .expect("proposal msg"),
        admin: Some(Admin::CoreModule {}),
        funds: None,
        label: format!("{}-proposal", spec.name),
        salt: None,
    };
    dao_interface::msg::InstantiateMsg {
        admin: spec.dao_admin.clone(),
        name: spec.name.clone(),
        description: spec.description.clone(),
        image_url: None,
        automatically_add_cw20s: true,
        automatically_add_cw721s: true,
        voting_module_instantiate_info: voting,
        proposal_modules_instantiate_info: vec![proposal],
        initial_items: None,
        initial_actions: None,
        dao_uri: None,
    }
}

fn instantiate2(spec: &DaoInit) -> CosmosMsg {
    CosmosMsg::Wasm(WasmMsg::Instantiate2 {
        admin: spec.wasm_admin.clone(),
        code_id: CODE_CORE,
        msg: to_json_binary(&dao_init_msg(
            spec,
            CODE_VOTING_CW4,
            CODE_CW4_GROUP,
            CODE_PROPOSAL_SINGLE,
        ))
        .expect("core msg"),
        funds: vec![],
        label: spec.name.clone(),
        salt: Binary::from(spec.salt.as_bytes()),
    })
}

fn instantiate_by_parent(spec: &DaoInit) -> CosmosMsg {
    CosmosMsg::Wasm(WasmMsg::Instantiate {
        admin: spec.wasm_admin.clone(),
        code_id: CODE_CORE,
        msg: to_json_binary(&dao_init_msg(
            spec,
            CODE_VOTING_CW4,
            CODE_CW4_GROUP,
            CODE_PROPOSAL_SINGLE,
        ))
        .expect("core msg"),
        funds: vec![],
        label: spec.name.clone(),
    })
}

fn parent_text(parent: Option<&str>) -> String {
    match parent {
        Some(p) => format!("Its parent (dao admin and wasm admin) is the Terp x/gov module account {p}; the Foundation is not the parent."),
        None => "Foundation is not the parent.".into(),
    }
}

fn oversight(name: &str, chain: &str, member: &str, parent: Option<&str>) -> DaoInit {
    DaoInit {
        name: name.into(),
        description: format!(
            "Liquidity steward oversight for {chain}. Foundation creates this DAO. {} One cw4 member ({member}) adds the rest once the committee is final.", parent_text(parent)
        ),
        member: member.into(),
        dao_admin: parent.map(String::from),
        wasm_admin: parent.map(String::from),
        vetoer: None,
        salt: name.into(),
    }
}

fn custody(name: &str, chain: &str, member: &str, steward_contract: &str, parent: Option<&str>) -> DaoInit {
    DaoInit {
        name: name.into(),
        description: format!(
            "Custody DAO for {chain} liquidity. {steward_contract} can veto a proposal for 48 hours, including before it passes. Early execute is off. {} One cw4 member ({member}).", parent_text(parent)
        ),
        member: member.into(),
        dao_admin: parent.map(String::from),
        wasm_admin: parent.map(String::from),
        vetoer: Some(steward_contract.into()),
        salt: name.into(),
    }
}

fn terp_subdao(name: &str, chain: &str, member: &str, custody_contract: &str) -> DaoInit {
    DaoInit {
        name: name.into(),
        description: format!(
            "TERP subDAO of the {chain} custody DAO {custody_contract}. The custody DAO is the parent and the 48-hour vetoer. One cw4 member ({member})."
        ),
        member: member.into(),
        dao_admin: Some(custody_contract.into()),
        wasm_admin: Some(custody_contract.into()),
        vetoer: Some(custody_contract.into()),
        salt: name.into(),
    }
}

/// Note `Execute` with an empty message list. The voice creates the custody's remote account.
pub fn polytone_open_account(note: &str) -> CosmosMsg {
    let msg = serde_json::json!({
        "execute": {
            "msgs": [],
            "callback": null,
            "timeout_seconds": POLYTONE_TIMEOUT_SECONDS.to_string(),
        }
    });
    CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: note.to_string(),
        msg: to_json_binary(&msg).expect("note execute"),
        funds: vec![],
    })
}

fn remote_design(
    name: &str,
    chain: &str,
    member: &str,
    codes: Option<&RemoteCodeIds>,
) -> serde_json::Value {
    let placeholder = format!("POLYTONE_PROXY_OF_{name}");
    let wasm_admin = placeholder.clone();
    let init = codes.map(|codes| {
        dao_init_msg(
            &DaoInit {
                name: name.into(),
                description: format!(
                    "Remote subDAO of the {chain} custody DAO, instantiated on Osmosis through polytone. The parent and 48-hour vetoer are the custody DAO's polytone account. Initial member is {member}."
                ),
                member: member.into(),
                dao_admin: Some(placeholder.clone()),
                wasm_admin: Some(placeholder.clone()),
                vetoer: Some(placeholder),
                salt: name.into(),
            },
            codes.voting_cw4,
            codes.cw4_group,
            codes.proposal_single,
        )
    });
    serde_json::json!({
        "remote_chain": "osmosis",
        "committee": chain,
        "initial_member": member,
        "member_prefix": "terp",
        "code_ids": codes.map(|c| serde_json::json!({
            "core": c.core,
            "voting_cw4": c.voting_cw4,
            "cw4_group": c.cw4_group,
            "proposal_single": c.proposal_single,
        })),
        "wasm_admin": wasm_admin,
        "instantiate_msg": init,
        "ready": false,
        "blocker": "Do not broadcast this instantiate yet. Osmosis rejects a terp bech32 member, and the wasm admin must be the custody DAO's polytone remote address. Query the note RemoteAddress after the open-account proposal, then replace the member and both admin fields.",
    })
}

pub fn build_steward_plan(
    accounts: &StewardAccounts,
    creator_bech32: &str,
    checksum: &[u8],
    polytone_note: Option<&str>,
    remote_codes: Option<&RemoteCodeIds>,
) -> Result<StewardPlan, String> {
    build_steward_plan_with_parent(accounts, creator_bech32, checksum, polytone_note, remote_codes, None)
}

/// Same plan, with `parent` (the Terp x/gov module account) as dao admin and
/// wasm admin of the two steward and two custody DAOs. `None` keeps them self-admin.
pub fn build_steward_plan_with_parent(
    accounts: &StewardAccounts,
    creator_bech32: &str,
    checksum: &[u8],
    polytone_note: Option<&str>,
    remote_codes: Option<&RemoteCodeIds>,
    parent: Option<&str>,
) -> Result<StewardPlan, String> {
    if checksum.len() != 32 {
        return Err(format!(
            "code checksum is {} bytes, expected 32",
            checksum.len()
        ));
    }
    let (hrp, creator) = decode_bech32(creator_bech32)?;
    if hrp != "terp" {
        return Err(format!("creator prefix is {hrp}, expected terp"));
    }
    let predict = |salt: &str| -> Result<String, String> {
        predictable_contract_address(checksum, &creator, salt.as_bytes(), "terp")
    };

    let steward_akash = predict("steward-akash")?;
    let steward_juno = predict("steward-juno")?;
    let custody_akash = predict("custody-akash")?;
    let custody_juno = predict("custody-juno")?;

    let mut addresses = BTreeMap::new();
    addresses.insert("steward-akash".into(), steward_akash.clone());
    addresses.insert("steward-juno".into(), steward_juno.clone());
    addresses.insert("custody-akash".into(), custody_akash.clone());
    addresses.insert("custody-juno".into(), custody_juno.clone());

    let foundation_msgs = vec![
        instantiate2(&oversight(
            "steward-akash",
            "Akash",
            &accounts.steward_akash,
            parent,
        )),
        instantiate2(&oversight("steward-juno", "Juno", &accounts.steward_juno, parent)),
        instantiate2(&custody(
            "custody-akash",
            "Akash",
            &accounts.custody_akash,
            &steward_akash,
            parent,
        )),
        instantiate2(&custody(
            "custody-juno",
            "Juno",
            &accounts.custody_juno,
            &steward_juno,
            parent,
        )),
    ];
    let admin_text = match parent {
        Some(p) => format!("Foundation is not the parent of any of them: dao admin and wasm admin are the Terp x/gov module account {p}."),
        None => "Foundation is not the parent of any of them: dao admin and wasm admin are unset, so each core becomes its own admin.".into(),
    };
    let foundation_proposal = foundation_proposal_wrapper(
        "Create Akash and Juno steward and custody DAOs",
        &format!("Foundation instantiates two oversight DAOs and two custody DAOs. {admin_text} custody-akash is vetoed by the steward-akash contract and custody-juno by the steward-juno contract. The veto window is 48 hours, the vetoer may veto before passage, and early execute is off. Each DAO has one cw4 member from steward-accounts.json. SubDAOs are separate proposals those custody DAOs submit after this executes."),
        foundation_msgs,
    );

    let subdaos = vec![
        subdao_file(
            "akash",
            &custody_akash,
            &accounts.sub_custody_akash_terp,
            &accounts.sub_custody_akash_osmosis,
            polytone_note,
            remote_codes,
        ),
        subdao_file(
            "juno",
            &custody_juno,
            &accounts.sub_custody_juno_terp,
            &accounts.sub_custody_juno_osmosis,
            polytone_note,
            remote_codes,
        ),
    ];

    Ok(StewardPlan {
        checksum_hex: hex_encode(checksum),
        creator: creator_bech32.to_string(),
        addresses,
        foundation_proposal,
        subdaos,
    })
}

fn subdao_file(
    chain: &str,
    custody_contract: &str,
    terp_member: &str,
    osmosis_member: &str,
    note: Option<&str>,
    remote_codes: Option<&RemoteCodeIds>,
) -> SubdaoFile {
    let terp_name = format!("sub-custody-{chain}-terp");
    let osmosis_name = format!("sub-custody-{chain}-osmosis");
    let chain_label = match chain {
        "akash" => "Akash",
        "juno" => "Juno",
        other => other,
    };
    let terp = instantiate_by_parent(&terp_subdao(
        &terp_name,
        chain_label,
        terp_member,
        custody_contract,
    ));
    let open_account = note.map(|note| {
        foundation_proposal_wrapper(
            &format!("Open the {chain_label} custody polytone account on Osmosis"),
            "Empty polytone execute. The voice creates this custody DAO's remote account and does not instantiate the subDAO. The remote address is queried after the packet returns.",
            vec![polytone_open_account(note)],
        )
    });
    SubdaoFile {
        file_stem: format!("custody-{chain}-subdaos"),
        body: serde_json::json!({
            "parent": custody_contract,
            "submit_to": "pre-propose module of the parent. Query {\"proposal_modules\":{}} on the parent after the foundation proposal executes.",
            "submit_as": "the single cw4 member of the parent custody DAO",
            "terp_subdao": {
                "name": terp_name,
                "member": terp_member,
                "ready": true,
                "propose": foundation_proposal_wrapper(
                    &format!("Create the {chain_label} custody TERP subDAO"),
                    "The custody DAO instantiates its TERP subDAO. The custody contract is the dao admin, the wasm admin, and the 48-hour vetoer. One cw4 member.",
                    vec![terp],
                ),
            },
            "osmosis_open_account": {
                "ready": open_account.is_some(),
                "note": note,
                "propose": open_account,
                "blocker": if note.is_some() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!("Pass --polytone-note. No Osmosis note is deployed from this script.")
                },
            },
            "osmosis_subdao": remote_design(&osmosis_name, chain_label, osmosis_member, remote_codes),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tuned structure (default since the new Terp<->Juno / Terp<->Osmosis routes).
//
// Terp holds three DAOs: steward-akash, custody-akash (vetoed by steward-akash),
// and steward-juno. The Juno custody DAO lives on juno-1: steward-juno creates
// it through its own Polytone proxy on Juno. On Juno the x/gov module account is
// wasm admin, dao admin and 48-hour vetoer, and the proxy is the only cw4 member.
// custody-akash opens a Polytone account it owns on Osmosis.

/// juno-1 DAO-DAO v2.7.0 code ids (dao-dao-ui codeIds.json, checked on chain).
pub const JUNO_DAO_CODES: RemoteCodeIds = RemoteCodeIds {
    core: 4862,
    voting_cw4: 4872,
    cw4_group: 1992,
    proposal_single: 4869,
};
/// juno-1 x/gov module account (auth ModuleAccountByName "gov").
pub const JUNO_GOV: &str = "juno10d07y265gmmuvt4z0w9aw880jnsr700jvss730";

pub struct JunoRemote<'a> {
    /// Terp note pinned to the new Terp<->Juno connection.
    pub note: Option<&'a str>,
    /// steward-juno's proxy on Juno, when it can be predicted (voice and Juno-side connection known).
    pub proxy: Option<&'a str>,
    pub juno_gov: &'a str,
    pub codes: RemoteCodeIds,
}

/// Note `Execute` carrying `msgs` for the remote proxy.
pub fn polytone_execute(note: &str, msgs: Vec<CosmosMsg>) -> CosmosMsg {
    let msg = serde_json::json!({
        "execute": {
            "msgs": msgs,
            "callback": null,
            "timeout_seconds": POLYTONE_TIMEOUT_SECONDS.to_string(),
        }
    });
    CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: note.to_string(),
        msg: to_json_binary(&msg).expect("note execute"),
        funds: vec![],
    })
}

/// The custody-juno instantiate executed on juno-1 by steward-juno's proxy.
pub fn juno_custody_instantiate(proxy: &str, juno_gov: &str, codes: &RemoteCodeIds) -> CosmosMsg {
    let spec = DaoInit {
        name: "custody-juno".into(),
        description: format!(
            "Custody DAO for Juno liquidity, created on Juno by the Terp steward-juno DAO through Polytone. The Juno x/gov module account {juno_gov} is the dao admin, the wasm admin and the 48-hour vetoer (veto before passage on, early execute off). Sole cw4 member: the steward-juno Polytone account {proxy}."
        ),
        member: proxy.into(),
        dao_admin: Some(juno_gov.into()),
        wasm_admin: Some(juno_gov.into()),
        vetoer: Some(juno_gov.into()),
        salt: "custody-juno".into(),
    };
    CosmosMsg::Wasm(WasmMsg::Instantiate {
        admin: Some(juno_gov.into()),
        code_id: codes.core,
        msg: to_json_binary(&dao_init_msg(&spec, codes.voting_cw4, codes.cw4_group, codes.proposal_single))
            .expect("core msg"),
        funds: vec![],
        label: "custody-juno".into(),
    })
}

pub fn build_steward_plan_tuned(
    accounts: &StewardAccounts,
    creator_bech32: &str,
    checksum: &[u8],
    osmosis_note: Option<&str>,
    parent: Option<&str>,
    juno: &JunoRemote,
) -> Result<StewardPlan, String> {
    if checksum.len() != 32 {
        return Err(format!("code checksum is {} bytes, expected 32", checksum.len()));
    }
    let (hrp, creator) = decode_bech32(creator_bech32)?;
    if hrp != "terp" {
        return Err(format!("creator prefix is {hrp}, expected terp"));
    }
    let predict = |salt: &str| predictable_contract_address(checksum, &creator, salt.as_bytes(), "terp");
    let steward_akash = predict("steward-akash")?;
    let steward_juno = predict("steward-juno")?;
    let custody_akash = predict("custody-akash")?;
    let mut addresses = BTreeMap::new();
    addresses.insert("steward-akash".into(), steward_akash.clone());
    addresses.insert("steward-juno".into(), steward_juno.clone());
    addresses.insert("custody-akash".into(), custody_akash.clone());

    let msgs = vec![
        instantiate2(&oversight("steward-akash", "Akash", &accounts.steward_akash, parent)),
        instantiate2(&oversight("steward-juno", "Juno", &accounts.steward_juno, parent)),
        instantiate2(&custody("custody-akash", "Akash", &accounts.custody_akash, &steward_akash, parent)),
    ];
    let admin_text = match parent {
        Some(p) => format!("The Foundation is not their parent: dao admin and wasm admin are the Terp x/gov module account {p}."),
        None => "The Foundation is not their parent: each core is its own admin.".into(),
    };
    let foundation_proposal = foundation_proposal_wrapper(
        "Create the Akash and Juno steward DAOs and the Akash custody DAO",
        &format!("Foundation instantiates steward-akash, steward-juno and custody-akash on Terp. {admin_text} steward-akash can veto custody-akash for 48 hours, including before a proposal passes; early execute is off. Each DAO has one cw4 member from steward-accounts.json. steward-juno later creates the Juno custody DAO on Juno through Polytone, and custody-akash opens a Polytone account on Osmosis; both are separate proposals of those DAOs."),
        msgs,
    );

    let mut subdaos = vec![subdao_file(
        "akash",
        &custody_akash,
        &accounts.sub_custody_akash_terp,
        &accounts.sub_custody_akash_osmosis,
        osmosis_note,
        None,
    )];
    subdaos.push(SubdaoFile {
        file_stem: "custody-akash-osmosis-account".into(),
        body: serde_json::json!({
            "dao": custody_akash,
            "submit_to": "custody-akash proposal module (AnyoneMayPropose); vote and execute as its cw4 member. steward-akash can veto for 48h.",
            "route": "Terp polytone note pinned to the new Terp<->Osmosis connection (client 07-tendermint-44) -> Osmosis voice",
            "note": osmosis_note,
            "ready": osmosis_note.is_some(),
            "blocker": if osmosis_note.is_some() { serde_json::Value::Null } else { serde_json::json!("Osmosis note not deployed: needs the Terp<->Osmosis connection, voice and polytone channel (Hermes), then polytone_note --remote osmosis.") },
            "propose": osmosis_note.map(|n| foundation_proposal_wrapper(
                "Open the custody-akash Polytone account on Osmosis",
                "Empty Polytone execute through the Terp<->Osmosis note. The Osmosis voice creates a proxy owned by this DAO. Read its address afterwards with the note's remote_address query.",
                vec![polytone_open_account(n)],
            )),
        }),
    });

    let placeholder = "POLYTONE_PROXY_OF_steward-juno";
    let proxy = juno.proxy.unwrap_or(placeholder);
    let create = juno_custody_instantiate(proxy, juno.juno_gov, &juno.codes);
    let ready = juno.note.is_some() && juno.proxy.is_some();
    let note = juno.note.unwrap_or("POLYTONE_NOTE_TO_juno-1");
    subdaos.push(SubdaoFile {
        file_stem: "steward-juno-custody-on-juno".into(),
        body: serde_json::json!({
            "dao": steward_juno,
            "submit_to": "steward-juno proposal module (AnyoneMayPropose); vote and execute as its cw4 member",
            "route": "Terp polytone note pinned to the new Terp<->Juno connection (client 07-tendermint-46) -> Juno voice",
            "juno": {"chain_id": "juno-1", "gov_module": juno.juno_gov, "code_ids": {"core": juno.codes.core, "voting_cw4": juno.codes.voting_cw4, "cw4_group": juno.codes.cw4_group, "proposal_single": juno.codes.proposal_single}, "code_source": "DAO-DAO v2.7.0"},
            "steward_juno_proxy": juno.proxy,
            "note": juno.note,
            "ready": ready,
            "blocker": if ready { serde_json::Value::Null } else { serde_json::json!("Needs the Juno-side client/connection on 07-tendermint-46, a Juno voice, the Terp note pinned to it and the polytone channel. Then rerun with --juno-note, --juno-voice and --juno-connection so the proxy is predicted; or submit open_account first and read the proxy from the note's remote_address query.") },
            "open_account": juno.note.map(|n| foundation_proposal_wrapper(
                "Open the steward-juno Polytone account on Juno",
                "Empty Polytone execute through the Terp<->Juno note. The Juno voice creates the proxy owned by steward-juno. Only needed if the proxy address was not predicted.",
                vec![polytone_open_account(n)],
            )),
            "create_custody": foundation_proposal_wrapper(
                "Create custody-juno on Juno",
                &format!("Through Polytone, steward-juno's Juno account instantiates custody-juno (DAO-DAO v2.7.0 code {}). The Juno x/gov module account {} is wasm admin, dao admin and 48-hour vetoer. Sole cw4 member: the steward-juno Polytone account {}, weight 1.", juno.codes.core, juno.juno_gov, proxy),
                vec![polytone_execute(note, vec![create])],
            ),
        }),
    });

    Ok(StewardPlan {
        checksum_hex: hex_encode(checksum),
        creator: creator_bech32.to_string(),
        addresses,
        foundation_proposal,
        subdaos,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

pub fn assert_no_foundation_admin(msg: &CosmosMsg, foundation: &str) -> Result<(), String> {
    let v = serde_json::to_value(msg).map_err(|e| e.to_string())?;
    let inst = v
        .get("wasm")
        .and_then(|w| w.get("instantiate2").or_else(|| w.get("instantiate")))
        .ok_or("not a wasm instantiate")?;
    if inst.get("admin").and_then(|a| a.as_str()) == Some(foundation) {
        return Err("wasm admin is the foundation DAO".into());
    }
    let raw = inst
        .get("msg")
        .and_then(|m| m.as_str())
        .ok_or("instantiate msg missing")?;
    let bytes = Binary::from_base64(raw).map_err(|e| e.to_string())?;
    let init: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if init.get("admin").and_then(|a| a.as_str()) == Some(foundation) {
        return Err("dao admin is the foundation DAO".into());
    }
    Ok(())
}

/// Steward guard: Foundation is never wasm or DAO admin; the only accepted
/// admin is `parent` (the x/gov module account), or unset when `parent` is None.
pub fn assert_steward_admin(msg: &CosmosMsg, foundation: &str, parent: Option<&str>) -> Result<(), String> {
    assert_no_foundation_admin(msg, foundation)?;
    let v = serde_json::to_value(msg).map_err(|e| e.to_string())?;
    let inst = v.get("wasm").and_then(|w| w.get("instantiate2").or_else(|| w.get("instantiate"))).ok_or("not a wasm instantiate")?;
    let bytes = Binary::from_base64(inst.get("msg").and_then(|m| m.as_str()).ok_or("instantiate msg missing")?).map_err(|e| e.to_string())?;
    let init: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    for (what, got) in [("wasm admin", inst.get("admin")), ("dao admin", init.get("admin"))] {
        let got = got.and_then(|a| a.as_str());
        if got != parent {
            return Err(format!("{what} is {got:?}, expected {parent:?} (gov module parent)"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ibc_ops::PARENT_CORE;

    fn golden_checksum() -> Vec<u8> {
        decode_hex("13a1fc994cc6d1c81b746ee0c0ff6f90043875e0bf1d9be6b7d779fc978dc2a5").unwrap()
    }

    #[test]
    fn instantiate2_matches_wasmd_32_byte_vector() {
        let creator = decode_hex("9999999999aaaaaaaaaabbbbbbbbbbcccccccccc").unwrap();
        let addr =
            predictable_contract_address(&golden_checksum(), &creator, &[0x61], "purple").unwrap();
        assert_eq!(
            addr,
            "purple1t6r960j945lfv8mhl4mage2rg97w63xeynwrupum2s2l7em4lprs9ce5hk"
        );
    }

    #[test]
    fn accounts_file_is_the_initial_member_of_each_dao() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../steward-accounts.json");
        let accounts = load_steward_accounts(&path).unwrap();
        let plan =
            build_steward_plan(&accounts, PARENT_CORE, &golden_checksum(), None, None).unwrap();
        assert_eq!(plan.addresses.len(), 4);
        let proposal = &plan.foundation_proposal["propose"]["msg"]["propose"];
        let msgs = proposal["msgs"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        let members = [
            &accounts.steward_akash,
            &accounts.steward_juno,
            &accounts.custody_akash,
            &accounts.custody_juno,
        ];
        for (msg, member) in msgs.iter().zip(members) {
            let raw = msg["wasm"]["instantiate2"]["msg"].as_str().unwrap();
            let init: serde_json::Value =
                serde_json::from_slice(&Binary::from_base64(raw).unwrap()).unwrap();
            assert!(init["admin"].is_null(), "foundation must not be dao admin");
            assert!(msg["wasm"]["instantiate2"]["admin"].is_null());
            let voting: serde_json::Value = serde_json::from_slice(
                &Binary::from_base64(
                    init["voting_module_instantiate_info"]["msg"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            let initial = &voting["group_contract"]["new"]["initial_members"];
            assert_eq!(initial.as_array().unwrap().len(), 1);
            assert_eq!(initial[0]["addr"], member.as_str());
            assert_eq!(initial[0]["weight"], 1);
        }

        let custody_init = decode_init(&msgs[2]);
        let proposal_msg: serde_json::Value = serde_json::from_slice(
            &Binary::from_base64(
                custody_init["proposal_modules_instantiate_info"][0]["msg"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            proposal_msg["veto"]["timelock_duration"]["time"],
            VETO_SECONDS
        );
        assert_eq!(
            proposal_msg["veto"]["vetoer"],
            plan.addresses["steward-akash"]
        );
        assert_eq!(proposal_msg["veto"]["early_execute"], false);
        assert_eq!(proposal_msg["veto"]["veto_before_passed"], true);
        assert_eq!(proposal_msg["max_voting_period"]["time"], VETO_SECONDS);

        let steward_init = decode_init(&msgs[0]);
        let steward_prop: serde_json::Value = serde_json::from_slice(
            &Binary::from_base64(
                steward_init["proposal_modules_instantiate_info"][0]["msg"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(steward_prop["veto"].is_null());

        let akash = &plan.subdaos[0].body;
        assert_eq!(akash["parent"], plan.addresses["custody-akash"]);
        assert_eq!(
            akash["terp_subdao"]["member"],
            accounts.sub_custody_akash_terp
        );
        assert_eq!(akash["terp_subdao"]["ready"], true);
        assert_eq!(
            akash["osmosis_subdao"]["initial_member"],
            accounts.sub_custody_akash_osmosis
        );
        assert_eq!(akash["osmosis_subdao"]["remote_chain"], "osmosis");
        assert_eq!(akash["osmosis_subdao"]["ready"], false);
        let terp_msgs = &akash["terp_subdao"]["propose"]["propose"]["msg"]["propose"]["msgs"];
        let sub_init = decode_init(&terp_msgs[0]);
        assert_eq!(sub_init["admin"], plan.addresses["custody-akash"]);
        assert_eq!(
            terp_msgs[0]["wasm"]["instantiate"]["admin"],
            plan.addresses["custody-akash"]
        );
        for msg in msgs {
            let cosmos: CosmosMsg = serde_json::from_value(msg.clone()).unwrap();
            assert_no_foundation_admin(&cosmos, PARENT_CORE).unwrap();
        }
    }

    #[test]
    fn tuned_plan_drops_terp_custody_juno_and_builds_juno_custody() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../steward-accounts.json");
        let accounts = load_steward_accounts(&path).unwrap();
        let gov = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";
        let proxy = "juno1proxyproxyproxy";
        let juno = JunoRemote { note: Some("terp1note"), proxy: Some(proxy), juno_gov: JUNO_GOV, codes: JUNO_DAO_CODES };
        let plan = build_steward_plan_tuned(&accounts, PARENT_CORE, &golden_checksum(), Some("terp1osmonote"), Some(gov), &juno).unwrap();
        assert_eq!(plan.addresses.len(), 3);
        assert!(!plan.addresses.contains_key("custody-juno"));
        let msgs = plan.foundation_proposal["propose"]["msg"]["propose"]["msgs"].as_array().unwrap().clone();
        assert_eq!(msgs.len(), 3);
        for m in &msgs {
            let c: CosmosMsg = serde_json::from_value(m.clone()).unwrap();
            assert_steward_admin(&c, PARENT_CORE, Some(gov)).unwrap();
        }
        let j = &plan.subdaos.iter().find(|s| s.file_stem == "steward-juno-custody-on-juno").unwrap().body;
        assert_eq!(j["ready"], true);
        let note_msg = &j["create_custody"]["propose"]["msg"]["propose"]["msgs"][0]["wasm"]["execute"];
        assert_eq!(note_msg["contract_addr"], "terp1note");
        let exec: serde_json::Value = serde_json::from_slice(&Binary::from_base64(note_msg["msg"].as_str().unwrap()).unwrap()).unwrap();
        let inst = &exec["execute"]["msgs"][0];
        assert_eq!(inst["wasm"]["instantiate"]["admin"], JUNO_GOV);
        assert_eq!(inst["wasm"]["instantiate"]["code_id"], 4862);
        let init = decode_init(inst);
        assert_eq!(init["admin"], JUNO_GOV);
        let prop: serde_json::Value = serde_json::from_slice(&Binary::from_base64(init["proposal_modules_instantiate_info"][0]["msg"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(prop["veto"]["vetoer"], JUNO_GOV);
        let voting: serde_json::Value = serde_json::from_slice(&Binary::from_base64(init["voting_module_instantiate_info"]["msg"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(voting["group_contract"]["new"]["initial_members"], serde_json::json!([{"addr": proxy, "weight": 1}]));
        let a = &plan.subdaos.iter().find(|s| s.file_stem == "custody-akash-osmosis-account").unwrap().body;
        assert_eq!(a["dao"], plan.addresses["custody-akash"]);
        assert_eq!(a["ready"], true);
    }

    fn decode_init(msg: &serde_json::Value) -> serde_json::Value {
        let wasm = &msg["wasm"];
        let raw = wasm["instantiate2"]["msg"]
            .as_str()
            .or_else(|| wasm["instantiate"]["msg"].as_str())
            .unwrap();
        serde_json::from_slice(&Binary::from_base64(raw).unwrap()).unwrap()
    }
}
