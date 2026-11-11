//! `auctions-frost` (workstream A): sealed cw721-svg auctions on `svg-auction`, settled
//! in shielded ZEC, with a fresh FROST group per bid (bidder shares 1-3, platform 4-5,
//! seller 6; reclaim 5-of-6, finalize 6-of-6). Spec:
//! `~/grok-bot/privacy-products/auctions/SPEC.md`.
//!
//! The Zcash leg reuses the shielded pieces already in the tree: payment outputs are
//! terp-seams SEAM-NOTE-OUT notes (`SeamNoteOutV0`, XChaCha20-Poly1305 envelopes under a
//! one-time OVK), and each escrow spend is a frost_escrow Object A digest + Object B
//! `ZcashSpendPackageV0`, FROST-signed by the bid's reclaim group and checked with
//! `verify_escrow_release_encoded` / `verify_spend_encoded`. Broadcasting to Zcash is out
//! of scope here; Terp only ever sees commitments.
//!
//! Seller view and atomicity (SPEC "seller view", ATOMICITY.md): every bid is labelled
//! by a contract-checked memo commitment and seals its OVK + opening to the seller's view
//! key ([`super::bid_view`]); the seller opens all bids and finalizes the highest, and the
//! NFT moves only once the payment client attests the seller payment on Zcash.
//!
//! [`private_auction_flow`] is generic over `CwEnv` and needs no waits, so it runs the
//! same on `MockBech32` and on an ict-rs `Daemon`. Roles: platform = the chain's default
//! sender (deploys, relays), seller = operator 0, bidders = operators 1 and 2.
use super::bid_view::{BidOpening, PaymentClient, SellerPayment, ViewKey, open_own, seal};
use super::frost_util::Group;
use super::{ChainSlot, Outcome, ProductModule, ProductReport, SuiteCtx, Workstream};
use anyhow::{Context, Result, anyhow, ensure};
use cosmwasm_std::{Addr, Api, Binary, HexBinary, Uint128, Uint256, testing::MockApi, to_json_binary};
use cw_orch::prelude::*;
use cw_svg::{MintMsg, VariableDef, VariableKind};
use cw721_svg::{
    Cw721SvgContractSuite, ExecuteMsg as SvgExec, InstantiateMsg as SvgInit, QueryMsg as SvgQuery,
    SvgCollectionMetadata, SvgExecuteMsgExt,
};
use frost_escrow::{
    SpendParts, ZcashSpendPackageV0, encode_frost_public, escrow_release_digest,
    spend_package_from_parts, spend_sighash, verify_escrow_release_encoded, verify_spend_encoded,
};
use rand08::{RngCore, rngs::OsRng};
use svg_auction::private::{
    DigestInput, commitment_preimage, memo_preimage, notes_preimage, ovk_tag_preimage,
    signing_preimage,
};
use svg_auction::{
    BidStatus, CreatePrivateAuction, Cw721HookMsg, ExecuteMsg, InclusionStatus, InstantiateMsg,
    PlacePrivateBid,
    PrivateAction, PrivateAuction, PrivateBid, PrivateStatus, QueryMsg, ShieldedRelease,
    SvgAuction,
};
use terp_seams::dex::abstract_leaf_cm;
use terp_seams::dex::note::{
    CM_ABSTRACT_LEAF_V0, NF_POOL_SPEND, NoteEnvelope, SeamNoteOutV0, decrypt_note_out,
    encrypt_note_out,
};

pub const B: &[u16] = &[1, 2, 3];
pub const P: &[u16] = &[4, 5];
pub const S: &[u16] = &[6];
const TEMPLATE: &str = "<svg><circle fill='${a}' /><path fill='${b}' /></svg>";
/// Shielded escrow per bid (zatoshi). Hidden on Terp; only `escrow_cm` is public.
pub const ESCROW_ZAT: u64 = 5_000_000;
/// Zcash network fee carried by each release transaction (ZIP-317 floor for 2 actions).
pub const ZEC_FEE_ZAT: u64 = 10_000;
/// Platform fee taken in ZEC from the winning amount.
pub const FEE_BPS: u64 = 250;

/// Legacy-fixture style 32-byte asset id for shielded ZEC (cf. `asset_id_b`).
pub fn zec_asset_id() -> [u8; 32] {
    let mut a = [0u8; 32];
    a[..3].copy_from_slice(b"ZEC");
    a
}

