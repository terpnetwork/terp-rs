//! Private (sealed) auctions settled in shielded ZEC, with a fresh weighted FROST
//! multisig per bid.
//!
//! Spec: `~/grok-bot/privacy-products/auctions/SPEC.md`.
//!
//! Per bid, bidder (B), platform (P) and seller (S) run two FROST(Ed25519, SHA-512)
//! DKGs over the same six-share allocation: B holds shares 1-3, P holds 4-5, S holds 6.
//! * `reclaim_vk` is a 5-of-6 group key: only {B,P} or {B,P,S} reach 5 shares. It is
//!   also the spend authority of the bid's shielded ZEC escrow note (`escrow_cm`).
//! * `finalize_vk` is a 6-of-6 group key: only {B,P,S}.
//!
//! No funds touch this contract. The bid amount, the escrow value and every payment
//! destination live in the Zcash shielded pool; Terp only sees 32-byte commitments.
//! Reclaim and finalize each record a [`ShieldedRelease`]: a commitment to the Zcash
//! payment notes plus those notes encrypted under a one-time outgoing viewing key
//! (OVK). Publishing the OVK later ([`disclose`]) lets anyone decrypt the notes and
//! check them against the commitment. Nothing else about the payment is public.
//!
//! Seller view: each bid is labelled by a memo commitment ([`memo_preimage`]: contract,
//! lot id, bid index) that the contract recomputes, and carries `sealed_bid`, the bid's
//! OVK and opening encrypted to the seller's registered `seller_view_pk`. The seller
//! decrypts every bid, ranks them, and later reads the release under the same OVK
//! (`ovk_tag` is fixed at bid time).
//!
//! Atomicity: finalize moves the NFT only against a Confirmed zap-1 payment inclusion
//! from the auction's `payment_client` (cw-lc-zcash `PaymentInclusion`) whose note is the
//! signed seller payment (`payment_cm`), whose FROST group is the bid's escrow key
//! (`reclaim_vk`), and whose zap-1 leaf names the bid memo ([`payment_leaf`]). See
//! `~/grok-bot/privacy-products/auctions/ATOMICITY.md`.
//!
//! FROST-Ed25519 signatures are plain RFC 8032 signatures, so the contract checks them
//! with `Api::ed25519_verify` and never learns which shares signed. All hashes are the
//! host `Api::blake2b_256` (BLAKE2b-256, unkeyed).
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    Addr, Api, Binary, Deps, DepsMut, Env, Event, HexBinary, MessageInfo, Order, Response,
    StdResult, Timestamp,
};
use zap1_verify::{EventPayload, compute_leaf_hash};
use cw_storage_plus::{Bound, Item, Map};

use crate::{Config, ContractError, Cw721ReceiveMsg, err, send_nft_back};

/// Domain for every signed private-auction digest.
pub const SIGN_DOMAIN: &[u8] = b"terp/svg-auction/private-bid/v2";
/// Domain for the sealed bid-amount commitment (opened to the seller off chain only).
pub const COMMIT_DOMAIN: &[u8] = b"terp/svg-auction/bid-commit/v2";
/// Domain for the commitment to a release's Zcash payment notes.
pub const NOTES_DOMAIN: &[u8] = b"terp/svg-auction/release-notes/v1";
/// Domain for the OVK tag: `ovk_tag = blake2b(OVK_DOMAIN || ovk)`.
pub const OVK_DOMAIN: &[u8] = b"terp/svg-auction/release-ovk/v1";
/// Domain of the bid memo commitment that labels an auction instance and bid index.
pub const MEMO_DOMAIN: &[u8] = b"terp/svg-auction/bid-memo/v1";
/// Prefix of the Zcash memo text / zap-1 `program_entry` id naming a bid (seam-style
/// reference: prefix + 64 lowercase hex digits of `memo_cm`).
pub const PAYMENT_MEMO_PREFIX: &str = "tsa1:";
/// Seller view envelope: ephemeral X25519 key (32) + XChaCha20 nonce (24) + Poly1305 tag (16).
pub const VIEW_ENVELOPE_OVERHEAD: usize = 72;
/// Share allocation and thresholds (documentation + query surface).
pub const BIDDER_SHARES: u16 = 3;
pub const PLATFORM_SHARES: u16 = 2;
pub const SELLER_SHARES: u16 = 1;
pub const TOTAL_SHARES: u16 = 6;
pub const RECLAIM_THRESHOLD: u16 = 5;
pub const FINALIZE_THRESHOLD: u16 = 6;
/// Upper bound on the opaque seller ciphertext.
pub const MAX_SEALED_BID: usize = 1024;
/// Upper bound on a release's OVK ciphertext (a few SEAM-NOTE-OUT envelopes).
pub const MAX_OVK_CIPHERTEXT: usize = 8192;

