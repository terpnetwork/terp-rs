//! FROST group-vk rotate contract.
//!
//! `MsgExecuteContract { rotate_frost_pubkey }` is always **Ok + write**
//! (pending or fail). t-of-n matching shares apply `group_vk`.
//! There is **no** `sudo` / Authenticate entry — CosmwasmAuthenticatorV1
//! discards writes and is forbidden for inbox and rotate.

use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    entry_point, to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use sha2::{Digest, Sha256};

const CONTRACT_NAME: &str = "crates.io:terp-frost";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Bind prefix — must match onboard-authz `itl_frost_rotate::HKDF_INFO`.
pub const HKDF_INFO: &[u8] = b"itl-frost-rotate-v1";

const CONFIG: Item<Config> = Item::new("config");
const PROPOSAL: Item<RotateProposal> = Item::new("proposal");
const LOG: Item<Vec<RotateEvent>> = Item::new("log");

#[cw_serde]
pub struct InstantiateMsg {
    pub group_vk: String,
    pub t: u16,
    pub n: u16,
    pub seats: Vec<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Invoke a rotation. Always Ok+write (pending or fail).
    RotateFrostPubkey {
        new_vk_commit: String,
        hkdf_commit: Option<String>,
        /// Players for the next epoch. Omitted = keep current seats/t/n.
        next_seats: Option<Vec<String>>,
        next_t: Option<u16>,
        next_n: Option<u16>,
    },
    /// Seat share bound to the pending commit. Always Ok+write.
    AddRotateShare {
        new_vk: String,
        hkdf_commit: Option<String>,
        blob: String,
        admin_id: Option<String>,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(GroupVkResponse)]
    GroupVk {},
    #[returns(PendingResponse)]
    Pending {},
    #[returns(StatusResponse)]
    Status {},
}

#[cw_serde]
pub struct GroupVkResponse {
    pub group_vk: String,
    pub old_vk: Option<String>,
}

#[cw_serde]
pub struct PendingResponse {
    pub pending: bool,
    pub new_vk_commit: Option<String>,
    pub share_count: u32,
    pub shares: Vec<String>,
    pub status: String,
}

#[cw_serde]
pub struct StatusResponse {
    pub group_vk: String,
    pub old_vk: Option<String>,
    pub t: u16,
    pub n: u16,
    pub pending: bool,
    pub new_vk_commit: Option<String>,
    pub share_count: u32,
    pub shares: Vec<String>,
    pub seats: Vec<String>,
    pub last_reason: Option<String>,
}

#[cw_serde]
pub struct Config {
    pub group_vk: String,
    pub old_vk: Option<String>,
    pub t: u16,
    pub n: u16,
    pub seats: Vec<String>,
    pub owner: String,
}

#[cw_serde]
pub struct RotateProposal {
    pub new_vk_commit: String,
    pub hkdf_commit: Option<String>,
    pub initiator: String,
    pub status: String,
    pub shares: Vec<String>,
    pub new_vk: Option<String>,
    pub next_seats: Vec<String>,
    pub next_t: u16,
    pub next_n: u16,
}

#[cw_serde]
pub struct RotateEvent {
    pub admin: String,
    pub kind: String,
    pub reason: Option<String>,
}

pub fn vk_commit(group_vk: &str) -> String {
    let mut h = Sha256::new();
    h.update(HKDF_INFO);
    h.update(group_vk.as_bytes());
    hex32(&h.finalize())
}

fn hex32(bytes: &[u8]) -> String {
    const H: &[u8] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0xf) as usize] as char);
    }
    s
}

fn append_log(deps: DepsMut, admin: &str, kind: &str, reason: Option<&str>) -> StdResult<()> {
    let mut log = LOG.may_load(deps.storage)?.unwrap_or_default();
    log.push(RotateEvent {
        admin: admin.into(),
        kind: kind.into(),
        reason: reason.map(|s| s.into()),
    });
    if log.len() > 32 {
        let skip = log.len() - 32;
        log = log[skip..].to_vec();
    }
    LOG.save(deps.storage, &log)
}

