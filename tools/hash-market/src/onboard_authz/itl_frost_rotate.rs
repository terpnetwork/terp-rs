//! Admin FROST **group key** rotation — in-process SoT.
//!
//! One admin invokes `rotate_frost_pubkey`; the call is **always written**
//! (pending or fail event). Shares accumulate until t-of-n bound to the same
//! `new_vk_commit` (and optional HKDF/DKG commitment). Then the group verifying
//! key **changes**. Old key stays until apply — never empty.
//!
//! On-chain analog: MsgExecuteContract rotate_frost_pubkey on terp-frost
//! (never Authenticate sudo — those writes discard). Spends still
//! use frost-spend with the current group_vk. Rotate is t-of-n of **admin seats**
//! (passkey authenticators), not a chicken-egg FROST-sign of the rotate itself.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::itl_frost::{FROST_N, FROST_T};

/// Next epoch roster. Dealers (current `seats`) ack the dealing; players
/// (`NextRoster::seats`) hold shares after apply. Commonware-reshare insight:
/// membership change is an epoch boundary + new dealing, never a same-vk refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextRoster {
    pub seats: Vec<String>,
    pub t: u16,
    pub n: u16,
}

impl NextRoster {
    pub fn from_seats(seats: &[&str], t: u16, n: u16) -> Result<Self, &'static str> {
        let seats: Vec<String> = seats.iter().map(|s| (*s).to_string()).collect();
        Self::checked(seats, t, n)
    }

    pub fn checked(seats: Vec<String>, t: u16, n: u16) -> Result<Self, &'static str> {
        if seats.is_empty() {
            return Err("empty_roster");
        }
        if seats.iter().any(|s| s.trim().is_empty()) {
            return Err("empty_seat");
        }
        let uniq: BTreeSet<&str> = seats.iter().map(|s| s.as_str()).collect();
        if uniq.len() != seats.len() {
            return Err("duplicate_seat");
        }
        if n as usize != seats.len() {
            return Err("n_mismatch");
        }
        if t == 0 || t > n {
            return Err("bad_threshold");
        }
        Ok(Self { seats, t, n })
    }
}

pub const HKDF_INFO: &[u8] = b"itl-frost-rotate-v1";
pub const ROTATE_MSG: &str = "rotate_frost_pubkey";
pub const EXECUTE_CONTRACT_TYPE_URL: &str = "/cosmwasm.wasm.v1.MsgExecuteContract";