#[cw_serde]
pub struct CreatePrivateAuction {
    /// Seconds after `end_time` before a bidder may reclaim alone and anyone may expire
    /// the auction. Defaults to the config `duration`.
    pub reclaim_grace: Option<u64>,
    /// 32-byte X25519 public key bids encrypt their OVK and opening to.
    pub seller_view_pk: HexBinary,
    /// Zcash light client (cw-lc-zcash) whose `PaymentInclusion` gates finalize.
    pub payment_client: String,
}

#[cw_serde]
pub enum PrivateAction {
    /// Proof-of-possession of `reclaim_vk` at bid time (5-of-6 => B and P present).
    BindReclaim,
    /// Proof-of-possession of `finalize_vk` at bid time (6-of-6 => B, P and S present).
    BindFinalize,
    /// Close the bid and record the shielded refund (signed by `reclaim_vk`).
    Reclaim,
    /// Settle the auction to this bid and record the shielded payment (`finalize_vk`).
    Finalize,
}
impl PrivateAction {
    fn tag(&self) -> u8 {
        match self {
            PrivateAction::BindReclaim => 1,
            PrivateAction::BindFinalize => 2,
            PrivateAction::Reclaim => 3,
            PrivateAction::Finalize => 4,
        }
    }
}

/// A Zcash payment authorised by a bid group, as recorded on Terp.
#[cw_serde]
pub struct ShieldedRelease {
    /// `blake2b(NOTES_DOMAIN || n || note_1 || .. || note_n)` over the 382-byte
    /// SEAM-NOTE-OUT encodings of the payment outputs (see [`notes_preimage`]). Each note
    /// carries a random `rcm`, so the commitment hides values and destinations.
    pub notes_cm: HexBinary,
    /// The same notes encrypted under the bid's OVK. Opaque on chain.
    pub ovk_ciphertext: Binary,
    /// `blake2b(OVK_DOMAIN || ovk)`; must equal the bid's `ovk_tag`.
    pub ovk_tag: HexBinary,
    /// Finalize only: note commitment of the seller's payment output, which the payment
    /// client must attest as included on Zcash.
    pub payment_cm: Option<HexBinary>,
}

#[cw_serde]
pub struct PlacePrivateBid {
    pub auction_id: u64,
    /// Note commitment of the shielded ZEC escrow note, spendable by `reclaim_vk`.
    pub escrow_cm: HexBinary,
    /// blake2b(COMMIT_DOMAIN || auction_id || bidder || escrow_cm || amount_zat || salt).
    pub commitment: HexBinary,
    /// Seller view envelope: `eph_pk || nonce || XChaCha20-Poly1305(ovk, amount, salt)`
    /// to `seller_view_pk`, AAD `memo_cm` (see SPEC "seller view").
    pub sealed_bid: Binary,
    /// `blake2b(memo_preimage(chain, contract, auction_id, bid_index))`, bid_index = the
    /// bid id this bid will get. The contract recomputes it.
    pub memo_cm: HexBinary,
    /// `blake2b(OVK_DOMAIN || ovk)` of the bid's OVK; every release must use it.
    pub ovk_tag: HexBinary,
    /// 32-byte FROST-Ed25519 group key, 5-of-6.
    pub reclaim_vk: HexBinary,
    /// 32-byte FROST-Ed25519 group key, 6-of-6.
    pub finalize_vk: HexBinary,
    /// Signature by `reclaim_vk` over the `BindReclaim` digest.
    pub reclaim_bind_sig: HexBinary,
    /// Signature by `finalize_vk` over the `BindFinalize` digest.
    pub finalize_bind_sig: HexBinary,
    /// Optional hash of the DKG transcript (identifier -> party map), bound in both
    /// bind digests so all three parties attest the same share allocation.
    pub dkg_transcript: Option<HexBinary>,
}

#[cw_serde]
pub enum PrivateStatus {
    Open,
    Finalized,
    Cancelled,
}
#[cw_serde]
pub enum BidStatus {
    Active,
    Reclaimed,
    Won,
}

#[cw_serde]
pub struct PrivateAuction {
    pub auction_id: u64,
    pub nft_contract: Addr,
    pub token_id: String,
    pub seller: Addr,
    pub seller_view_pk: HexBinary,
    pub payment_client: Addr,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    pub reclaim_grace: u64,
    pub status: PrivateStatus,
    pub winning_bid: Option<u64>,
    pub bid_count: u64,
    /// Bids neither reclaimed nor won. The seller may only cancel at zero.
    pub active_bids: u64,
}

#[cw_serde]
pub struct PrivateBid {
    pub bid_id: u64,
    pub bidder: Addr,
    pub escrow_cm: HexBinary,
    pub commitment: HexBinary,
    pub sealed_bid: Binary,
    pub memo_cm: HexBinary,
    pub ovk_tag: HexBinary,
    pub reclaim_vk: HexBinary,
    pub finalize_vk: HexBinary,
    pub dkg_transcript: Option<HexBinary>,
    pub placed_at: Timestamp,
    pub status: BidStatus,
    /// Refund (reclaimed) or payment (won) record.
    pub release: Option<ShieldedRelease>,
    /// Set once someone publishes the release OVK.
    pub disclosed_ovk: Option<HexBinary>,
}