fn rand32() -> [u8; 32] {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b
}

/// Stand-in for a party's Zcash shielded recipient (32-byte owner binding).
pub fn zcash_recipient(api: &MockApi, who: &Addr) -> [u8; 32] {
    api.blake2b_256(&[b"terp/test/zcash-recipient/".as_slice(), who.as_bytes()].concat())
}

/// The bid's shielded ZEC escrow note, owned by the reclaim group key.
pub struct Escrow {
    pub value_zat: u64,
    pub cm: [u8; 32],
    pub nf: [u8; 32],
    pub addr: String,
}

impl Escrow {
    pub fn fund(reclaim: &Group, value_zat: u64) -> Result<Self> {
        let owner: [u8; 32] = reclaim.vk()?.as_slice().try_into()?;
        let rcm = rand32();
        let cm = abstract_leaf_cm(&zec_asset_id(), value_zat, &owner, &rcm);
        Ok(Self {
            value_zat,
            cm,
            nf: terp_seams::dex::synthetic_pool_spend_nf(&cm, &rcm),
            addr: format!("zfrost:{}", HexBinary::from(owner.to_vec()).to_hex()),
        })
    }
}

/// One escrow spend (refund or settlement): the payment notes, their FROST-signed
/// frost_escrow spend authorisations, the bid OVK and the Terp record.
pub struct Release {
    pub notes: Vec<SeamNoteOutV0>,
    pub spends: Vec<(ZcashSpendPackageV0, Vec<u8>)>,
    pub ovk: [u8; 32],
    pub record: ShieldedRelease,
}

/// Build and FROST-authorise a release of `escrow` to `outputs` (label, recipient,
/// value), encrypted under the bid's `ovk`. `signers` must reach the reclaim threshold
/// (5 of 6 shares).
pub fn release(
    api: &MockApi,
    escrow: &Escrow,
    reclaim: &Group,
    signers: &[u16],
    outputs: &[(&str, [u8; 32], u64)],
    ovk: [u8; 32],
) -> Result<Release> {
    let total: u64 = outputs.iter().map(|o| o.2).sum();
    ensure!(
        total + ZEC_FEE_ZAT == escrow.value_zat,
        "release must spend the whole escrow"
    );
    let asset = zec_asset_id();
    let committee_pk = encode_frost_public(5, 6, reclaim.vk()?.as_slice());
    let (mut notes, mut spends) = (vec![], vec![]);
    for (i, (label, owner, value)) in outputs.iter().enumerate() {
        let rcm = rand32();
        let cm = abstract_leaf_cm(&asset, *value, owner, &rcm);
        let mut memo = [0u8; 64];
        memo[..label.len()].copy_from_slice(label.as_bytes());
        notes.push(SeamNoteOutV0 {
            asset_id: asset,
            value: *value,
            owner_binding: *owner,
            cm_public: cm,
            cm_encoding: CM_ABSTRACT_LEAF_V0,
            nullifier_lineage: escrow.nf,
            nullifier_domain: NF_POOL_SPEND,
            rcm,
            rcm_flag: 1,
            memo,
            memo_flag: 1,
            ..Default::default()
        });
        // Object A (release policy digest) then Object B (Zcash spend package).
        let a_sig = reclaim.sign(&escrow_release_digest(&escrow.nf, &cm, *value, &asset), signers)?;
        verify_escrow_release_encoded(&committee_pk, &escrow.nf, &cm, *value, &asset, a_sig.as_slice())?;
        let pkg = spend_package_from_parts(SpendParts {
            escrow_addr: escrow.addr.clone(),
            dest_display: label.to_string(),
            dest_commitment: cm,
            amount_zat: *value,
            fee_zat: if i == 0 { ZEC_FEE_ZAT } else { 0 },
            burn_nullifier: escrow.nf,
            asset_id: asset,
            object_a_sig_hex: a_sig.to_hex(),
        });
        let b_sig = reclaim.sign(&spend_sighash(&pkg), signers)?.to_vec();
        verify_spend_encoded(&committee_pk, &pkg, &b_sig)?;
        pkg.validate_matches_burn(&escrow.nf, &cm, *value, &asset)?;
        spends.push((pkg, b_sig));
    }
    let envelopes = notes
        .iter()
        .map(|n| encrypt_note_out(n, &ovk).map_err(|e| anyhow!("encrypt note: {e:?}")))
        .collect::<Result<Vec<NoteEnvelope>>>()?;
    let bytes: Vec<[u8; 382]> = notes.iter().map(|n| n.to_bytes()).collect();
    let record = ShieldedRelease {
        notes_cm: HexBinary::from(api.blake2b_256(&notes_preimage(&bytes)).to_vec()),
        ovk_ciphertext: Binary::from(serde_json::to_vec(&envelopes)?),
        ovk_tag: HexBinary::from(api.blake2b_256(&ovk_tag_preimage(&ovk)).to_vec()),
        payment_cm: None,
    };
    Ok(Release {
        notes,
        spends,
        ovk,
        record,
    })
}

