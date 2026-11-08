//! wasm32-unknown-unknown ABI. No WASI. RNG from host import `env.host_random`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use frost_ed25519 as frost;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

use crate::scheme::{CommitmentMap, ConsensusScheme};
use crate::FrostEd25519;

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_random(ptr: *mut u8, len: u32);
}

fn fill_random(buf: &mut [u8]) {
    unsafe { host_random(buf.as_mut_ptr(), buf.len() as u32) }
}

getrandom::register_custom_getrandom!(guest_getrandom);
fn guest_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    fill_random(buf);
    Ok(())
}

enum Slot {
    Share(frost::keys::KeyPackage),
    Nonces(frost::round1::SigningNonces),
    Group(frost::keys::PublicKeyPackage),
    DkgR1(frost::keys::dkg::round1::SecretPackage),
    DkgR2(frost::keys::dkg::round2::SecretPackage),
}

struct Store {
    next: u32,
    slots: BTreeMap<u32, Slot>,
}

impl Store {
    fn put(&mut self, s: Slot) -> u32 {
        let id = self.next;
        self.next += 1;
        self.slots.insert(id, s);
        id
    }
}

static STORE: Mutex<Store> = Mutex::new(Store {
    next: 1,
    slots: BTreeMap::new(),
});

#[derive(Deserialize)]
struct Req {
    op: String,
    #[serde(default)]
    id: u16,
    #[serde(default)]
    n: u16,
    #[serde(default)]
    t: u16,
    #[serde(default)]
    slot: u32,
    #[serde(default)]
    share_slot: u32,
    #[serde(default)]
    nonce_slot: u32,
    #[serde(default)]
    bytes_b64: String,
    #[serde(default)]
    msg_b64: String,
    #[serde(default)]
    cmap_b64: BTreeMap<String, String>,
    #[serde(default)]
    partials_b64: BTreeMap<String, String>,
    #[serde(default)]
    group_slot: u32,
    #[serde(default)]
    r1_b64: BTreeMap<String, String>,
    #[serde(default)]
    r2_b64: BTreeMap<String, String>,
}

#[derive(Serialize, Default)]
struct Resp {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slot: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce_slot: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    share_slot: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_slot: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pkg_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pkgs_b64: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    partial_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sig_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pk_hex: Option<String>,
}

fn b64e(b: &[u8]) -> String {
    B64.encode(b)
}
fn b64d(s: &str) -> Result<Vec<u8>, String> {
    B64.decode(s).map_err(|e| e.to_string())
}

fn handle(req: Req) -> Resp {
    let r = match req.op.as_str() {
        "slot_load_share" => op_load_share(&req),
        "slot_load_group" => op_load_group(&req),
        "slot_zeroize" => op_zeroize(&req),
        "dkg_r1" => op_dkg_r1(&req),
        "dkg_r2" => op_dkg_r2(&req),
        "dkg_finish" => op_dkg_finish(&req),
        "frost_commit" => op_commit(&req),
        "frost_sign" => op_sign(&req),
        "frost_combine" => op_combine(&req),
        "group_pk" => op_group_pk(&req),
        _ => Err(format!("unknown op {}", req.op)),
    };
    match r {
        Ok(x) => x,
        Err(e) => Resp {
            ok: false,
            error: Some(e),
            ..Default::default()
        },
    }
}

fn op_load_share(req: &Req) -> Result<Resp, String> {
    let raw = b64d(&req.bytes_b64)?;
    let share = frost::keys::KeyPackage::deserialize(&raw).map_err(|e| e.to_string())?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let slot = st.put(Slot::Share(share));
    Ok(Resp {
        ok: true,
        share_slot: Some(slot),
        slot: Some(slot),
        ..Default::default()
    })
}

fn op_zeroize(req: &Req) -> Result<Resp, String> {
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    st.slots.remove(&req.slot);
    Ok(Resp {
        ok: true,
        ..Default::default()
    })
}

fn op_dkg_r1(req: &Req) -> Result<Resp, String> {
    let mut rng = OsRng;
    let (sec, pkg) =
        FrostEd25519::dkg_r1(req.id, req.n, req.t, &mut rng).map_err(|e| e.to_string())?;
    let bytes = pkg.serialize().map_err(|e| e.to_string())?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let slot = st.put(Slot::DkgR1(sec));
    Ok(Resp {
        ok: true,
        slot: Some(slot),
        pkg_b64: Some(b64e(&bytes)),
        ..Default::default()
    })
}