#[cw_serde]
pub struct PrivatePolicyResponse {
    pub bidder_shares: u16,
    pub platform_shares: u16,
    pub seller_shares: u16,
    pub total_shares: u16,
    pub reclaim_threshold: u16,
    pub finalize_threshold: u16,
    pub sign_domain: String,
    pub commit_domain: String,
    pub notes_domain: String,
    pub ovk_domain: String,
    pub memo_domain: String,
    pub payment_memo_prefix: String,
}

const PRIV_NEXT_ID: Item<u64> = Item::new("p_next_id");
const PRIV_AUCTIONS: Map<u64, PrivateAuction> = Map::new("p_auctions");
const PRIV_ACTIVE: Map<(&Addr, &str), u64> = Map::new("p_active");
const PRIV_BIDS: Map<(u64, u64), PrivateBid> = Map::new("p_bids");
/// (auction_id, reclaim_vk) -> bid_id. A group key backs at most one bid per auction.
const PRIV_KEYS: Map<(u64, &[u8]), u64> = Map::new("p_keys");
/// Payment inclusions (app_data_hash) already used to settle an auction.
const PRIV_PAYMENTS: Map<&[u8], u64> = Map::new("p_payments");

fn lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Preimage of the sealed bid commitment (checked by the seller, never on chain).
pub fn commitment_preimage(
    auction_id: u64,
    bidder: &str,
    escrow_cm: &[u8; 32],
    amount_zat: u64,
    salt: &[u8; 32],
) -> Vec<u8> {
    let mut v = Vec::with_capacity(160);
    v.extend_from_slice(COMMIT_DOMAIN);
    v.extend_from_slice(&auction_id.to_be_bytes());
    lp(&mut v, bidder.as_bytes());
    v.extend_from_slice(escrow_cm);
    v.extend_from_slice(&amount_zat.to_be_bytes());
    v.extend_from_slice(salt);
    v
}

/// Preimage of the bid memo commitment: labels contract, lot (`auction_id`) and bid index.
pub fn memo_preimage(chain_id: &str, contract: &str, auction_id: u64, bid_index: u64) -> Vec<u8> {
    let mut v = MEMO_DOMAIN.to_vec();
    lp(&mut v, chain_id.as_bytes());
    lp(&mut v, contract.as_bytes());
    v.extend_from_slice(&auction_id.to_be_bytes());
    v.extend_from_slice(&bid_index.to_be_bytes());
    v
}

/// Memo text a bid's Zcash payment carries, and its zap-1 `program_entry` id.
pub fn payment_memo(memo_cm: &[u8]) -> String {
    format!("{PAYMENT_MEMO_PREFIX}{}", HexBinary::from(memo_cm).to_hex())
}

/// zap-1 leaf the payment client must attest for a bid (`program_entry` of [`payment_memo`]).
pub fn payment_leaf(memo_cm: &[u8]) -> [u8; 32] {
    compute_leaf_hash(&EventPayload::ProgramEntry {
        wallet_hash: payment_memo(memo_cm).as_bytes(),
    })
}

/// Preimage of [`ShieldedRelease::notes_cm`] over serialized payment notes.
pub fn notes_preimage<N: AsRef<[u8]>>(notes: &[N]) -> Vec<u8> {
    let mut v = NOTES_DOMAIN.to_vec();
    v.extend_from_slice(&(notes.len() as u16).to_be_bytes());
    for n in notes {
        lp(&mut v, n.as_ref());
    }
    v
}

/// Preimage of [`ShieldedRelease::ovk_tag`].
pub fn ovk_tag_preimage(ovk: &[u8]) -> Vec<u8> {
    [OVK_DOMAIN, ovk].concat()
}

/// Inputs bound into every signed digest.
pub struct DigestInput<'a> {
    pub chain_id: &'a str,
    pub contract: &'a str,
    pub auction_id: u64,
    pub bidder: &'a str,
    pub escrow_cm: &'a [u8],
    pub commitment: &'a [u8],
    pub memo_cm: &'a [u8],
    pub ovk_tag: &'a [u8],
    pub reclaim_vk: &'a [u8],
    pub finalize_vk: &'a [u8],
    pub bid_id: Option<u64>,
    pub dkg_transcript: Option<&'a [u8]>,
    /// Reclaim / Finalize only: the release fields bound into the digest.
    pub release: Option<ReleaseFields<'a>>,
}

/// (notes_cm, blake2b(ovk_ciphertext), ovk_tag, payment_cm or empty).
pub type ReleaseFields<'a> = (&'a [u8], [u8; 32], &'a [u8], &'a [u8]);

