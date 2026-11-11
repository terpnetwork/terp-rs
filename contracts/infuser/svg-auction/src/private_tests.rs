//! Private auction tests with real FROST-Ed25519 DKGs (bidder shares 1-3, platform 4-5,
//! seller 6). Settlement is shielded ZEC, so no test sees funds or amounts on Terp.
//! Each property in SPEC.md has a test here.
use super::*;
use crate::private::{
    DigestInput, commitment_preimage, memo_preimage, notes_preimage, ovk_tag_preimage,
    payment_leaf, signing_preimage,
};
use cosmwasm_std::testing::{
    MockApi, MockQuerier, MockStorage, message_info, mock_dependencies, mock_env,
};
use cosmwasm_std::{Api, ContractResult, OwnedDeps, SystemResult, WasmQuery, coin};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use frost_ed25519 as frost;
use rand08::rngs::OsRng;
use std::collections::BTreeMap;

#[cfg(not(target_arch = "wasm32"))]
mod vm_gas;

type D = OwnedDeps<MockStorage, MockApi, MockQuerier>;
const T0: u64 = 1_000_000;
const DUR: u64 = 3600;
const GRACE: u64 = 600;
const B: &[u16] = &[1, 2, 3];
const P: &[u16] = &[4, 5];
const S: &[u16] = &[6];

struct Group {
    keys: BTreeMap<frost::Identifier, frost::keys::KeyPackage>,
    pubkey: frost::keys::PublicKeyPackage,
}
impl Group {
    /// Full three-round DKG among the six share holders.
    fn dkg(min: u16) -> Group {
        let rng = OsRng;
        let ids: Vec<frost::Identifier> = (1..=6u16).map(|i| i.try_into().unwrap()).collect();
        let mut r1s = BTreeMap::new();
        let mut r1p = BTreeMap::new();
        for id in &ids {
            let (s, p) = frost::keys::dkg::part1(*id, 6, min, rng).unwrap();
            r1s.insert(*id, s);
            r1p.insert(*id, p);
        }
        let mut r2s = BTreeMap::new();
        let mut r2p: BTreeMap<frost::Identifier, BTreeMap<_, _>> = BTreeMap::new();
        for id in &ids {
            let others: BTreeMap<_, _> = r1p
                .iter()
                .filter(|(k, _)| *k != id)
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            let (s, out) = frost::keys::dkg::part2(r1s.remove(id).unwrap(), &others).unwrap();
            r2s.insert(*id, s);
            for (to, pkg) in out {
                r2p.entry(to).or_default().insert(*id, pkg);
            }
        }
        let mut keys = BTreeMap::new();
        let mut pubkey = None;
        for id in &ids {
            let others: BTreeMap<_, _> = r1p
                .iter()
                .filter(|(k, _)| *k != id)
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            let (kp, pk) = frost::keys::dkg::part3(&r2s[id], &others, &r2p[id]).unwrap();
            keys.insert(*id, kp);
            pubkey = Some(pk);
        }
        Group {
            keys,
            pubkey: pubkey.unwrap(),
        }
    }
    fn vk(&self) -> HexBinary {
        HexBinary::from(self.pubkey.verifying_key().serialize().unwrap())
    }
    /// FROST sign with the given share holders. Errors when the shares are below threshold.
    fn sign(&self, msg: &[u8], parties: &[&[u16]]) -> Result<HexBinary, frost::Error> {
        let mut rng = OsRng;
        let ids: Vec<frost::Identifier> = parties
            .iter()
            .flat_map(|p| p.iter())
            .map(|i| (*i).try_into().unwrap())
            .collect();
        let mut nonces = BTreeMap::new();
        let mut commits = BTreeMap::new();
        for id in &ids {
            let (n, c) = frost::round1::commit(self.keys[id].signing_share(), &mut rng);
            nonces.insert(*id, n);
            commits.insert(*id, c);
        }
        let pkg = frost::SigningPackage::new(commits, msg);
        let mut shares = BTreeMap::new();
        for id in &ids {
            shares.insert(*id, frost::round2::sign(&pkg, &nonces[id], &self.keys[id])?);
        }
        let sig = frost::aggregate(&pkg, &shares, &self.pubkey)?;
        Ok(HexBinary::from(sig.serialize().unwrap()))
    }
}