fn map_r1(
    m: &BTreeMap<String, String>,
) -> Result<BTreeMap<u16, frost::keys::dkg::round1::Package>, String> {
    let mut out = BTreeMap::new();
    for (k, v) in m {
        let id: u16 = k
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        let raw = b64d(v)?;
        out.insert(
            id,
            frost::keys::dkg::round1::Package::deserialize(&raw).map_err(|e| e.to_string())?,
        );
    }
    Ok(out)
}

fn map_r2(
    m: &BTreeMap<String, String>,
) -> Result<BTreeMap<u16, frost::keys::dkg::round2::Package>, String> {
    let mut out = BTreeMap::new();
    for (k, v) in m {
        let id: u16 = k
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        let raw = b64d(v)?;
        out.insert(
            id,
            frost::keys::dkg::round2::Package::deserialize(&raw).map_err(|e| e.to_string())?,
        );
    }
    Ok(out)
}

fn op_dkg_r2(req: &Req) -> Result<Resp, String> {
    let r1 = map_r1(&req.r1_b64)?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let sec = match st.slots.remove(&req.slot) {
        Some(Slot::DkgR1(s)) => s,
        _ => return Err("dkg_r2: missing r1 slot".into()),
    };
    drop(st);
    let (sec2, by_peer) = FrostEd25519::dkg_r2(sec, &r1).map_err(|e| e.to_string())?;
    let mut pkgs = BTreeMap::new();
    for (id, pkg) in &by_peer {
        pkgs.insert(
            id.to_string(),
            b64e(&pkg.serialize().map_err(|e| e.to_string())?),
        );
    }
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let slot = st.put(Slot::DkgR2(sec2));
    Ok(Resp {
        ok: true,
        slot: Some(slot),
        pkgs_b64: Some(pkgs),
        ..Default::default()
    })
}

fn op_dkg_finish(req: &Req) -> Result<Resp, String> {
    let r1 = map_r1(&req.r1_b64)?;
    let r2 = map_r2(&req.r2_b64)?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let sec = match st.slots.remove(&req.slot) {
        Some(Slot::DkgR2(s)) => s,
        _ => return Err("dkg_finish: missing r2 slot".into()),
    };
    drop(st);
    let (share, group) = FrostEd25519::dkg_finish(sec, &r1, &r2).map_err(|e| e.to_string())?;
    let pk = FrostEd25519::group_pk_bytes(&group).map_err(|e| e.to_string())?;
    let share_bytes = share.serialize().map_err(|e| e.to_string())?;
    let group_bytes = group.serialize().map_err(|e| e.to_string())?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let share_slot = st.put(Slot::Share(share));
    let group_slot = st.put(Slot::Group(group));
    Ok(Resp {
        ok: true,
        share_slot: Some(share_slot),
        group_slot: Some(group_slot),
        pk_hex: Some(hex(&pk)),
        pkg_b64: Some(b64e(&share_bytes)),
        sig_b64: Some(b64e(&group_bytes)),
        ..Default::default()
    })
}

fn op_commit(req: &Req) -> Result<Resp, String> {
    let mut rng = OsRng;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let share = match st.slots.get(&req.share_slot) {
        Some(Slot::Share(s)) => s.clone(),
        _ => return Err("frost_commit: missing share".into()),
    };
    drop(st);
    let (nonces, commit) = FrostEd25519::commit(&share, &mut rng).map_err(|e| e.to_string())?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let nonce_slot = st.put(Slot::Nonces(nonces));
    Ok(Resp {
        ok: true,
        nonce_slot: Some(nonce_slot),
        commit_b64: Some(b64e(&commit.serialize().map_err(|e| e.to_string())?)),
        ..Default::default()
    })
}

fn parse_cmap(m: &BTreeMap<String, String>) -> Result<CommitmentMap, String> {
    let mut cmap = CommitmentMap::new();
    for (k, v) in m {
        let id: u16 = k
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        let ident = crate::scheme::identifier(id).map_err(|e| e.to_string())?;
        let raw = b64d(v)?;
        let c = frost::round1::SigningCommitments::deserialize(&raw).map_err(|e| e.to_string())?;
        cmap.insert(ident, c);
    }
    Ok(cmap)
}

fn op_sign(req: &Req) -> Result<Resp, String> {
    let cmap = parse_cmap(&req.cmap_b64)?;
    let msg = b64d(&req.msg_b64)?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let share = match st.slots.get(&req.share_slot) {
        Some(Slot::Share(s)) => s.clone(),
        _ => return Err("frost_sign: missing share".into()),
    };
    let nonces = match st.slots.remove(&req.nonce_slot) {
        Some(Slot::Nonces(n)) => n,
        _ => return Err("frost_sign: missing/consumed nonces".into()),
    };
    drop(st);
    let p = FrostEd25519::sign_share(&share, nonces, &cmap, &msg).map_err(|e| e.to_string())?;
    Ok(Resp {
        ok: true,
        partial_b64: Some(b64e(&p.serialize())),
        ..Default::default()
    })
}