/// Preimage of the digest a FROST group signs for `action`. Hash with BLAKE2b-256.
pub fn signing_preimage(action: &PrivateAction, d: &DigestInput) -> Vec<u8> {
    let mut v = Vec::with_capacity(320);
    v.extend_from_slice(SIGN_DOMAIN);
    v.push(action.tag());
    lp(&mut v, d.chain_id.as_bytes());
    lp(&mut v, d.contract.as_bytes());
    v.extend_from_slice(&d.auction_id.to_be_bytes());
    lp(&mut v, d.bidder.as_bytes());
    lp(&mut v, d.escrow_cm);
    lp(&mut v, d.commitment);
    lp(&mut v, d.memo_cm);
    lp(&mut v, d.ovk_tag);
    lp(&mut v, d.reclaim_vk);
    lp(&mut v, d.finalize_vk);
    v.extend_from_slice(&d.bid_id.unwrap_or(0).to_be_bytes());
    lp(&mut v, d.dkg_transcript.unwrap_or(&[]));
    if let Some((notes_cm, ct_hash, ovk_tag, payment_cm)) = d.release {
        lp(&mut v, notes_cm);
        lp(&mut v, &ct_hash);
        lp(&mut v, ovk_tag);
        lp(&mut v, payment_cm);
    }
    v
}

fn digest(api: &dyn Api, action: &PrivateAction, d: &DigestInput) -> [u8; 32] {
    api.blake2b_256(&signing_preimage(action, d))
}

fn check_sig(
    api: &dyn Api,
    msg: &[u8; 32],
    sig: &HexBinary,
    vk: &HexBinary,
    what: &str,
) -> Result<(), ContractError> {
    if sig.len() != 64 {
        return err(format!("{what}: signature must be 64 bytes"));
    }
    match api.ed25519_verify(msg, sig.as_slice(), vk.as_slice()) {
        Ok(true) => Ok(()),
        Ok(false) => err(format!("{what}: invalid group signature")),
        Err(e) => err(format!("{what}: {e}")),
    }
}

fn need32(h: &HexBinary, what: &str) -> Result<(), ContractError> {
    if h.len() != 32 {
        return err(format!("{what} must be 32 bytes"));
    }
    Ok(())
}

fn no_funds(info: &MessageInfo) -> Result<(), ContractError> {
    if !info.funds.is_empty() {
        return err("private auctions settle in shielded ZEC: attach no funds");
    }
    Ok(())
}

fn check_release(r: &ShieldedRelease) -> Result<(), ContractError> {
    need32(&r.notes_cm, "release notes_cm")?;
    need32(&r.ovk_tag, "release ovk_tag")?;
    if r.ovk_ciphertext.is_empty() || r.ovk_ciphertext.len() > MAX_OVK_CIPHERTEXT {
        return err(format!("ovk_ciphertext must be 1..={MAX_OVK_CIPHERTEXT} bytes"));
    }
    if let Some(cm) = &r.payment_cm {
        need32(cm, "release payment_cm")?;
    }
    Ok(())
}

pub fn create(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    c: &Config,
    r: Cw721ReceiveMsg,
    p: CreatePrivateAuction,
) -> Result<Response, ContractError> {
    let nft = info.sender;
    let seller = deps.api.addr_validate(&r.sender)?;
    need32(&p.seller_view_pk, "seller_view_pk")?;
    let payment_client = deps.api.addr_validate(&p.payment_client)?;
    let id = PRIV_NEXT_ID.may_load(deps.storage)?.unwrap_or(1);
    PRIV_NEXT_ID.save(deps.storage, &(id + 1))?;
    let a = PrivateAuction {
        auction_id: id,
        nft_contract: nft.clone(),
        token_id: r.token_id.clone(),
        seller: seller.clone(),
        seller_view_pk: p.seller_view_pk,
        payment_client,
        start_time: env.block.time,
        end_time: env.block.time.plus_seconds(c.duration),
        reclaim_grace: p.reclaim_grace.unwrap_or(c.duration),
        status: PrivateStatus::Open,
        winning_bid: None,
        bid_count: 0,
        active_bids: 0,
    };
    PRIV_AUCTIONS.save(deps.storage, id, &a)?;
    PRIV_ACTIVE.save(deps.storage, (&nft, &r.token_id), &id)?;
    Ok(Response::new().add_event(
        Event::new("private-auction-create")
            .add_attribute("auction_id", id.to_string())
            .add_attribute("nft_contract", nft)
            .add_attribute("token_id", r.token_id)
            .add_attribute("seller", seller)
            .add_attribute("end_time", a.end_time.seconds().to_string()),
    ))
}