/// What anyone can do with a disclosed OVK: decrypt the payment notes and check them
/// against the on-chain `notes_cm`.
pub fn open_release(api: &MockApi, r: &ShieldedRelease, ovk: &[u8]) -> Result<Vec<SeamNoteOutV0>> {
    let key: [u8; 32] = ovk.try_into()?;
    ensure!(
        api.blake2b_256(&ovk_tag_preimage(&key)) == r.ovk_tag.as_slice(),
        "ovk does not match ovk_tag"
    );
    let envelopes: Vec<NoteEnvelope> = serde_json::from_slice(r.ovk_ciphertext.as_slice())?;
    let notes = envelopes
        .iter()
        .map(|e| decrypt_note_out(e, &key).map_err(|e| anyhow!("decrypt note: {e:?}")))
        .collect::<Result<Vec<_>>>()?;
    let bytes: Vec<[u8; 382]> = notes.iter().map(|n| n.to_bytes()).collect();
    ensure!(
        api.blake2b_256(&notes_preimage(&bytes)) == r.notes_cm.as_slice(),
        "notes do not match notes_cm"
    );
    Ok(notes)
}

/// A signer on `Chain` plus its address.
pub struct Party<S> {
    pub sender: S,
    pub addr: Addr,
}

/// A sealed bid as the bidder holds it; the opening never leaves bidder and seller.
pub struct SealedBid {
    pub bidder: Addr,
    pub amount_zat: u64,
    pub salt: [u8; 32],
    pub commitment: HexBinary,
    /// Bid label checked by the contract: `blake2b(memo_preimage(.., bid_index))`.
    pub memo_cm: [u8; 32],
    /// The bid's OVK, sealed to the seller; every release of the escrow uses it.
    pub ovk: [u8; 32],
    pub escrow: Escrow,
    pub reclaim: Group,
    pub fin: Group,
    /// Refund to the bidder, pre-signed by {B,P} at bid time: the bidder's solo exit.
    pub refund: Release,
    pub bid_id: u64,
}

/// Deployed contracts for one run.
pub struct AuctionDeployment<Chain: CwEnv> {
    pub svg: Cw721SvgContractSuite<Chain>,
    pub auction: SvgAuction<Chain>,
}

/// Upload + instantiate the collection (as the seller) and the auction contract (as the
/// platform, which is the chain's default sender). `denom` only serves the contract's
/// public (non-private) auctions; private settlement never touches Terp funds.
pub fn deploy<Chain: CwEnv>(
    chain: &Chain,
    denom: &str,
    seller: &Party<Chain::Sender>,
) -> Result<AuctionDeployment<Chain>> {
    let svg = Cw721SvgContractSuite::new(chain.clone());
    svg.upload().context("upload cw721-svg")?;
    let vars = vec![
        VariableDef {
            name: "a".into(),
            kind: VariableKind::Options(vec!["rgb(1,2,3)".into(), "rgb(4,5,6)".into()]),
        },
        VariableDef {
            name: "b".into(),
            kind: VariableKind::Options(vec!["rgb(7,8,9)".into(), "rgb(9,9,9)".into()]),
        },
    ];
    svg.call_as(&seller.sender)
        .instantiate(
            &SvgInit {
                name: "Private Orbs".into(),
                symbol: "PORB".into(),
                collection_info_extension: SvgCollectionMetadata {
                    total: 10,
                    seed: Binary::from(b"privacy-suite-auction-seed-0000"),
                    mint_start_time: None,
                    mint_end_time: None,
                    price_tiers: vec![],
                    whitelist: None,
                    require_auth: false,
                    packed: cw721_svg::pack_svg(TEMPLATE, &vars),
                },
                minter: None,
                creator: Some(seller.addr.to_string()),
                withdraw_address: Some(seller.addr.to_string()),
            },
            Some(&seller.addr),
            &[],
        )
        .context("instantiate cw721-svg")?;
    let auction = SvgAuction::new(chain.clone());
    auction.upload().context("upload svg-auction")?;
    auction
        .instantiate(
            &InstantiateMsg {
                protocol_fee_bps: FEE_BPS as u16,
                min_increment_bps: 100,
                min_reserve_price: Uint128::new(1),
                duration: 3600,
                extension: 60,
                accepted_denom: vec![denom.into()],
                protocol_addr: chain.sender_addr().to_string(),
            },
            None,
            &[],
        )
        .context("instantiate svg-auction")?;
    Ok(AuctionDeployment { svg, auction })
}