/// Mock payment client: app_data_hash hex -> inclusion (stands in for cw-lc-zcash).
type Lc = Rc<RefCell<HashMap<String, PaymentInclusionResponse>>>;

struct W {
    d: D,
    nft: Addr,
    seller: Addr,
    bidder: Addr,
    reclaim: Group,
    fin: Group,
    lc: Lc,
}
fn env_at(t: u64) -> Env {
    let mut e = mock_env();
    e.block.time = Timestamp::from_seconds(t);
    e
}

fn setup() -> W {
    let mut d = mock_dependencies();
    let (nft, seller, bidder) = (
        d.api.addr_make("orbs"),
        d.api.addr_make("seller"),
        d.api.addr_make("bidder"),
    );
    let (admin, protocol) = (d.api.addr_make("admin"), d.api.addr_make("protocol"));
    instantiate(
        d.as_mut(),
        env_at(T0),
        message_info(&admin, &[]),
        InstantiateMsg {
            protocol_fee_bps: 250,
            min_increment_bps: 500,
            min_reserve_price: Uint128::new(100),
            duration: DUR,
            extension: 300,
            accepted_denom: vec!["uterp".into()],
            protocol_addr: protocol.to_string(),
        },
    )
    .unwrap();
    let client = d.api.addr_make("zcash-lc");
    let lc: Lc = Rc::default();
    let served = lc.clone();
    d.querier.update_wasm(move |q| match q {
        WasmQuery::Smart { contract_addr, msg } if *contract_addr == client.to_string() => {
            let PaymentClientQuery::PaymentInclusion { app_data_hash_hex } = from_json(msg).unwrap();
            let r = served.borrow().get(&app_data_hash_hex).cloned();
            SystemResult::Ok(ContractResult::Ok(to_json_binary(&r).unwrap()))
        }
        _ => panic!("unexpected query {q:?}"),
    });
    let hook = Cw721HookMsg::CreatePrivateAuction(CreatePrivateAuction {
        reclaim_grace: Some(GRACE),
        seller_view_pk: HexBinary::from([5u8; 32].to_vec()),
        payment_client: d.api.addr_make("zcash-lc").to_string(),
    });
    execute(
        d.as_mut(),
        env_at(T0),
        message_info(&nft, &[]),
        ExecuteMsg::ReceiveNft(Cw721ReceiveMsg {
            sender: seller.to_string(),
            token_id: "orb-7".into(),
            msg: to_json_binary(&hook).unwrap(),
        }),
    )
    .unwrap();
    let w = W {
        d,
        nft,
        seller,
        bidder,
        reclaim: Group::dkg(5),
        fin: Group::dkg(6),
        lc,
    };
    // The seller payment of bid 1, attested Confirmed under PAYREF.
    let inc = inclusion(&w, PAY_CM, &w.reclaim.vk(), &memo(&w, 1, 1));
    w.lc.borrow_mut().insert(HexBinary::from(PAYREF.to_vec()).to_hex(), inc);
    w
}