pub fn place_bid(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    b: PlacePrivateBid,
) -> Result<Response, ContractError> {
    no_funds(&info)?;
    let mut a = PRIV_AUCTIONS.load(deps.storage, b.auction_id)?;
    if a.status != PrivateStatus::Open || env.block.time >= a.end_time {
        return err("private auction is not open for bids");
    }
    if info.sender == a.seller {
        return err("seller cannot bid");
    }
    need32(&b.escrow_cm, "escrow_cm")?;
    need32(&b.commitment, "commitment")?;
    need32(&b.memo_cm, "memo_cm")?;
    need32(&b.ovk_tag, "ovk_tag")?;
    need32(&b.reclaim_vk, "reclaim_vk")?;
    need32(&b.finalize_vk, "finalize_vk")?;
    if let Some(t) = &b.dkg_transcript {
        need32(t, "dkg_transcript")?;
    }
    if b.reclaim_vk == b.finalize_vk {
        return err("reclaim_vk and finalize_vk must be distinct DKG outputs");
    }
    if b.sealed_bid.len() <= VIEW_ENVELOPE_OVERHEAD || b.sealed_bid.len() > MAX_SEALED_BID {
        return err(format!(
            "sealed_bid must be a seller view envelope of {}..={MAX_SEALED_BID} bytes",
            VIEW_ENVELOPE_OVERHEAD + 1
        ));
    }
    if PRIV_KEYS.has(deps.storage, (b.auction_id, b.reclaim_vk.as_slice())) {
        return err("group key already used for this auction");
    }
    let contract = env.contract.address.to_string();
    let bid_id = a.bid_count + 1;
    let memo = memo_preimage(&env.block.chain_id, &contract, b.auction_id, bid_id);
    if deps.api.blake2b_256(&memo) != b.memo_cm.as_slice() {
        return err("memo_cm does not label this auction contract, lot and bid index");
    }
    let bidder = info.sender.to_string();
    let d = DigestInput {
        chain_id: &env.block.chain_id,
        contract: &contract,
        auction_id: b.auction_id,
        bidder: &bidder,
        escrow_cm: b.escrow_cm.as_slice(),
        commitment: b.commitment.as_slice(),
        memo_cm: b.memo_cm.as_slice(),
        ovk_tag: b.ovk_tag.as_slice(),
        reclaim_vk: b.reclaim_vk.as_slice(),
        finalize_vk: b.finalize_vk.as_slice(),
        bid_id: None,
        dkg_transcript: b.dkg_transcript.as_ref().map(|t| t.as_slice()),
        release: None,
    };
    let api = deps.api;
    check_sig(
        api,
        &digest(api, &PrivateAction::BindReclaim, &d),
        &b.reclaim_bind_sig,
        &b.reclaim_vk,
        "reclaim bind",
    )?;
    check_sig(
        api,
        &digest(api, &PrivateAction::BindFinalize, &d),
        &b.finalize_bind_sig,
        &b.finalize_vk,
        "finalize bind",
    )?;
    a.bid_count = bid_id;
    a.active_bids += 1;
    PRIV_KEYS.save(
        deps.storage,
        (b.auction_id, b.reclaim_vk.as_slice()),
        &bid_id,
    )?;
    PRIV_BIDS.save(
        deps.storage,
        (b.auction_id, bid_id),
        &PrivateBid {
            bid_id,
            bidder: info.sender.clone(),
            escrow_cm: b.escrow_cm,
            commitment: b.commitment,
            sealed_bid: b.sealed_bid,
            memo_cm: b.memo_cm,
            ovk_tag: b.ovk_tag,
            reclaim_vk: b.reclaim_vk,
            finalize_vk: b.finalize_vk,
            dkg_transcript: b.dkg_transcript,
            placed_at: env.block.time,
            status: BidStatus::Active,
            release: None,
            disclosed_ovk: None,
        },
    )?;
    PRIV_AUCTIONS.save(deps.storage, b.auction_id, &a)?;
    Ok(Response::new().add_event(
        Event::new("private-bid")
            .add_attribute("auction_id", b.auction_id.to_string())
            .add_attribute("bid_id", bid_id.to_string())
            .add_attribute("bidder", info.sender),
    ))
}

fn release_fields<'a>(api: &dyn Api, r: &'a ShieldedRelease) -> (&'a [u8], [u8; 32], &'a [u8], &'a [u8]) {
    (
        r.notes_cm.as_slice(),
        api.blake2b_256(r.ovk_ciphertext.as_slice()),
        r.ovk_tag.as_slice(),
        r.payment_cm.as_ref().map_or(&[][..], |c| c.as_slice()),
    )
}

/// Every release is encrypted under the OVK the seller received at bid time.
fn check_bid_release(bid: &PrivateBid, r: &ShieldedRelease) -> Result<(), ContractError> {
    if r.ovk_tag != bid.ovk_tag {
        return err("release ovk_tag is not the bid's ovk_tag");
    }
    Ok(())
}