fn bal<Chain: CwEnv>(chain: &Chain, a: &Addr, denom: &str) -> Result<Uint256> {
    Ok(chain
        .balance(a, Some(denom.into()))
        .map_err(Into::into)?
        .first()
        .map(|c| c.amount)
        .unwrap_or_default())
}

/// Mint, list privately, bid with shielded escrows, reclaim (B+P), refuse (P+S, B+P),
/// seller opens and ranks bids, pay + attest + finalize (B+P+S), disclose the payment
/// OVK, solo reclaim.
pub fn private_auction_flow<Chain: CwEnv>(
    chain: &Chain,
    d: &AuctionDeployment<Chain>,
    payments: &PaymentClient<Chain>,
    denom: &str,
    seller: &Party<Chain::Sender>,
    bidders: [&Party<Chain::Sender>; 2],
) -> Result<Vec<String>> {
    let api = MockApi::default(); // pure hashing only
    let platform = chain.sender_addr();
    let (svg, auction) = (&d.svg, &d.auction);
    let mut notes = vec![];
    // The seller's view key: bids seal their OVK and opening to its public half.
    let view = ViewKey::random();
    let view_pk = view.public();
    // Seller mints the next token and lists it as a sealed auction.
    let minted = svg
        .call_as(&seller.sender)
        .execute(
            &SvgExec::UpdateExtension {
                msg: SvgExecuteMsgExt::Mint(MintMsg {
                    amnt: 1,
                    proof_hashes: vec![],
                    alloc: 0,
                    ext: None,
                }),
            },
            &[],
        )
        .context("mint")?;
    let token_id = minted
        .event_attr_value("wasm", "token_id")
        .unwrap_or_else(|_| "0".into());
    let listed = svg
        .call_as(&seller.sender)
        .execute(
            &SvgExec::SendNft {
                contract: auction.address()?.to_string(),
                token_id: token_id.clone(),
                msg: to_json_binary(&Cw721HookMsg::CreatePrivateAuction(CreatePrivateAuction {
                    reclaim_grace: Some(600),
                    seller_view_pk: HexBinary::from(view_pk.to_vec()),
                    payment_client: payments.addr()?.to_string(),
                }))
                .map_err(|e| anyhow!("{e}"))?,
            },
            &[],
        )
        .context("list private auction")?;
    let auction_id: u64 = listed
        .event_attr_value("wasm", "auction_id")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let contract = auction.address()?.to_string();
    let chain_id = chain.block_info().map_err(Into::into)?.chain_id;
    let digest = |action: PrivateAction,
                  b: &SealedBid,
                  r: Option<&ShieldedRelease>|
     -> Result<[u8; 32]> {
        Ok(api.blake2b_256(&signing_preimage(
            &action,
            &DigestInput {
                chain_id: &chain_id,
                contract: &contract,
                auction_id,
                bidder: b.bidder.as_str(),
                escrow_cm: &b.escrow.cm,
                commitment: b.commitment.as_slice(),
                memo_cm: &b.memo_cm,
                ovk_tag: &api.blake2b_256(&ovk_tag_preimage(&b.ovk)),
                reclaim_vk: b.reclaim.vk()?.as_slice(),
                finalize_vk: b.fin.vk()?.as_slice(),
                bid_id: r.map(|_| b.bid_id),
                dkg_transcript: None,
                release: r.map(|r| {
                    (
                        r.notes_cm.as_slice(),
                        api.blake2b_256(r.ovk_ciphertext.as_slice()),
                        r.ovk_tag.as_slice(),
                        r.payment_cm.as_ref().map_or(&[][..], |c| c.as_slice()),
                    )
                }),
            },
        )))
    };
    let bp: Vec<u16> = [B, P].concat();
    let ps: Vec<u16> = [P, S].concat();
    let all: Vec<u16> = [B, P, S].concat();
    let bids = || -> Result<Vec<PrivateBid>> {
        Ok(auction.query(&QueryMsg::PrivateBids {
            auction_id,
            start_after: None,
            limit: Some(30),
        })?)
    };
    let bid = |party: &Party<Chain::Sender>, amount_zat: u64, salt: u8| -> Result<SealedBid> {
        let salt = [salt; 32];
        let reclaim = Group::dkg(6, 5)?;
        let escrow = Escrow::fund(&reclaim, ESCROW_ZAT)?;
        let commitment = HexBinary::from(
            api.blake2b_256(&commitment_preimage(
                auction_id,
                party.addr.as_str(),
                &escrow.cm,
                amount_zat,
                &salt,
            ))
            .to_vec(),
        );
        let ovk = rand32();
        let index = auction
            .query::<PrivateAuction>(&QueryMsg::PrivateAuction { auction_id })?
            .bid_count
            + 1;
        let memo_cm = api.blake2b_256(&memo_preimage(&chain_id, &contract, auction_id, index));
        let me = zcash_recipient(&api, &party.addr);
        let refund = release(
            &api,
            &escrow,
            &reclaim,
            &bp,
            &[("bidder-refund", me, ESCROW_ZAT - ZEC_FEE_ZAT)],
            ovk,
        )?;
        let mut b = SealedBid {
            bidder: party.addr.clone(),
            amount_zat,
            salt,
            commitment,
            memo_cm,
            ovk,
            escrow,
            reclaim,
            fin: Group::dkg(6, 6)?,
            refund,
            bid_id: 0,
        };
        let rb = b.reclaim.sign(&digest(PrivateAction::BindReclaim, &b, None)?, &bp)?;
        let fb = b.fin.sign(&digest(PrivateAction::BindFinalize, &b, None)?, &all)?;
        auction.call_as(&party.sender).execute(
            &ExecuteMsg::PlacePrivateBid(PlacePrivateBid {
                auction_id,
                escrow_cm: HexBinary::from(b.escrow.cm.to_vec()),
                commitment: b.commitment.clone(),
                sealed_bid: Binary::from(seal(
                    &view_pk,
                    &memo_cm,
                    &BidOpening {
                        ovk,
                        amount_zat,
                        salt,
                    },
                )?),
                memo_cm: HexBinary::from(memo_cm.to_vec()),
                ovk_tag: HexBinary::from(api.blake2b_256(&ovk_tag_preimage(&ovk)).to_vec()),
                reclaim_vk: b.reclaim.vk()?,
                finalize_vk: b.fin.vk()?,
                reclaim_bind_sig: rb,
                finalize_bind_sig: fb,
                dkg_transcript: None,
            }),
            &[],
        )?;
        b.bid_id = bids()?
            .iter()
            .filter(|x| x.bidder == party.addr)
            .map(|x| x.bid_id)
            .max()
            .ok_or_else(|| anyhow!("bid not stored"))?;
        Ok(b)
    };
    let status = |id: u64| -> Result<(BidStatus, Option<ShieldedRelease>)> {
        let b = bids()?
            .into_iter()
            .find(|b| b.bid_id == id)
            .ok_or_else(|| anyhow!("bid {id}"))?;
        Ok((b.status, b.release))
    };

    // Bid #1 (A) and #2 (B): fresh DKGs among bidder, platform and seller, and a shielded
    // escrow each. The transactions carry no funds.
    let [a, bb] = bidders;
    let b1 = bid(a, 4_000_000, 1)?;
    let b2 = bid(bb, 4_600_000, 2)?;
    ensure!(
        bids()?.iter().all(|b| b.release.is_none()),
        "no payment data at bid time"
    );
    notes.push(format!(
        "2 sealed bids, {} zat shielded escrow each, no Terp funds attached",
        ESCROW_ZAT
    ));

    // Bidder A + platform pull bid #1 mid-auction; the platform relays the tx.
    let sig = b1.reclaim.sign(
        &digest(PrivateAction::Reclaim, &b1, Some(&b1.refund.record))?,
        &bp,
    )?;
    let before = bal(chain, &a.addr, denom)?;
    auction.execute(
        &ExecuteMsg::ReclaimPrivateBid {
            auction_id,
            bid_id: b1.bid_id,
            release: b1.refund.record.clone(),
            signature: Some(sig),
        },
        &[],
    )?;
    ensure!(
        status(b1.bid_id)? == (BidStatus::Reclaimed, Some(b1.refund.record.clone())),
        "B+P reclaim recorded"
    );
    ensure!(bal(chain, &a.addr, denom)? == before, "no Terp refund leg");
    notes.push("B+P reclaimed bid mid-auction (shielded refund recorded, no Terp funds)".into());

    // Bid #3: A re-enters lower.
    let b3 = bid(a, 4_300_000, 3)?;

    // Seller view: open every active bid with the view key, check each opening against
    // its on-chain commitment and OVK tag, rank, pick the highest. A third party's key
    // opens nothing; each bidder re-opens its own envelope from its OVK.
    let outsider = ViewKey::random();
    let mut ranked = vec![];
    for pb in bids()?.into_iter().filter(|b| b.status == BidStatus::Active) {
        let memo: [u8; 32] = pb.memo_cm.as_slice().try_into()?;
        let o = view.open(&memo, pb.sealed_bid.as_slice())?;
        let escrow_cm: [u8; 32] = pb.escrow_cm.as_slice().try_into()?;
        ensure!(
            api.blake2b_256(&commitment_preimage(auction_id, pb.bidder.as_str(), &escrow_cm, o.amount_zat, &o.salt))
                == pb.commitment.as_slice()
                && api.blake2b_256(&ovk_tag_preimage(&o.ovk)) == pb.ovk_tag.as_slice(),
            "bid {} opening does not match its commitments",
            pb.bid_id
        );
        ensure!(outsider.open(&memo, pb.sealed_bid.as_slice()).is_err(), "third party opened a bid");
        ranked.push((o.amount_zat, pb.bid_id));
    }
    ensure!(
        open_own(&b3.ovk, &view_pk, &b3.memo_cm, bids()?[2].sealed_bid.as_slice())?.amount_zat == b3.amount_zat,
        "bidder re-opens its own bid"
    );
    ranked.sort();
    ensure!(
        ranked.iter().map(|r| r.1).collect::<Vec<_>>() == vec![b3.bid_id, b2.bid_id],
        "seller sees both live bids, ranked"
    );
    ensure!(ranked.last().map(|r| r.1) == Some(b2.bid_id), "highest bid wins");
    notes.push(format!(
        "seller opened {} live bids with its view key (third party: none), highest {} zat is bid {}",
        ranked.len(),
        ranked[ranked.len() - 1].0,
        b2.bid_id
    ));

    // The winning payment: seller proceeds, platform fee and the bidder's change, all as
    // shielded notes spending the bid's escrow, under the OVK the seller already holds.
    let fee = b2.amount_zat * FEE_BPS / 10_000;
    let (seller_z, platform_z, winner_z) = (
        zcash_recipient(&api, &seller.addr),
        zcash_recipient(&api, &platform),
        zcash_recipient(&api, &bb.addr),
    );
    let pay = release(
        &api,
        &b2.escrow,
        &b2.reclaim,
        &bp,
        &[
            ("seller", seller_z, b2.amount_zat - fee),
            ("platform-fee", platform_z, fee),
            ("bidder-change", winner_z, ESCROW_ZAT - b2.amount_zat - ZEC_FEE_ZAT),
        ],
        b2.ovk,
    )?;
    let mut pay = pay;
    pay.record.payment_cm = Some(HexBinary::from(pay.notes[0].cm_public.to_vec()));
    let fdig = digest(PrivateAction::Finalize, &b2, Some(&pay.record))?;

    // Platform + seller can never produce either signature; B+P cannot finalize.
    let rdig = digest(PrivateAction::Reclaim, &b2, Some(&b2.refund.record))?;
    ensure!(b2.reclaim.sign(&rdig, &ps).is_err(), "P+S reclaim must fail");
    ensure!(b2.fin.sign(&fdig, &ps).is_err(), "P+S finalize must fail");
    ensure!(b2.fin.sign(&fdig, &bp).is_err(), "B+P finalize must fail");
    ensure!(
        b2.reclaim.sign(&spend_sighash(&pay.spends[0].0), &ps).is_err(),
        "P+S cannot authorise the Zcash spend"
    );
    notes.push("P+S cannot reclaim/finalize/spend escrow; B+P cannot finalize".into());

    // Exchange order (SPEC): all three sign Finalize over the payment record, the {B,P}
    // signed Zcash payment is broadcast (here: its spend authorisations are verified),
    // then the platform submits Finalize. The seller cannot cancel while bids are live.
    let fsig = b2.fin.sign(&fdig, &all)?;
    ensure!(
        pay.spends.len() == 3 && pay.spends.iter().all(|(p, _)| p.burn_nullifier == b2.escrow.nf),
        "payment spends the winning escrow"
    );
    ensure!(
        auction
            .call_as(&seller.sender)
            .execute(&ExecuteMsg::CancelPrivateAuction { auction_id }, &[])
            .is_err(),
        "seller cannot cancel under live bids"
    );
    // Atomicity (ATOMICITY.md): the NFT moves only against a Confirmed zap-1 inclusion
    // of the seller payment, from this bid's escrow group, naming this bid's memo.
    let seller_payment = SellerPayment {
        note_cm: pay.notes[0].cm_public,
        amount_zat: pay.notes[0].value,
        frost_group: b2.reclaim.vk()?.as_slice().try_into()?,
        memo_cm: b2.memo_cm,
    };
    let fin = |payment_ref: HexBinary| ExecuteMsg::FinalizePrivateAuction {
        auction_id,
        bid_id: b2.bid_id,
        release: pay.record.clone(),
        signature: fsig.clone(),
        payment_ref,
    };
    let payment_ref = HexBinary::from(seller_payment.app_data_hash().to_vec());
    ensure!(auction.execute(&fin(payment_ref.clone()), &[]).is_err(), "finalize before payment");
    payments.attest(&seller_payment, InclusionStatus::Preconfirmed)?;
    ensure!(auction.execute(&fin(payment_ref.clone()), &[]).is_err(), "finalize on a preconfirmed payment");
    let payment_ref = payments.attest(&seller_payment, InclusionStatus::Confirmed)?;
    let (s0, w0) = (bal(chain, &seller.addr, denom)?, bal(chain, &bb.addr, denom)?);
    auction.execute(&fin(payment_ref), &[])?;
    ensure!(
        bal(chain, &seller.addr, denom)? == s0 && bal(chain, &bb.addr, denom)? == w0,
        "no Terp payment leg"
    );
    let owner: cw721::msg::OwnerOfResponse = svg.query(&SvgQuery::OwnerOf {
        token_id: token_id.clone(),
        include_expired: None,
    })?;
    ensure!(owner.owner == bb.addr.to_string(), "NFT to winner");
    let st: PrivateAuction = auction.query(&QueryMsg::PrivateAuction { auction_id })?;
    ensure!(st.status == PrivateStatus::Finalized, "finalized");
    let (_, rec) = status(b2.bid_id)?;
    let rec = rec.ok_or_else(|| anyhow!("payment record"))?;
    let public = format!("{:?}", bids()?);
    ensure!(
        !public.contains(&b2.amount_zat.to_string()) && !public.contains(&HexBinary::from(seller_z.to_vec()).to_hex()),
        "amount and destination stay private"
    );
    notes.push(format!(
        "B+P+S finalized auction {auction_id} after a Confirmed zap-1 seller payment (refused before / preconfirmed): NFT moved, payment only as notes_cm"
    ));

    // Optional disclosure: the platform publishes the payment OVK; anyone can then open
    // the notes and check seller proceeds and fee against the on-chain commitment.
    auction.execute(
        &ExecuteMsg::DisclosePrivateRelease {
            auction_id,
            bid_id: b2.bid_id,
            ovk: HexBinary::from(b2.ovk.to_vec()),
        },
        &[],
    )?;
    let ovk = bids()?
        .into_iter()
        .find(|b| b.bid_id == b2.bid_id)
        .and_then(|b| b.disclosed_ovk)
        .ok_or_else(|| anyhow!("ovk not stored"))?;
    let opened = open_release(&api, &rec, ovk.as_slice())?;
    ensure!(
        opened[0].owner_binding == seller_z
            && opened[0].value == b2.amount_zat - fee
            && opened[1].owner_binding == platform_z
            && opened[1].value == fee,
        "disclosed payment"
    );
    notes.push(format!(
        "OVK disclosed: seller {} zat, fee {fee} zat verified against notes_cm",
        b2.amount_zat - fee
    ));

    // Losing bid #3: bidder reclaims alone once the auction is closed, recording the
    // refund {B,P} pre-signed at bid time.
    auction.call_as(&a.sender).execute(
        &ExecuteMsg::ReclaimPrivateBid {
            auction_id,
            bid_id: b3.bid_id,
            release: b3.refund.record.clone(),
            signature: None,
        },
        &[],
    )?;
    ensure!(status(b3.bid_id)?.0 == BidStatus::Reclaimed, "solo reclaim");
    ensure!(
        b3.refund.spends[0].0.dest_commitment == b3.refund.notes[0].cm_public,
        "pre-signed refund pays the bidder"
    );
    notes.push("losing bidder reclaimed alone after close (pre-signed shielded refund)".into());
    Ok(notes)
}