/// Persistable status. Restore must fail if KeyPackages appear.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommitteeSnapshot {
    pub group_vk: String,
    pub old_vk: Option<String>,
    pub rekey_txid: Option<String>,
    pub log_seq: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotateStatus {
    Pending,
    Applied,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotateKind {
    Invoke,
    Share,
    Apply,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateEvent {
    pub seq: u32,
    pub admin: String,
    pub kind: RotateKind,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RotateProposal {
    pub id: String,
    /// sha256(new_group_vk) hex — all shares must bind this
    pub new_vk_commit: String,
    /// Optional HKDF/DKG commitment. If set on invoke, every share must match.
    pub hkdf_commit: Option<String>,
    pub initiator: String,
    pub status: RotateStatus,
    pub shares: BTreeMap<String, String>,
    pub new_vk: Option<String>,
    /// Players for the next epoch (installed at apply).
    pub next_seats: Vec<String>,
    pub next_t: u16,
    pub next_n: u16,
}

#[derive(Debug, Clone)]
pub struct FrostCommittee {
    pub group_vk: String,
    pub seats: Vec<String>,
    pub t: u16,
    pub n: u16,
    pub rotate: Option<RotateProposal>,
    pub log: Vec<RotateEvent>,
    /// Previous group vk until shielded rekey spend is recorded.
    pub old_vk: Option<String>,
    pub rekey_txid: Option<String>,
    /// Applied dealings. Incremented at t-of-n apply (epoch boundary).
    pub epoch: u32,
    seq: u32,
    /// Share blobs consumed in an applied epoch — cannot be reused next epoch.
    spent_blobs: BTreeSet<String>,
}

impl FrostCommittee {
    pub fn new(group_vk: &str, seats: &[&str]) -> Self {
        Self {
            group_vk: group_vk.into(),
            seats: seats.iter().map(|s| (*s).to_string()).collect(),
            t: FROST_T,
            n: FROST_N,
            rotate: None,
            log: vec![],
            old_vk: None,
            rekey_txid: None,
            epoch: 0,
            seq: 0,
            spent_blobs: BTreeSet::new(),
        }
    }

    fn write(&mut self, admin: &str, kind: RotateKind, reason: Option<&str>) {
        self.seq += 1;
        self.log.push(RotateEvent {
            seq: self.seq,
            admin: admin.into(),
            kind,
            reason: reason.map(|s| s.into()),
        });
    }

    pub fn snapshot(&self) -> CommitteeSnapshot {
        CommitteeSnapshot {
            group_vk: self.group_vk.clone(),
            old_vk: self.old_vk.clone(),
            rekey_txid: self.rekey_txid.clone(),
            log_seq: self.seq,
        }
    }

    pub fn restore_snapshot(&mut self, snap: CommitteeSnapshot) -> Result<(), &'static str> {
        let raw = serde_json::to_string(&snap).unwrap_or_default();
        if super::itl_frost::session_json_ok(&raw) == false
            || raw.to_ascii_lowercase().contains("keypackage")
            || raw.contains("packages")
        {
            self.write("system", RotateKind::Fail, Some("snapshot_dirty"));
            return Err("snapshot_dirty");
        }
        self.group_vk = snap.group_vk;
        self.old_vk = snap.old_vk;
        self.rekey_txid = snap.rekey_txid;
        Ok(())
    }

    pub fn is_seat(&self, admin: &str) -> bool {
        self.seats.iter().any(|s| s == admin)
    }

    /// Live holder-mempool commit (current epoch seats + vk). Pending next_seats are not live.
    pub fn roster_commit(&self) -> String {
        super::itl_frost::roster_commit(
            self.epoch,
            self.t,
            self.n,
            &self.group_vk,
            &self.seats,
        )
    }

    /// Hash of the *proposed* next roster — must not authorize spends until apply.
    pub fn pending_roster_commit(&self) -> Option<String> {
        let p = self.rotate.as_ref()?;
        if p.status != RotateStatus::Pending {
            return None;
        }
        Some(super::itl_frost::roster_commit(
            self.epoch.saturating_add(1),
            p.next_t,
            p.next_n,
            p.new_vk.as_deref().unwrap_or(""),
            &p.next_seats,
        ))
    }

    /// Both FROST domains: SpendAuth covers the tx; memo must bind this roster.
    /// HashMerchant cannot move funds without holder consensus on `roster_commit`.
    pub fn authorize_spend(
        &self,
        job: &super::itl_frost::PendingJob,
        memo: &str,
    ) -> Result<(), &'static str> {
        self.can_spend_new_vk()?;
        if job.roster_commit.is_empty() {
            return Err("roster_commit_required");
        }
        if job.roster_commit != self.roster_commit() {
            return Err("roster_stale");
        }
        for id in job.admin_ids() {
            if !self.is_seat(id) {
                return Err("not_holder");
            }
        }
        super::itl_frost::require_spend_memo_roster(memo, &self.roster_commit())?;
        Ok(())
    }

    /// Same roster, new vk. Membership-preserving rotate.
    pub fn invoke_rotate(
        &mut self,
        admin: &str,
        new_vk: &str,
        hkdf_commit: Option<&str>,
    ) -> Result<(), &'static str> {
        let roster = NextRoster {
            seats: self.seats.clone(),
            t: self.t,
            n: self.n,
        };
        self.invoke_rotate_roster(admin, new_vk, hkdf_commit, roster)
    }

    /// Epoch rotate: new group vk **and** next player set (add/remove members).
    /// Dealers = current seats (must ack). Players = `roster` (installed at apply).
    /// Always writes. Same-vk refresh is forbidden — that would keep old shares live.
    pub fn invoke_rotate_roster(
        &mut self,
        admin: &str,
        new_vk: &str,
        hkdf_commit: Option<&str>,
        roster: NextRoster,
    ) -> Result<(), &'static str> {
        if !self.is_seat(admin) {
            self.write(admin, RotateKind::Fail, Some("not_admin"));
            return Err("not_admin");
        }
        if new_vk.trim().is_empty() {
            self.write(admin, RotateKind::Fail, Some("empty_vk"));
            return Err("empty_vk");
        }
        if new_vk == self.group_vk {
            self.write(admin, RotateKind::Fail, Some("same_vk"));
            return Err("same_vk");
        }
        if let Err(e) = NextRoster::checked(roster.seats.clone(), roster.t, roster.n) {
            self.write(admin, RotateKind::Fail, Some(e));
            return Err(e);
        }
        if let Some(p) = &self.rotate {
            if p.status == RotateStatus::Pending {
                self.write(admin, RotateKind::Fail, Some("already_pending"));
                return Err("already_pending");
            }
        }
        let commit = vk_commit(new_vk);
        self.rotate = Some(RotateProposal {
            id: format!("rot-{}", self.seq + 1),
            new_vk_commit: commit,
            hkdf_commit: hkdf_commit.map(|s| s.to_string()),
            initiator: admin.into(),
            status: RotateStatus::Pending,
            shares: BTreeMap::new(),
            new_vk: Some(new_vk.into()),
            next_seats: roster.seats,
            next_t: roster.t,
            next_n: roster.n,
        });
        self.write(admin, RotateKind::Invoke, None);
        Ok(())
    }

    /// Always writes. Commit mismatch fails closed; group_vk unchanged.
    pub fn add_rotate_share(
        &mut self,
        admin: &str,
        new_vk: &str,
        hkdf_commit: Option<&str>,
        blob: &str,
    ) -> Result<RotateStatus, &'static str> {
        if !self.is_seat(admin) {
            self.write(admin, RotateKind::Fail, Some("not_admin"));
            return Err("not_admin");
        }
        let Some(p) = self.rotate.as_mut() else {
            self.write(admin, RotateKind::Fail, Some("no_proposal"));
            return Err("no_proposal");
        };
        if p.status != RotateStatus::Pending {
            self.write(admin, RotateKind::Fail, Some("not_pending"));
            return Err("not_pending");
        }
        if vk_commit(new_vk) != p.new_vk_commit {
            self.write(admin, RotateKind::Fail, Some("commit_mismatch"));
            return Err("commit_mismatch");
        }
        if p.hkdf_commit.as_deref() != hkdf_commit {
            self.write(admin, RotateKind::Fail, Some("hkdf_mismatch"));
            return Err("hkdf_mismatch");
        }
        if blob.to_ascii_lowercase().contains("keypackage") {
            self.write(admin, RotateKind::Fail, Some("keypackage_rejected"));
            return Err("keypackage_rejected");
        }
        if p.shares.contains_key(admin) {
            self.write(admin, RotateKind::Fail, Some("duplicate_admin"));
            return Err("duplicate_admin");
        }
        if self.spent_blobs.contains(blob) {
            self.write(admin, RotateKind::Fail, Some("share_replay_epoch"));
            return Err("share_replay_epoch");
        }
        p.shares.insert(admin.into(), blob.into());
        let share_n = p.shares.len() as u16;
        let vk = p.new_vk.clone().unwrap_or_default();
        let next_seats = p.next_seats.clone();
        let next_t = p.next_t;
        let next_n = p.next_n;
        let blobs: Vec<String> = p.shares.values().cloned().collect();
        drop(p);
        self.write(admin, RotateKind::Share, None);
        if share_n >= self.t {
            let old = self.group_vk.clone();
            if let Some(p) = self.rotate.as_mut() {
                p.status = RotateStatus::Applied;
            }
            self.old_vk = Some(old.clone());
            self.rekey_txid = None;
            self.group_vk = vk;
            self.seats = next_seats;
            self.t = next_t;
            self.n = next_n;
            self.epoch = self.epoch.saturating_add(1);
            for b in blobs {
                self.spent_blobs.insert(b);
            }
            debug_assert!(!self.group_vk.is_empty());
            debug_assert_ne!(self.group_vk, old);
            self.write(admin, RotateKind::Apply, None);
            return Ok(RotateStatus::Applied);
        }
        Ok(RotateStatus::Pending)
    }

    /// Pool notes still sit at old orchard UA until this spend.
    pub fn can_spend_new_vk(&self) -> Result<(), &'static str> {
        if self.old_vk.is_some() && self.rekey_txid.is_none() {
            return Err("rekey_incomplete");
        }
        Ok(())
    }

    /// Shielded transfer old UA → new UA, authorized by the *old* committee.
    pub fn record_rekey_spend(
        &mut self,
        txid: &str,
        from_vk: &str,
        to_vk: &str,
    ) -> Result<(), &'static str> {
        let Some(old) = self.old_vk.as_deref() else {
            self.write("system", RotateKind::Fail, Some("no_old_vk"));
            return Err("no_old_vk");
        };
        if from_vk != old {
            self.write("system", RotateKind::Fail, Some("from_vk_mismatch"));
            return Err("from_vk_mismatch");
        }
        if to_vk != self.group_vk {
            self.write("system", RotateKind::Fail, Some("to_vk_mismatch"));
            return Err("to_vk_mismatch");
        }
        if txid.trim().is_empty() {
            self.write("system", RotateKind::Fail, Some("no_txid"));
            return Err("no_txid");
        }
        self.rekey_txid = Some(txid.into());
        self.old_vk = None;
        self.write("system", RotateKind::Apply, Some("rekey_spend"));
        Ok(())
    }

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