fn ok_write(action: &str, status: &str, admin: &str, reason: Option<&str>) -> Response {
    let mut r = Response::new()
        .add_attribute("action", action)
        .add_attribute("status", status)
        .add_attribute("admin", admin);
    if let Some(reason) = reason {
        r = r.add_attribute("reason", reason);
    }
    r
}

fn seat_of(cfg: &Config, admin: &str) -> bool {
    cfg.seats.iter().any(|s| s == admin)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    if msg.group_vk.trim().is_empty() {
        return Err(cosmwasm_std::StdError::msg("empty_vk"));
    }
    let t = if msg.t == 0 { 2 } else { msg.t };
    let n = if msg.n == 0 { 3 } else { msg.n };
    if t == 0 || t > n {
        return Err(cosmwasm_std::StdError::msg("bad_threshold"));
    }
    let seats = if msg.seats.is_empty() {
        vec!["desk".into(), "steward".into(), "spare".into()]
    } else {
        msg.seats
    };
    let cfg = Config {
        group_vk: msg.group_vk,
        old_vk: None,
        t,
        n,
        seats,
        owner: info.sender.to_string(),
    };
    CONFIG.save(deps.storage, &cfg)?;
    LOG.save(deps.storage, &Vec::new())?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("group_vk", cfg.group_vk))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> StdResult<Response> {
    match msg {
        ExecuteMsg::RotateFrostPubkey {
            new_vk_commit,
            hkdf_commit,
            next_seats,
            next_t,
            next_n,
        } => invoke(deps, info, new_vk_commit, hkdf_commit, next_seats, next_t, next_n),
        ExecuteMsg::AddRotateShare {
            new_vk,
            hkdf_commit,
            blob,
            admin_id,
        } => add_share(deps, info, new_vk, hkdf_commit, blob, admin_id),
    }
}

fn check_roster(seats: &[String], t: u16, n: u16) -> Result<(), &'static str> {
    if seats.is_empty() {
        return Err("empty_roster");
    }
    if seats.iter().any(|s| s.trim().is_empty()) {
        return Err("empty_seat");
    }
    let mut seen = std::collections::BTreeSet::new();
    for s in seats {
        if !seen.insert(s.as_str()) {
            return Err("duplicate_seat");
        }
    }
    if n as usize != seats.len() {
        return Err("n_mismatch");
    }
    if t == 0 || t > n {
        return Err("bad_threshold");
    }
    Ok(())
}

fn invoke(
    mut deps: DepsMut,
    info: MessageInfo,
    new_vk_commit: String,
    hkdf_commit: Option<String>,
    next_seats: Option<Vec<String>>,
    next_t: Option<u16>,
    next_n: Option<u16>,
) -> StdResult<Response> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender.as_str() != cfg.owner {
        append_log(deps.branch(), info.sender.as_str(), "fail", Some("not_owner"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            info.sender.as_str(),
            Some("not_owner"),
        ));
    }
    if new_vk_commit.trim().is_empty() {
        append_log(deps.branch(), cfg.owner.as_str(), "fail", Some("empty_commit"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            cfg.owner.as_str(),
            Some("empty_commit"),
        ));
    }
    if let Some(p) = PROPOSAL.may_load(deps.storage)? {
        if p.status == "pending" {
            append_log(deps.branch(), cfg.owner.as_str(), "fail", Some("already_pending"))?;
            return Ok(ok_write(
                "rotate_frost_pubkey",
                "fail",
                cfg.owner.as_str(),
                Some("already_pending"),
            ));
        }
    }
    let (next_seats, next_t, next_n) = match (next_seats, next_t, next_n) {
        (None, None, None) => (cfg.seats.clone(), cfg.t, cfg.n),
        (Some(s), Some(t), Some(n)) => match check_roster(&s, t, n) {
            Ok(()) => (s, t, n),
            Err(e) => {
                append_log(deps.branch(), cfg.owner.as_str(), "fail", Some(e))?;
                return Ok(ok_write(
                    "rotate_frost_pubkey",
                    "fail",
                    cfg.owner.as_str(),
                    Some(e),
                ));
            }
        },
        _ => {
            append_log(
                deps.branch(),
                cfg.owner.as_str(),
                "fail",
                Some("roster_partial"),
            )?;
            return Ok(ok_write(
                "rotate_frost_pubkey",
                "fail",
                cfg.owner.as_str(),
                Some("roster_partial"),
            ));
        }
    };
    PROPOSAL.save(
        deps.storage,
        &RotateProposal {
            new_vk_commit: new_vk_commit.clone(),
            hkdf_commit,
            initiator: cfg.owner.clone(),
            status: "pending".into(),
            shares: Vec::new(),
            new_vk: None,
            next_seats,
            next_t,
            next_n,
        },
    )?;
    append_log(deps.branch(), cfg.owner.as_str(), "invoke", None)?;
    Ok(ok_write(
        "rotate_frost_pubkey",
        "pending",
        cfg.owner.as_str(),
        None,
    )
    .add_attribute("new_vk_commit", new_vk_commit))
}