fn op_combine(req: &Req) -> Result<Resp, String> {
    let cmap = parse_cmap(&req.cmap_b64)?;
    let msg = b64d(&req.msg_b64)?;
    let mut partials = BTreeMap::new();
    for (k, v) in &req.partials_b64 {
        let id: u16 = k
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        let raw = b64d(v)?;
        partials.insert(
            id,
            frost::round2::SignatureShare::deserialize(&raw).map_err(|e| e.to_string())?,
        );
    }
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let group = match st.slots.get(&req.group_slot) {
        Some(Slot::Group(g)) => g.clone(),
        _ => return Err("frost_combine: missing group".into()),
    };
    drop(st);
    let sig = FrostEd25519::combine(&cmap, &partials, &msg, &group).map_err(|e| e.to_string())?;
    let bytes = FrostEd25519::signature_bytes(&sig).map_err(|e| e.to_string())?;
    Ok(Resp {
        ok: true,
        sig_b64: Some(b64e(&bytes)),
        ..Default::default()
    })
}

fn op_load_group(req: &Req) -> Result<Resp, String> {
    let raw = b64d(&req.bytes_b64)?;
    let group = frost::keys::PublicKeyPackage::deserialize(&raw).map_err(|e| e.to_string())?;
    let pk = FrostEd25519::group_pk_bytes(&group).map_err(|e| e.to_string())?;
    let mut st = STORE.lock().map_err(|e| e.to_string())?;
    let slot = st.put(Slot::Group(group));
    Ok(Resp {
        ok: true,
        group_slot: Some(slot),
        slot: Some(slot),
        pk_hex: Some(hex(&pk)),
        ..Default::default()
    })
}

fn op_group_pk(req: &Req) -> Result<Resp, String> {
    let st = STORE.lock().map_err(|e| e.to_string())?;
    let pk = match st.slots.get(&req.group_slot) {
        Some(Slot::Group(g)) => FrostEd25519::group_pk_bytes(g).map_err(|e| e.to_string())?,
        Some(Slot::Share(s)) => {
            let v: Vec<u8> = s.verifying_key().serialize().map_err(|e| e.to_string())?;
            v.try_into().map_err(|_| "pk len".to_string())?
        }
        _ => return Err("group_pk: missing slot".into()),
    };
    drop(st);
    Ok(Resp {
        ok: true,
        pk_hex: Some(hex(&pk)),
        ..Default::default()
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[no_mangle]
pub extern "C" fn kms_alloc(n: u32) -> u32 {
    let layout = std::alloc::Layout::from_size_align(n.max(1) as usize, 8).expect("layout");
    let p = unsafe { std::alloc::alloc(layout) };
    p as usize as u32
}

#[no_mangle]
pub extern "C" fn kms_free(ptr: u32, n: u32) {
    if ptr == 0 {
        return;
    }
    let layout = std::alloc::Layout::from_size_align(n.max(1) as usize, 8).expect("layout");
    unsafe { std::alloc::dealloc(ptr as usize as *mut u8, layout) }
}

/// JSON in, JSON out. out_ptr is written as u32 pointer; return is byte length, or -1.
#[no_mangle]
pub extern "C" fn kms_call(in_ptr: u32, in_len: u32, out_ptr_ptr: u32) -> i32 {
    let input = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, in_len as usize) };
    let req: Req = match serde_json::from_slice(input) {
        Ok(r) => r,
        Err(e) => {
            return write_err(&e.to_string(), out_ptr_ptr);
        }
    };
    let resp = handle(req);
    let bytes = serde_json::to_vec(&resp).unwrap_or_else(|e| {
        serde_json::to_vec(&Resp {
            ok: false,
            error: Some(e.to_string()),
            ..Default::default()
        })
        .unwrap()
    });
    write_bytes(&bytes, out_ptr_ptr)
}

fn write_err(msg: &str, out_ptr_ptr: u32) -> i32 {
    let resp = Resp {
        ok: false,
        error: Some(msg.into()),
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&resp).unwrap();
    write_bytes(&bytes, out_ptr_ptr)
}

fn write_bytes(bytes: &[u8], out_ptr_ptr: u32) -> i32 {
    let n = bytes.len() as u32;
    let ptr = kms_alloc(n.max(1));
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as usize as *mut u8, bytes.len());
        *(out_ptr_ptr as usize as *mut u32) = ptr;
    }
    n as i32
}