const ESCROW: [u8; 32] = [7u8; 32];
const SALT: [u8; 32] = [9u8; 32];
/// The bid's OVK (in the seller view envelope); every release is encrypted under it.
const OVK: [u8; 32] = [0x0b; 32];
const PAY_CM: [u8; 32] = [0x5e; 32];
const PAYREF: [u8; 32] = [0xad; 32];
fn memo(w: &W, auction_id: u64, bid_index: u64) -> HexBinary {
    let env = mock_env();
    h(
        w,
        &memo_preimage(&env.block.chain_id, env.contract.address.as_str(), auction_id, bid_index),
    )
}
fn tag(w: &W, ovk: &[u8; 32]) -> HexBinary {
    h(w, &ovk_tag_preimage(ovk))
}
fn inclusion(w: &W, note: [u8; 32], group: &HexBinary, memo_cm: &HexBinary) -> PaymentInclusionResponse {
    let _ = w;
    PaymentInclusionResponse {
        inclusion: PaymentInclusion {
            app_data_hash: HexBinary::from(PAYREF.to_vec()).to_hex(),
            height: 100,
            block_hash: "ab".repeat(32),
            payment_root: "cd".repeat(32),
            source_client: "07-zap1-0".into(),
            sequence: 1,
            recorded_at: 12,
            leaf: HexBinary::from(payment_leaf(memo_cm.as_slice()).to_vec()).to_hex(),
            note: HexBinary::from(note.to_vec()).to_hex(),
            amount_zat: 4_000,
            frost_group: group.to_hex(),
        },
        status: InclusionStatus::Confirmed,
    }
}
fn h(w: &W, b: &[u8]) -> HexBinary {
    HexBinary::from(w.d.api.blake2b_256(b).to_vec())
}
fn commit(w: &W, amount_zat: u64) -> HexBinary {
    h(
        w,
        &commitment_preimage(1, w.bidder.as_str(), &ESCROW, amount_zat, &SALT),
    )
}
/// A refund over one stand-in 382-byte note `[seed; 382]`, under the bid OVK.
fn release(w: &W, seed: u8) -> (ShieldedRelease, HexBinary) {
    let r = ShieldedRelease {
        notes_cm: h(w, &notes_preimage(&[[seed; 382]])),
        ovk_ciphertext: Binary::from(vec![seed ^ 0xff; 400]),
        ovk_tag: tag(w, &OVK),
        payment_cm: None,
    };
    (r, HexBinary::from(OVK.to_vec()))
}
/// The winning release: as [`release`] plus the attested seller payment note.
fn payment(w: &W, seed: u8) -> ShieldedRelease {
    ShieldedRelease {
        payment_cm: Some(HexBinary::from(PAY_CM.to_vec())),
        ..release(w, seed).0
    }
}
fn dig(
    w: &W,
    action: PrivateAction,
    bid_id: Option<u64>,
    r: Option<&ShieldedRelease>,
) -> [u8; 32] {
    let env = mock_env();
    let contract = env.contract.address.to_string();
    let c = commit(w, 4_000);
    w.d.api.blake2b_256(&signing_preimage(
        &action,
        &DigestInput {
            chain_id: &env.block.chain_id,
            contract: &contract,
            auction_id: 1,
            bidder: w.bidder.as_str(),
            escrow_cm: &ESCROW,
            commitment: c.as_slice(),
            memo_cm: memo(w, 1, 1).as_slice(),
            ovk_tag: tag(w, &OVK).as_slice(),
            reclaim_vk: w.reclaim.vk().as_slice(),
            finalize_vk: w.fin.vk().as_slice(),
            bid_id,
            dkg_transcript: None,
            release: r.map(|r| {
                (
                    r.notes_cm.as_slice(),
                    w.d.api.blake2b_256(r.ovk_ciphertext.as_slice()),
                    r.ovk_tag.as_slice(),
                    r.payment_cm.as_ref().map_or(&[][..], |c| c.as_slice()),
                )
            }),
        },
    ))
}
fn bid_msg(w: &W, finalize_bind_sig: HexBinary) -> ExecuteMsg {
    bid_msg_with(w, finalize_bind_sig, memo(w, 1, 1))
}
fn bid_msg_with(w: &W, finalize_bind_sig: HexBinary, memo_cm: HexBinary) -> ExecuteMsg {
    ExecuteMsg::PlacePrivateBid(PlacePrivateBid {
        auction_id: 1,
        escrow_cm: HexBinary::from(ESCROW.to_vec()),
        commitment: commit(w, 4_000),
        // eph_pk || nonce || ct+tag (stand-in bytes; terp-orch tests the real envelope).
        sealed_bid: Binary::from(vec![0x5b; 72 + 72]),
        memo_cm,
        ovk_tag: tag(w, &OVK),
        reclaim_vk: w.reclaim.vk(),
        finalize_vk: w.fin.vk(),
        reclaim_bind_sig: w
            .reclaim
            .sign(&dig(w, PrivateAction::BindReclaim, None, None), &[B, P])
            .unwrap(),
        finalize_bind_sig,
        dkg_transcript: None,
    })
}
fn bid_with(w: &mut W, funds: &[Coin]) -> Result<Response, ContractError> {
    let fb = w
        .fin
        .sign(&dig(w, PrivateAction::BindFinalize, None, None), &[B, P, S])
        .unwrap();
    let msg = bid_msg(w, fb);
    let b = w.bidder.clone();
    execute(w.d.as_mut(), env_at(T0 + 10), message_info(&b, funds), msg)
}
fn bid(w: &mut W) -> Result<Response, ContractError> {
    bid_with(w, &[])
}
fn reclaim(
    w: &mut W,
    t: u64,
    who: &Addr,
    r: ShieldedRelease,
    sig: Option<HexBinary>,
) -> Result<Response, ContractError> {
    execute(
        w.d.as_mut(),
        env_at(t),
        message_info(who, &[]),
        ExecuteMsg::ReclaimPrivateBid {
            auction_id: 1,
            bid_id: 1,
            release: r,
            signature: sig,
        },
    )
}
fn finalize(w: &mut W, r: ShieldedRelease, sig: HexBinary) -> Result<Response, ContractError> {
    finalize_ref(w, r, sig, PAYREF)
}
fn finalize_ref(
    w: &mut W,
    r: ShieldedRelease,
    sig: HexBinary,
    payment_ref: [u8; 32],
) -> Result<Response, ContractError> {
    let relayer = w.d.api.addr_make("platform");
    execute(
        w.d.as_mut(),
        env_at(T0 + DUR + 1),
        message_info(&relayer, &[]),
        ExecuteMsg::FinalizePrivateAuction {
            auction_id: 1,
            bid_id: 1,
            release: r,
            signature: sig,
            payment_ref: HexBinary::from(payment_ref.to_vec()),
        },
    )
}
fn cancel(w: &mut W, t: u64, who: &Addr) -> Result<Response, ContractError> {
    execute(
        w.d.as_mut(),
        env_at(t),
        message_info(who, &[]),
        ExecuteMsg::CancelPrivateAuction { auction_id: 1 },
    )
}
fn bids(w: &W) -> Vec<PrivateBid> {
    from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::PrivateBids {
                auction_id: 1,
                start_after: None,
                limit: None,
            },
        )
        .unwrap(),
    )
    .unwrap()
}
fn no_bank(r: &Response) {
    assert!(
        r.messages
            .iter()
            .all(|m| !matches!(m.msg, CosmosMsg::Bank(_))),
        "private settlement must not move Terp funds"
    );
}