fn add_share(
    mut deps: DepsMut,
    info: MessageInfo,
    new_vk: String,
    hkdf_commit: Option<String>,
    blob: String,
    admin_id: Option<String>,
) -> StdResult<Response> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender.as_str() != cfg.owner {
        append_log(deps.branch(), info.sender.as_str(), "fail", Some("not_owner"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            info.sender.as_str(),
            Some("not_owner"),
        ));
    }
    let admin = admin_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(info.sender.as_str())
        .to_string();
    if !seat_of(&cfg, &admin) {
        append_log(deps.branch(), &admin, "fail", Some("not_admin"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("not_admin"),
        ));
    }
    let Some(mut p) = PROPOSAL.may_load(deps.storage)? else {
        append_log(deps.branch(), &admin, "fail", Some("no_proposal"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("no_proposal"),
        ));
    };
    if p.status != "pending" {
        append_log(deps.branch(), &admin, "fail", Some("not_pending"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("not_pending"),
        ));
    }
    if vk_commit(&new_vk) != p.new_vk_commit {
        append_log(deps.branch(), &admin, "fail", Some("commit_mismatch"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("commit_mismatch"),
        ));
    }
    if p.hkdf_commit != hkdf_commit {
        append_log(deps.branch(), &admin, "fail", Some("hkdf_mismatch"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("hkdf_mismatch"),
        ));
    }
    if blob.to_ascii_lowercase().contains("keypackage") {
        append_log(deps.branch(), &admin, "fail", Some("keypackage_rejected"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("keypackage_rejected"),
        ));
    }
    if p.shares.iter().any(|s| s == &admin) {
        append_log(deps.branch(), &admin, "fail", Some("duplicate_admin"))?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "fail",
            &admin,
            Some("duplicate_admin"),
        ));
    }
    p.shares.push(admin.clone());
    if p.new_vk.is_none() {
        p.new_vk = Some(new_vk.clone());
    }
    let share_n = p.shares.len() as u16;
    append_log(deps.branch(), &admin, "share", None)?;
    if share_n >= cfg.t {
        let old = cfg.group_vk.clone();
        cfg.old_vk = Some(old.clone());
        cfg.group_vk = p.new_vk.clone().unwrap_or(new_vk);
        cfg.seats = p.next_seats.clone();
        cfg.t = p.next_t;
        cfg.n = p.next_n;
        p.status = "applied".into();
        CONFIG.save(deps.storage, &cfg)?;
        PROPOSAL.save(deps.storage, &p)?;
        append_log(deps.branch(), &admin, "apply", None)?;
        return Ok(ok_write(
            "rotate_frost_pubkey",
            "applied",
            &admin,
            None,
        )
        .add_attribute("group_vk", cfg.group_vk)
        .add_attribute("old_vk", old)
        .add_attribute("share_count", share_n.to_string())
        .add_attribute("n", cfg.n.to_string())
        .add_attribute("t", cfg.t.to_string()));
    }
    PROPOSAL.save(deps.storage, &p)?;
    Ok(ok_write("rotate_frost_pubkey", "pending", &admin, None)
        .add_attribute("share_count", share_n.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GroupVk {} => {
            let cfg = CONFIG.load(deps.storage)?;
            to_json_binary(&GroupVkResponse {
                group_vk: cfg.group_vk,
                old_vk: cfg.old_vk,
            })
        }
        QueryMsg::Pending {} => to_json_binary(&pending_of(deps)?),
        QueryMsg::Status {} => {
            let cfg = CONFIG.load(deps.storage)?;
            let p = pending_of(deps)?;
            let log = LOG.may_load(deps.storage)?.unwrap_or_default();
            let last_reason = log.iter().rev().find_map(|e| e.reason.clone());
            to_json_binary(&StatusResponse {
                group_vk: cfg.group_vk,
                old_vk: cfg.old_vk,
                t: cfg.t,
                n: cfg.n,
                pending: p.pending,
                new_vk_commit: p.new_vk_commit,
                share_count: p.share_count,
                shares: p.shares,
                seats: cfg.seats,
                last_reason,
            })
        }
    }
}