/// `auctions-frost` slot.
pub struct PrivateAuctionModule<Chain: CwEnv> {
    pub denom: String,
    pub deployment: Option<AuctionDeployment<Chain>>,
    /// Payment client gating finalize: the mock on `MockBech32`; set B's `cw-lc-zcash`
    /// (`PaymentClient::External`) for a live chain.
    pub payments: Option<PaymentClient<Chain>>,
}

impl<Chain: CwEnv> Default for PrivateAuctionModule<Chain> {
    fn default() -> Self {
        Self {
            denom: "uterp".into(),
            deployment: None,
            payments: None,
        }
    }
}

fn party<Chain: CwEnv>(ctx: &SuiteCtx<Chain>, i: usize) -> Party<Chain::Sender> {
    Party {
        sender: ctx.operators[i].sender.clone(),
        addr: ctx.operators[i].addr.clone(),
    }
}

impl<Chain: CwEnv + 'static> ProductModule<Chain> for PrivateAuctionModule<Chain> {
    fn id(&self) -> &'static str {
        "auctions-frost"
    }
    fn workstream(&self) -> Workstream {
        Workstream::A
    }
    fn chains(&self) -> &'static [ChainSlot] {
        &[ChainSlot::Terp]
    }
    fn deploy(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<()> {
        ensure!(
            ctx.operators.len() >= 3,
            "auctions-frost needs ≥3 operators"
        );
        super::fund_if_mock(ctx, &self.denom, 100_000); // gas only on Mock
        self.deployment = Some(deploy(&ctx.terp, &self.denom, &party(ctx, 0))?);
        if self.payments.is_none() && super::is_mock(&ctx.terp) {
            self.payments = Some(PaymentClient::deploy_mock(&ctx.terp)?);
        }
        Ok(())
    }
    fn exercise(&mut self, ctx: &mut SuiteCtx<Chain>) -> Result<ProductReport> {
        let d = self
            .deployment
            .as_ref()
            .ok_or_else(|| anyhow!("not deployed"))?;
        let Some(payments) = self.payments.as_ref() else {
            return Ok(ProductReport {
                id: "auctions-frost",
                workstream: Workstream::A,
                outcome: Outcome::Skipped(
                    "no Zcash payment client on this chain (set B's cw-lc-zcash: PaymentClient::External)".into(),
                ),
                notes: vec![],
            });
        };
        let (s, a, b) = (party(ctx, 0), party(ctx, 1), party(ctx, 2));
        let notes = private_auction_flow(&ctx.terp, d, payments, &self.denom, &s, [&a, &b])?;
        Ok(ProductReport {
            id: "auctions-frost",
            workstream: Workstream::A,
            outcome: Outcome::Passed,
            notes,
        })
    }
}