#[test]
fn bids_carry_no_funds_and_no_amount() {
    let mut w = setup();
    assert!(
        bid_with(&mut w, &[coin(10_000, "uterp")])
            .unwrap_err()
            .to_string()
            .contains("attach no funds")
    );
    let r = bid(&mut w).unwrap();
    assert!(!format!("{:?}", r.events).contains("4000"));
    let q = bids(&w);
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].escrow_cm.as_slice(), ESCROW);
    assert!(q[0].release.is_none() && !format!("{q:?}").contains("4000"));
}

#[test]
fn all_three_finalize_and_only_the_nft_moves() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let rel = payment(&w, 1);
    let sig = w
        .fin
        .sign(&dig(&w, PrivateAction::Finalize, Some(1), Some(&rel)), &[B, P, S])
        .unwrap();
    let r = finalize(&mut w, rel.clone(), sig).unwrap();
    no_bank(&r);
    assert_eq!(r.messages.len(), 1);
    match &r.messages[0].msg {
        CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr, msg, ..
        }) => {
            assert_eq!(contract_addr, &w.nft.to_string());
            assert!(String::from_utf8_lossy(msg).contains(w.bidder.as_str()));
        }
        m => panic!("expected nft transfer, got {m:?}"),
    }
    let a: PrivateAuction = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::PrivateAuction { auction_id: 1 },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (a.status, a.winning_bid, a.active_bids),
        (PrivateStatus::Finalized, Some(1), 0)
    );
    let b = &bids(&w)[0];
    assert_eq!((&b.status, b.release.as_ref()), (&BidStatus::Won, Some(&rel)));
}

#[test]
fn finalize_needs_every_party() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let rel = payment(&w, 1);
    let m = dig(&w, PrivateAction::Finalize, Some(1), Some(&rel));
    // 6-of-6: any coalition missing a party holds at most 5 shares and FROST refuses.
    assert!(w.fin.sign(&m, &[B, P]).is_err());
    assert!(w.fin.sign(&m, &[B, S]).is_err());
    assert!(w.fin.sign(&m, &[P, S]).is_err());
    // A {B,P} reclaim-key signature over the finalize digest is not accepted.
    let wrong = w.reclaim.sign(&m, &[B, P]).unwrap();
    assert!(
        finalize(&mut w, rel, wrong)
            .unwrap_err()
            .to_string()
            .contains("invalid group signature")
    );
}