fn bid_digest(
    api: &dyn Api,
    env: &Env,
    action: &PrivateAction,
    auction_id: u64,
    bid: &PrivateBid,
    r: &ShieldedRelease,
) -> [u8; 32] {
    let contract = env.contract.address.to_string();
    let bidder = bid.bidder.to_string();
    digest(
        api,
        action,
        &DigestInput {
            chain_id: &env.block.chain_id,
            contract: &contract,
            auction_id,
            bidder: &bidder,
            escrow_cm: bid.escrow_cm.as_slice(),
            commitment: bid.commitment.as_slice(),
            memo_cm: bid.memo_cm.as_slice(),
            ovk_tag: bid.ovk_tag.as_slice(),
            reclaim_vk: bid.reclaim_vk.as_slice(),
            finalize_vk: bid.finalize_vk.as_slice(),
            bid_id: Some(bid.bid_id),
            dkg_transcript: bid.dkg_transcript.as_ref().map(|t| t.as_slice()),
            release: Some(release_fields(api, r)),
        },
    )
}

pub fn reclaim(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    auction_id: u64,
    bid_id: u64,
    release: ShieldedRelease,
    signature: Option<HexBinary>,
) -> Result<Response, ContractError> {
    no_funds(&info)?;
    check_release(&release)?;
    let mut a = PRIV_AUCTIONS.load(deps.storage, auction_id)?;
    let mut bid = PRIV_BIDS.load(deps.storage, (auction_id, bid_id))?;
    if bid.status != BidStatus::Active {
        return err("bid is not active");
    }
    check_bid_release(&bid, &release)?;
    if release.payment_cm.is_some() {
        return err("a refund carries no payment_cm");
    }
    let path = match signature {
        // {B,P} (5 of 6 shares): any relayer may submit; the refund notes are bound.
        Some(sig) => {
            let msg = bid_digest(
                deps.api,
                &env,
                &PrivateAction::Reclaim,
                auction_id,
                &bid,
                &release,
            );
            check_sig(deps.api, &msg, &sig, &bid.reclaim_vk, "reclaim")?;
            "group"
        }
        // Decided liveness fallback: the bidder alone, after close or the grace period,
        // records the refund that {B,P} pre-signed on the Zcash side at bid time.
        None => {
            if info.sender != bid.bidder {
                return Err(ContractError::Unauthorized);
            }
            let timed_out = env.block.time >= a.end_time.plus_seconds(a.reclaim_grace);
            if a.status == PrivateStatus::Open && !timed_out {
                return err("unilateral reclaim only after the auction closes or the grace period");
            }
            "timeout"
        }
    };
    bid.status = BidStatus::Reclaimed;
    bid.release = Some(release);
    a.active_bids -= 1;
    PRIV_BIDS.save(deps.storage, (auction_id, bid_id), &bid)?;
    PRIV_AUCTIONS.save(deps.storage, auction_id, &a)?;
    Ok(Response::new().add_event(
        Event::new("private-bid-reclaim")
            .add_attribute("auction_id", auction_id.to_string())
            .add_attribute("bid_id", bid_id.to_string())
            .add_attribute("path", path),
    ))
}

// --- payment client (cw-lc-zcash) ------------------------------------------------------
// Exact wire mirror of cw-lc-zcash `QueryMsg::PaymentInclusion`, `PaymentInclusionResponse`,
// `InclusionStatus` and `state::Inclusion` (cw_serde denies unknown fields, so every field
// is listed), kept local so the auction wasm does not link the light client. terp-orch
// checks the round trip against `cw_lc_zcash::msg` (bid_view tests). Only Confirmed releases.

#[cw_serde]
pub enum PaymentClientQuery {
    PaymentInclusion { app_data_hash_hex: String },
}

#[cw_serde]
pub enum InclusionStatus {
    Preconfirmed,
    Confirmed,
    Reorged,
}

#[cw_serde]
pub struct PaymentInclusion {
    pub app_data_hash: String,
    pub height: u64,
    pub block_hash: String,
    pub payment_root: String,
    pub leaf: String,
    pub source_client: String,
    pub sequence: u64,
    pub note: String,
    pub amount_zat: u64,
    pub frost_group: String,
    pub recorded_at: u64,
}

#[cw_serde]
pub struct PaymentInclusionResponse {
    pub inclusion: PaymentInclusion,
    pub status: InclusionStatus,
}