fn pending_of(deps: Deps) -> StdResult<PendingResponse> {
    match PROPOSAL.may_load(deps.storage)? {
        Some(p) => Ok(PendingResponse {
            pending: p.status == "pending",
            new_vk_commit: Some(p.new_vk_commit),
            share_count: p.shares.len() as u32,
            shares: p.shares,
            status: p.status,
        }),
        None => Ok(PendingResponse {
            pending: false,
            new_vk_commit: None,
            share_count: 0,
            shares: vec![],
            status: "none".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{from_json, Coin};

    type TestDeps = cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >;

    fn setup() -> (TestDeps, cosmwasm_std::Addr) {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let info = message_info(&owner, &[] as &[Coin]);
        instantiate(
            deps.as_mut(),
            mock_env(),
            info,
            InstantiateMsg {
                group_vk: "vk-old".into(),
                t: 2,
                n: 3,
                seats: vec!["desk".into(), "steward".into(), "spare".into()],
            },
        )
        .unwrap();
        (deps, owner)
    }

    fn status(deps: &TestDeps) -> StatusResponse {
        from_json(query(deps.as_ref(), mock_env(), QueryMsg::Status {}).unwrap()).unwrap()
    }

    #[test]
    fn pending_until_t_then_group_vk_changes() {
        let (mut deps, owner) = setup();
        let info = message_info(&owner, &[] as &[Coin]);
        let commit = vk_commit("vk-new");
        let r = execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: commit,
                hkdf_commit: Some("hkdf-lab".into()),
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        assert!(r.attributes.iter().any(|a| a.value == "pending"));
        let s = status(&deps);
        assert!(s.pending);
        assert_eq!(s.group_vk, "vk-old");
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: Some("hkdf-lab".into()),
                blob: "S:0:a".into(),
                admin_id: Some("desk".into()),
            },
        )
        .unwrap();
        let s = status(&deps);
        assert!(s.pending);
        assert_eq!(s.group_vk, "vk-old");
        assert_eq!(s.share_count, 1);
        execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: Some("hkdf-lab".into()),
                blob: "S:0:b".into(),
                admin_id: Some("steward".into()),
            },
        )
        .unwrap();
        let s = status(&deps);
        assert!(!s.pending);
        assert_eq!(s.group_vk, "vk-new");
        assert_eq!(s.old_vk.as_deref(), Some("vk-old"));
        assert_eq!(s.share_count, 2);
    }

    #[test]
    fn commit_mismatch_leaves_old_vk() {
        let (mut deps, owner) = setup();
        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-new"),
                hkdf_commit: None,
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        let r = execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-OTHER".into(),
                hkdf_commit: None,
                blob: "S:0:x".into(),
                admin_id: Some("desk".into()),
            },
        )
        .unwrap();
        assert!(r.attributes.iter().any(|a| a.value == "commit_mismatch"));
        let s = status(&deps);
        assert_eq!(s.group_vk, "vk-old");
        assert!(s.pending);
    }

    #[test]
    fn second_invoke_while_pending_fail_written() {
        let (mut deps, owner) = setup();
        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-new"),
                hkdf_commit: None,
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        let r = execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-other"),
                hkdf_commit: None,
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        assert!(r.attributes.iter().any(|a| a.value == "already_pending"));
        let s = status(&deps);
        assert_eq!(s.group_vk, "vk-old");
        assert!(s.pending);
        assert_eq!(s.last_reason.as_deref(), Some("already_pending"));
    }

    #[test]
    fn add_member_at_apply_changes_n_and_seats() {
        let (mut deps, owner) = setup();
        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-new"),
                hkdf_commit: None,
                next_seats: Some(vec![
                    "desk".into(),
                    "steward".into(),
                    "spare".into(),
                    "usher".into(),
                ]),
                next_t: Some(2),
                next_n: Some(4),
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:a".into(),
                admin_id: Some("desk".into()),
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:b".into(),
                admin_id: Some("steward".into()),
            },
        )
        .unwrap();
        let s = status(&deps);
        assert_eq!(s.group_vk, "vk-new");
        assert_eq!(s.n, 4);
        assert_eq!(s.t, 2);
        assert!(s.seats.iter().any(|x| x == "usher"));
        assert_eq!(s.seats.len(), 4);
    }

    #[test]
    fn remove_member_at_apply_drops_seat() {
        let (mut deps, owner) = setup();
        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-new"),
                hkdf_commit: None,
                next_seats: Some(vec!["desk".into(), "steward".into()]),
                next_t: Some(2),
                next_n: Some(2),
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:a".into(),
                admin_id: Some("desk".into()),
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:b".into(),
                admin_id: Some("steward".into()),
            },
        )
        .unwrap();
        let s = status(&deps);
        assert_eq!(s.n, 2);
        assert!(!s.seats.iter().any(|x| x == "spare"));
        // removed seat cannot share on a new rotate
        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-newer"),
                hkdf_commit: None,
                next_seats: None,
                next_t: None,
                next_n: None,
            },
        )
        .unwrap();
        let r = execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-newer".into(),
                hkdf_commit: None,
                blob: "S:0:c".into(),
                admin_id: Some("spare".into()),
            },
        )
        .unwrap();
        assert!(r.attributes.iter().any(|a| a.value == "not_admin"));
        assert_eq!(status(&deps).group_vk, "vk-new");
    }

    #[test]
    fn incoming_player_cannot_share_before_apply() {
        let (mut deps, owner) = setup();
        let info = message_info(&owner, &[] as &[Coin]);
        execute(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            ExecuteMsg::RotateFrostPubkey {
                new_vk_commit: vk_commit("vk-new"),
                hkdf_commit: None,
                next_seats: Some(vec![
                    "desk".into(),
                    "steward".into(),
                    "spare".into(),
                    "usher".into(),
                ]),
                next_t: Some(2),
                next_n: Some(4),
            },
        )
        .unwrap();
        let r = execute(
            deps.as_mut(),
            mock_env(),
            info,
            ExecuteMsg::AddRotateShare {
                new_vk: "vk-new".into(),
                hkdf_commit: None,
                blob: "S:0:z".into(),
                admin_id: Some("usher".into()),
            },
        )
        .unwrap();
        assert!(r.attributes.iter().any(|a| a.value == "not_admin"));
        assert_eq!(status(&deps).group_vk, "vk-old");
        assert!(status(&deps).pending);
    }
}