#[test]
fn signatures_bind_the_payment_record() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let agreed = payment(&w, 1);
    let sig = w
        .fin
        .sign(
            &dig(&w, PrivateAction::Finalize, Some(1), Some(&agreed)),
            &[B, P, S],
        )
        .unwrap();
    // A relayer cannot swap the notes, the ciphertext, the OVK tag or the payment note.
    let other = ShieldedRelease {
        ovk_tag: tag(&w, &[2; 32]),
        payment_cm: Some(HexBinary::from([1u8; 32].to_vec())),
        ..release(&w, 2).0
    };
    for forged in [
        ShieldedRelease {
            notes_cm: other.notes_cm.clone(),
            ..agreed.clone()
        },
        ShieldedRelease {
            ovk_ciphertext: other.ovk_ciphertext.clone(),
            ..agreed.clone()
        },
        ShieldedRelease {
            ovk_tag: other.ovk_tag.clone(),
            ..agreed.clone()
        },
        ShieldedRelease {
            payment_cm: other.payment_cm.clone(),
            ..agreed.clone()
        },
    ] {
        assert!(
            finalize(&mut w, forged, sig.clone())
                .unwrap_err()
                .to_string()
                .contains("invalid group signature")
        );
    }
    finalize(&mut w, agreed, sig).unwrap();
}

#[test]
fn bidder_and_platform_reclaim_any_time() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let (rel, _) = release(&w, 3);
    let sig = w
        .reclaim
        .sign(&dig(&w, PrivateAction::Reclaim, Some(1), Some(&rel)), &[B, P])
        .unwrap();
    // Mid-auction, submitted by a third-party relayer.
    let relayer = w.d.api.addr_make("relayer");
    let r = reclaim(&mut w, T0 + 20, &relayer, rel.clone(), Some(sig.clone())).unwrap();
    no_bank(&r);
    assert_eq!(bids(&w)[0].status, BidStatus::Reclaimed);
    // Single use.
    assert!(
        reclaim(&mut w, T0 + 21, &relayer, rel, Some(sig))
            .unwrap_err()
            .to_string()
            .contains("not active")
    );
}

#[test]
fn platform_and_seller_can_never_act() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let (rel, _) = release(&w, 3);
    let rm = dig(&w, PrivateAction::Reclaim, Some(1), Some(&rel));
    let fm = dig(&w, PrivateAction::Finalize, Some(1), Some(&rel));
    // P+S hold 3 of 6 shares: below 5 (reclaim) and 6 (finalize).
    assert!(w.reclaim.sign(&rm, &[P, S]).is_err());
    assert!(w.fin.sign(&fm, &[P, S]).is_err());
    // Without a signature only the bidder may reclaim, and only after close/timeout.
    let (p, s) = (w.d.api.addr_make("platform"), w.seller.clone());
    for who in [p, s] {
        assert!(matches!(
            reclaim(&mut w, T0 + DUR + GRACE + 1, &who, rel.clone(), None),
            Err(ContractError::Unauthorized)
        ));
    }
    // Bidder alone (3 shares) and bidder+seller (4) cannot reach the reclaim threshold either.
    assert!(w.reclaim.sign(&rm, &[B]).is_err());
    assert!(w.reclaim.sign(&rm, &[B, S]).is_err());
}

#[test]
fn bidder_alone_reclaims_after_close_or_timeout() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let (rel, _) = release(&w, 3);
    let b = w.bidder.clone();
    let e = reclaim(&mut w, T0 + DUR + 1, &b, rel.clone(), None).unwrap_err();
    assert!(e.to_string().contains("grace"));
    reclaim(&mut w, T0 + DUR + GRACE, &b, rel, None).unwrap();
    assert_eq!(bids(&w)[0].status, BidStatus::Reclaimed);
}

#[test]
fn seller_cancels_only_without_live_bids() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let (s, stranger) = (w.seller.clone(), w.d.api.addr_make("stranger"));
    assert!(matches!(
        cancel(&mut w, T0 + 30, &stranger),
        Err(ContractError::Unauthorized)
    ));
    // The live bid may already be paying the seller in ZEC.
    assert!(
        cancel(&mut w, T0 + 30, &s)
            .unwrap_err()
            .to_string()
            .contains("bids are active")
    );
    let (rel, _) = release(&w, 3);
    let sig = w
        .reclaim
        .sign(&dig(&w, PrivateAction::Reclaim, Some(1), Some(&rel)), &[B, P])
        .unwrap();
    reclaim(&mut w, T0 + 31, &stranger, rel, Some(sig)).unwrap();
    let r = cancel(&mut w, T0 + 32, &s).unwrap();
    assert_eq!(r.messages.len(), 1);
}