/// The seller payment of `bid` is on Zcash: a Confirmed inclusion keyed `payment_ref`
/// (zap-1 `app_data_hash`) whose note is `payment_cm`, whose FROST group is the escrow
/// key `reclaim_vk`, and whose zap-1 leaf names the bid memo. Returns the paid amount.
fn check_payment(
    deps: Deps,
    a: &PrivateAuction,
    bid: &PrivateBid,
    payment_cm: &HexBinary,
    payment_ref: &HexBinary,
) -> Result<u64, ContractError> {
    need32(payment_ref, "payment_ref")?;
    if PRIV_PAYMENTS.has(deps.storage, payment_ref.as_slice()) {
        return err("payment inclusion already settled an auction");
    }
    let r: Option<PaymentInclusionResponse> = deps.querier.query_wasm_smart(
        &a.payment_client,
        &PaymentClientQuery::PaymentInclusion {
            app_data_hash_hex: payment_ref.to_hex(),
        },
    )?;
    let Some(r) = r else {
        return err("no Zcash payment inclusion for payment_ref");
    };
    let i = &r.inclusion;
    let eq = |got: &str, want: &[u8]| got.eq_ignore_ascii_case(&HexBinary::from(want).to_hex());
    if r.status != InclusionStatus::Confirmed {
        return err(format!("payment inclusion is {:?}, not Confirmed", r.status));
    }
    if !eq(&i.app_data_hash, payment_ref.as_slice()) {
        return err("payment client returned another inclusion");
    }
    if !eq(&i.note, payment_cm.as_slice()) {
        return err("included note is not the signed seller payment");
    }
    if !eq(&i.frost_group, bid.reclaim_vk.as_slice()) {
        return err("payment is not from this bid's escrow group");
    }
    if !eq(&i.leaf, &payment_leaf(bid.memo_cm.as_slice())) {
        return err("payment memo does not name this auction bid");
    }
    Ok(i.amount_zat)
}

/// Settle to `bid_id`. Needs the 6-of-6 signature over the payment record and a Confirmed
/// Zcash inclusion of the seller payment (`payment_ref`) before the NFT moves.
#[allow(clippy::too_many_arguments)]
pub fn finalize(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    auction_id: u64,
    bid_id: u64,
    release: ShieldedRelease,
    signature: HexBinary,
    payment_ref: HexBinary,
) -> Result<Response, ContractError> {
    no_funds(&info)?;
    check_release(&release)?;
    let mut a = PRIV_AUCTIONS.load(deps.storage, auction_id)?;
    if a.status != PrivateStatus::Open {
        return err("private auction is closed");
    }
    let mut bid = PRIV_BIDS.load(deps.storage, (auction_id, bid_id))?;
    if bid.status != BidStatus::Active {
        return err("bid is not active");
    }
    let msg = bid_digest(
        deps.api,
        &env,
        &PrivateAction::Finalize,
        auction_id,
        &bid,
        &release,
    );
    check_sig(deps.api, &msg, &signature, &bid.finalize_vk, "finalize")?;
    check_bid_release(&bid, &release)?;
    let Some(payment_cm) = &release.payment_cm else {
        return err("finalize needs release.payment_cm");
    };
    let paid = check_payment(deps.as_ref(), &a, &bid, payment_cm, &payment_ref)?;
    PRIV_PAYMENTS.save(deps.storage, payment_ref.as_slice(), &auction_id)?;
    bid.status = BidStatus::Won;
    bid.release = Some(release);
    a.status = PrivateStatus::Finalized;
    a.winning_bid = Some(bid_id);
    a.active_bids -= 1;
    PRIV_BIDS.save(deps.storage, (auction_id, bid_id), &bid)?;
    PRIV_ACTIVE.remove(deps.storage, (&a.nft_contract, &a.token_id));
    PRIV_AUCTIONS.save(deps.storage, auction_id, &a)?;
    Ok(Response::new()
        .add_message(send_nft_back(&a.nft_contract, &bid.bidder, &a.token_id)?)
        .add_event(
            Event::new("private-auction-finalize")
                .add_attribute("auction_id", auction_id.to_string())
                .add_attribute("bid_id", bid_id.to_string())
                .add_attribute("winner", bid.bidder)
                .add_attribute("payment_ref", payment_ref.to_hex())
                .add_attribute("paid_zat", paid.to_string()),
        ))
}

/// Publish the OVK of a recorded release. Anyone holding it may call; the contract only
/// checks it against `ovk_tag`, so a wrong key can never be attached.
pub fn disclose(
    deps: DepsMut,
    auction_id: u64,
    bid_id: u64,
    ovk: HexBinary,
) -> Result<Response, ContractError> {
    need32(&ovk, "ovk")?;
    let mut bid = PRIV_BIDS.load(deps.storage, (auction_id, bid_id))?;
    let Some(r) = &bid.release else {
        return err("bid has no release to disclose");
    };
    if deps.api.blake2b_256(&ovk_tag_preimage(ovk.as_slice())) != r.ovk_tag.as_slice() {
        return err("ovk does not match the release ovk_tag");
    }
    if bid.disclosed_ovk.is_some() {
        return err("release already disclosed");
    }
    bid.disclosed_ovk = Some(ovk.clone());
    PRIV_BIDS.save(deps.storage, (auction_id, bid_id), &bid)?;
    Ok(Response::new().add_event(
        Event::new("private-release-disclose")
            .add_attribute("auction_id", auction_id.to_string())
            .add_attribute("bid_id", bid_id.to_string())
            .add_attribute("ovk", ovk.to_hex()),
    ))
}