/// Successful Execute on a **rotate contract**. Tx selects a passkey/AnyOf
/// authenticator — never CosmwasmAuthenticatorV1 sudo (those writes discard).
pub fn msg_rotate_frost_pubkey(
    sender: &str,
    contract: &str,
    new_vk_commit: &str,
    hkdf_commit: Option<&str>,
) -> Value {
    json!({
        "@type": EXECUTE_CONTRACT_TYPE_URL,
        "sender": sender,
        "contract": contract,
        "msg": {
            ROTATE_MSG: {
                "new_vk_commit": new_vk_commit,
                "hkdf_commit": hkdf_commit,
            }
        },
    })
}

/// Same Execute, with next-epoch roster (add/remove). No authenticator_type.
pub fn msg_rotate_frost_pubkey_roster(
    sender: &str,
    contract: &str,
    new_vk_commit: &str,
    hkdf_commit: Option<&str>,
    roster: &NextRoster,
) -> Value {
    let mut m = msg_rotate_frost_pubkey(sender, contract, new_vk_commit, hkdf_commit);
    m["msg"][ROTATE_MSG]["next_seats"] = json!(roster.seats);
    m["msg"][ROTATE_MSG]["next_t"] = json!(roster.t);
    m["msg"][ROTATE_MSG]["next_n"] = json!(roster.n);
    m
}

/// HashMerchant inbox: which FROST curve/scheme. Analog to zk-wasmvm circuit+prover pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrostScheme {
    /// Artist 20/80 ZEC (frost-spend RedPallas / Orchard).
    RedPallas,
    /// Validator FilePV only — not artist pay.
    Ed25519Consensus,
    /// Future aggregatable.
    Bls,
}

/// Where HM broadcasts after t-of-n. Same inbox, different adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastTarget {
    Zcash,
    Penumbra,
    SmartAccountExecute,
}