#[test]
fn anyone_expires_after_grace_even_with_bids() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let stranger = w.d.api.addr_make("stranger");
    let r = cancel(&mut w, T0 + DUR + GRACE, &stranger).unwrap();
    assert_eq!(r.messages.len(), 1);
    // The bidder then leaves alone.
    let (rel, _) = release(&w, 3);
    let b = w.bidder.clone();
    reclaim(&mut w, T0 + DUR + GRACE + 1, &b, rel, None).unwrap();
}

#[test]
fn ovk_disclosure_is_optional_and_checked() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let anyone = w.d.api.addr_make("anyone");
    let disclose = |w: &mut W, ovk: HexBinary| {
        execute(
            w.d.as_mut(),
            env_at(T0 + DUR + 2),
            message_info(&anyone, &[]),
            ExecuteMsg::DisclosePrivateRelease {
                auction_id: 1,
                bid_id: 1,
                ovk,
            },
        )
    };
    let (_, ovk) = release(&w, 1);
    let rel = payment(&w, 1);
    assert!(
        disclose(&mut w, ovk.clone())
            .unwrap_err()
            .to_string()
            .contains("no release")
    );
    let sig = w
        .fin
        .sign(&dig(&w, PrivateAction::Finalize, Some(1), Some(&rel)), &[B, P, S])
        .unwrap();
    finalize(&mut w, rel, sig).unwrap();
    assert!(bids(&w)[0].disclosed_ovk.is_none(), "private by default");
    let wrong = HexBinary::from([2u8; 32].to_vec());
    assert!(
        disclose(&mut w, wrong)
            .unwrap_err()
            .to_string()
            .contains("ovk_tag")
    );
    let r = disclose(&mut w, ovk.clone()).unwrap();
    assert!(format!("{:?}", r.events).contains(&ovk.to_hex()));
    assert_eq!(bids(&w)[0].disclosed_ovk, Some(ovk.clone()));
    assert!(disclose(&mut w, ovk).is_err());
}

#[test]
fn bind_requires_both_groups_and_fresh_keys() {
    let mut w = setup();
    // Finalize bind signed by the reclaim group: seller never joined, rejected.
    let fake = w
        .reclaim
        .sign(&dig(&w, PrivateAction::BindFinalize, None, None), &[B, P])
        .unwrap();
    let m = bid_msg(&w, fake);
    let b = w.bidder.clone();
    let e = execute(w.d.as_mut(), env_at(T0 + 10), message_info(&b, &[]), m).unwrap_err();
    assert!(e.to_string().contains("finalize bind"));
    bid(&mut w).unwrap();
    // The same group keys cannot back a second bid.
    assert!(
        bid(&mut w)
            .unwrap_err()
            .to_string()
            .contains("already used")
    );
}