pub fn cancel(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    auction_id: u64,
) -> Result<Response, ContractError> {
    let mut a = PRIV_AUCTIONS.load(deps.storage, auction_id)?;
    if a.status != PrivateStatus::Open {
        return err("private auction is closed");
    }
    let expired = env.block.time >= a.end_time.plus_seconds(a.reclaim_grace);
    if !expired {
        if info.sender != a.seller {
            return Err(ContractError::Unauthorized);
        }
        // A live bid may already be paying the seller in ZEC; see SPEC "exchange order".
        if a.active_bids > 0 {
            return err("seller cannot cancel while bids are active");
        }
    }
    a.status = PrivateStatus::Cancelled;
    PRIV_ACTIVE.remove(deps.storage, (&a.nft_contract, &a.token_id));
    PRIV_AUCTIONS.save(deps.storage, auction_id, &a)?;
    Ok(Response::new()
        .add_message(send_nft_back(&a.nft_contract, &a.seller, &a.token_id)?)
        .add_event(
            Event::new("private-auction-cancel")
                .add_attribute("auction_id", auction_id.to_string())
                .add_attribute("expired", expired.to_string()),
        ))
}

pub fn query_auction(deps: Deps, auction_id: u64) -> StdResult<PrivateAuction> {
    PRIV_AUCTIONS.load(deps.storage, auction_id)
}

pub fn query_nft(deps: Deps, nft: &Addr, token_id: &str) -> StdResult<Option<PrivateAuction>> {
    match PRIV_ACTIVE.may_load(deps.storage, (nft, token_id))? {
        Some(id) => Ok(Some(PRIV_AUCTIONS.load(deps.storage, id)?)),
        None => Ok(None),
    }
}

pub fn query_bids(
    deps: Deps,
    auction_id: u64,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<Vec<PrivateBid>> {
    PRIV_BIDS
        .prefix(auction_id)
        .range(
            deps.storage,
            start_after.map(Bound::exclusive),
            None,
            Order::Ascending,
        )
        .take(limit.unwrap_or(30).min(100) as usize)
        .map(|r| r.map(|(_, b)| b))
        .collect()
}

/// Digest for `action`. Bind actions take the bid fields; Reclaim / Finalize also take
/// `bid_id` and the `release` they record.
#[cw_serde]
pub struct SigningDigestQuery {
    pub action: PrivateAction,
    pub auction_id: u64,
    pub bidder: String,
    pub escrow_cm: HexBinary,
    pub commitment: HexBinary,
    pub memo_cm: HexBinary,
    pub ovk_tag: HexBinary,
    pub reclaim_vk: HexBinary,
    pub finalize_vk: HexBinary,
    pub bid_id: Option<u64>,
    pub dkg_transcript: Option<HexBinary>,
    pub release: Option<ShieldedRelease>,
}

pub fn query_digest(deps: Deps, env: Env, q: SigningDigestQuery) -> StdResult<HexBinary> {
    let contract = env.contract.address.to_string();
    let d = DigestInput {
        chain_id: &env.block.chain_id,
        contract: &contract,
        auction_id: q.auction_id,
        bidder: &q.bidder,
        escrow_cm: q.escrow_cm.as_slice(),
        commitment: q.commitment.as_slice(),
        memo_cm: q.memo_cm.as_slice(),
        ovk_tag: q.ovk_tag.as_slice(),
        reclaim_vk: q.reclaim_vk.as_slice(),
        finalize_vk: q.finalize_vk.as_slice(),
        bid_id: q.bid_id,
        dkg_transcript: q.dkg_transcript.as_ref().map(|t| t.as_slice()),
        release: q.release.as_ref().map(|r| release_fields(deps.api, r)),
    };
    Ok(HexBinary::from(digest(deps.api, &q.action, &d).to_vec()))
}

pub fn policy() -> PrivatePolicyResponse {
    let s = |d: &[u8]| String::from_utf8_lossy(d).into_owned();
    PrivatePolicyResponse {
        bidder_shares: BIDDER_SHARES,
        platform_shares: PLATFORM_SHARES,
        seller_shares: SELLER_SHARES,
        total_shares: TOTAL_SHARES,
        reclaim_threshold: RECLAIM_THRESHOLD,
        finalize_threshold: FINALIZE_THRESHOLD,
        sign_domain: s(SIGN_DOMAIN),
        commit_domain: s(COMMIT_DOMAIN),
        notes_domain: s(NOTES_DOMAIN),
        ovk_domain: s(OVK_DOMAIN),
        memo_domain: s(MEMO_DOMAIN),
        payment_memo_prefix: PAYMENT_MEMO_PREFIX.into(),
    }
}