#[test]
fn digests_bind_chain_contract_action_and_release() {
    let w = setup();
    let (rel, _) = release(&w, 1);
    let a = dig(&w, PrivateAction::Reclaim, Some(1), Some(&rel));
    assert_ne!(a, dig(&w, PrivateAction::Finalize, Some(1), Some(&rel)));
    assert_ne!(a, dig(&w, PrivateAction::Reclaim, Some(2), Some(&rel)));
    assert_ne!(a, dig(&w, PrivateAction::Reclaim, Some(1), Some(&release(&w, 2).0)));
    let q: HexBinary = from_json(
        query(
            w.d.as_ref(),
            mock_env(),
            QueryMsg::PrivateSigningDigest {
                action: PrivateAction::Reclaim,
                auction_id: 1,
                bidder: w.bidder.to_string(),
                escrow_cm: HexBinary::from(ESCROW.to_vec()),
                commitment: commit(&w, 4_000),
                memo_cm: memo(&w, 1, 1),
                ovk_tag: tag(&w, &OVK),
                reclaim_vk: w.reclaim.vk(),
                finalize_vk: w.fin.vk(),
                bid_id: Some(1),
                dkg_transcript: None,
                release: Some(rel),
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(q.as_slice(), a.as_slice());
    let p: PrivatePolicyResponse =
        from_json(query(w.d.as_ref(), mock_env(), QueryMsg::PrivatePolicy {}).unwrap()).unwrap();
    assert_eq!(
        (p.reclaim_threshold, p.finalize_threshold, p.total_shares),
        (5, 6, 6)
    );
}

#[test]
fn memo_must_label_this_auction_contract_and_bid_index() {
    let mut w = setup();
    let fb = w
        .fin
        .sign(&dig(&w, PrivateAction::BindFinalize, None, None), &[B, P, S])
        .unwrap();
    let env = mock_env();
    let other_contract = h(&w, &memo_preimage(&env.block.chain_id, "terp1other", 1, 1));
    let other_chain = h(&w, &memo_preimage("other-chain", env.contract.address.as_str(), 1, 1));
    for wrong in [memo(&w, 2, 1), memo(&w, 1, 2), other_contract, other_chain] {
        let m = bid_msg_with(&w, fb.clone(), wrong);
        let b = w.bidder.clone();
        let e = execute(w.d.as_mut(), env_at(T0 + 10), message_info(&b, &[]), m).unwrap_err();
        assert!(e.to_string().contains("memo_cm"), "{e}");
    }
    bid(&mut w).unwrap();
    assert_eq!(bids(&w)[0].memo_cm, memo(&w, 1, 1));
}

#[test]
fn releases_use_the_ovk_fixed_at_bid_time() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let rel = ShieldedRelease {
        ovk_tag: tag(&w, &[2; 32]),
        ..release(&w, 3).0
    };
    let sig = w
        .reclaim
        .sign(&dig(&w, PrivateAction::Reclaim, Some(1), Some(&rel)), &[B, P])
        .unwrap();
    let relayer = w.d.api.addr_make("relayer");
    let e = reclaim(&mut w, T0 + 20, &relayer, rel, Some(sig)).unwrap_err();
    assert!(e.to_string().contains("ovk_tag"), "{e}");
}

#[test]
fn finalize_waits_for_a_confirmed_zcash_payment_of_this_bid() {
    let mut w = setup();
    bid(&mut w).unwrap();
    let rel = payment(&w, 1);
    let sig = w
        .fin
        .sign(&dig(&w, PrivateAction::Finalize, Some(1), Some(&rel)), &[B, P, S])
        .unwrap();
    let key = HexBinary::from(PAYREF.to_vec()).to_hex();
    let good = w.lc.borrow()[&key].clone();
    let other_group = w.fin.vk();
    let cases: Vec<(PaymentInclusionResponse, &str)> = vec![
        (
            PaymentInclusionResponse {
                status: InclusionStatus::Preconfirmed,
                ..good.clone()
            },
            "not Confirmed",
        ),
        (
            PaymentInclusionResponse {
                status: InclusionStatus::Reorged,
                ..good.clone()
            },
            "not Confirmed",
        ),
        (inclusion(&w, [1; 32], &w.reclaim.vk(), &memo(&w, 1, 1)), "signed seller payment"),
        (inclusion(&w, PAY_CM, &other_group, &memo(&w, 1, 1)), "escrow group"),
        (inclusion(&w, PAY_CM, &w.reclaim.vk(), &memo(&w, 1, 2)), "memo"),
        (inclusion(&w, PAY_CM, &w.reclaim.vk(), &memo(&w, 2, 1)), "memo"),
    ];
    for (inc, want) in cases {
        w.lc.borrow_mut().insert(key.clone(), inc);
        let e = finalize(&mut w, rel.clone(), sig.clone()).unwrap_err();
        assert!(e.to_string().contains(want), "{want}: {e}");
    }
    let e = finalize_ref(&mut w, rel.clone(), sig.clone(), [0xee; 32]).unwrap_err();
    assert!(e.to_string().contains("no Zcash payment"), "{e}");
    // A refund-shaped release (no payment note) cannot finalize.
    let (refund, _) = release(&w, 1);
    let rs = w
        .fin
        .sign(&dig(&w, PrivateAction::Finalize, Some(1), Some(&refund)), &[B, P, S])
        .unwrap();
    assert!(finalize(&mut w, refund, rs).unwrap_err().to_string().contains("payment_cm"));
    // Nothing moved so far; the attested payment releases the NFT.
    assert_eq!(bids(&w)[0].status, BidStatus::Active);
    w.lc.borrow_mut().insert(key, good);
    let r = finalize(&mut w, rel, sig).unwrap();
    assert!(format!("{:?}", r.events).contains("paid_zat"));
    assert_eq!(bids(&w)[0].status, BidStatus::Won);
}
