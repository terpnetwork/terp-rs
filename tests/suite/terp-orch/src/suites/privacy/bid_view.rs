//! Seller view of sealed bids and the Zcash payment gate for `auctions-frost`.
//! Spec: `~/grok-bot/privacy-products/auctions/{SPEC,ATOMICITY}.md`.
//!
//! **Seller view.** Each bid carries `sealed_bid = epk || nonce || XChaCha20-Poly1305(key,
//! ovk || amount_zat || salt, aad = memo_cm)` with `key = blake2b(VIEW_KDF || X25519(esk,
//! seller_view_pk) || epk || seller_view_pk)`. `esk = blake2b(VIEW_ESK || ovk || memo_cm)`,
//! so the bidder re-opens its own envelope from its OVK alone, and the seller with its
//! view key; nobody else can. `memo_cm` is the contract-checked label of the auction
//! instance (contract, lot, bid index), so an envelope cannot be replayed into another.
//!
//! **Payment gate.** Finalize needs a Confirmed zap-1 inclusion at the auction's payment
//! client. [`MockZcashLc`] stands in for workstream B's `cw-lc-zcash` offline (same
//! `PaymentInclusion` query, inclusions recorded by its instantiator in place of the
//! k-of-n header committee). On a live chain pass B's client ([`PaymentClient::External`]).
use anyhow::{Result, anyhow, ensure};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use cosmwasm_std::{
    Api, Binary, Deps, DepsMut, Empty, Env, HexBinary, MessageInfo, Response, StdError, StdResult,
    testing::MockApi, to_json_binary,
};
use curve25519_dalek::montgomery::MontgomeryPoint;
use cw_orch::prelude::*;
use rand08::{RngCore, rngs::OsRng};
use svg_auction::private::payment_leaf;
use svg_auction::{InclusionStatus, PaymentClientQuery, PaymentInclusion, PaymentInclusionResponse};

const VIEW_ESK: &[u8] = b"terp/svg-auction/view-esk/v1";
const VIEW_KDF: &[u8] = b"terp/svg-auction/view-kdf/v1";

fn h(parts: &[&[u8]]) -> [u8; 32] {
    MockApi::default().blake2b_256(&parts.concat())
}

/// What the seller learns from a bid: its OVK and the opening of its commitment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidOpening {
    pub ovk: [u8; 32],
    pub amount_zat: u64,
    pub salt: [u8; 32],
}

impl BidOpening {
    fn to_bytes(&self) -> Vec<u8> {
        [&self.ovk[..], &self.amount_zat.to_be_bytes(), &self.salt].concat()
    }
    fn from_bytes(b: &[u8]) -> Result<Self> {
        ensure!(b.len() == 72, "bid opening must be 72 bytes");
        Ok(Self {
            ovk: b[..32].try_into()?,
            amount_zat: u64::from_be_bytes(b[32..40].try_into()?),
            salt: b[40..].try_into()?,
        })
    }
}

/// The seller's X25519 view key (registered as `seller_view_pk`).
pub struct ViewKey([u8; 32]);

impl ViewKey {
    pub fn random() -> Self {
        let mut sk = [0u8; 32];
        OsRng.fill_bytes(&mut sk);
        Self(sk)
    }
    pub fn public(&self) -> [u8; 32] {
        MontgomeryPoint::mul_base_clamped(self.0).to_bytes()
    }
    /// Open a bid envelope addressed to this key.
    pub fn open(&self, memo_cm: &[u8; 32], envelope: &[u8]) -> Result<BidOpening> {
        ensure!(envelope.len() > 56, "short envelope");
        let epk = MontgomeryPoint(envelope[..32].try_into()?);
        let shared = epk.mul_clamped(self.0).to_bytes();
        decrypt(&kdf(&shared, &epk.to_bytes(), &self.public()), memo_cm, envelope)
    }
}

fn kdf(shared: &[u8; 32], epk: &[u8; 32], pk: &[u8; 32]) -> [u8; 32] {
    h(&[VIEW_KDF, shared, epk, pk])
}

fn esk(ovk: &[u8; 32], memo_cm: &[u8; 32]) -> [u8; 32] {
    h(&[VIEW_ESK, ovk, memo_cm])
}

fn decrypt(key: &[u8; 32], memo_cm: &[u8; 32], envelope: &[u8]) -> Result<BidOpening> {
    let pt = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&envelope[32..56]),
            Payload {
                msg: &envelope[56..],
                aad: memo_cm,
            },
        )
        .map_err(|_| anyhow!("envelope does not open with this key"))?;
    BidOpening::from_bytes(&pt)
}

/// Bidder side: seal `o` to `seller_pk` under the bid label `memo_cm`.
pub fn seal(seller_pk: &[u8; 32], memo_cm: &[u8; 32], o: &BidOpening) -> Result<Vec<u8>> {
    let esk = esk(&o.ovk, memo_cm);
    let epk = MontgomeryPoint::mul_base_clamped(esk).to_bytes();
    let shared = MontgomeryPoint(*seller_pk).mul_clamped(esk).to_bytes();
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ct = XChaCha20Poly1305::new((&kdf(&shared, &epk, seller_pk)).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &o.to_bytes(),
                aad: memo_cm,
            },
        )
        .map_err(|_| anyhow!("seal"))?;
    Ok([&epk[..], &nonce, &ct].concat())
}

/// Bidder side: re-open its own envelope from its OVK (no extra secret kept).
pub fn open_own(ovk: &[u8; 32], seller_pk: &[u8; 32], memo_cm: &[u8; 32], envelope: &[u8]) -> Result<BidOpening> {
    let esk = esk(ovk, memo_cm);
    ensure!(
        envelope.len() > 56 && envelope[..32] == MontgomeryPoint::mul_base_clamped(esk).to_bytes(),
        "envelope not sealed with this ovk"
    );
    let shared = MontgomeryPoint(*seller_pk).mul_clamped(esk).to_bytes();
    decrypt(&kdf(&shared, envelope[..32].try_into()?, seller_pk), memo_cm, envelope)
}

// --- payment client -----------------------------------------------------------------------

#[cosmwasm_schema::cw_serde]
pub struct MockLcInit {}

#[cosmwasm_schema::cw_serde]
pub enum MockLcExec {
    /// Record (or replace) an inclusion. Instantiator only: stands in for the header committee.
    Record(PaymentInclusionResponse),
}

fn lc_instantiate(deps: DepsMut, _: Env, info: MessageInfo, _: MockLcInit) -> StdResult<Response> {
    deps.storage.set(b"attestor", info.sender.as_bytes());
    Ok(Response::new())
}

fn lc_execute(deps: DepsMut, _: Env, info: MessageInfo, m: MockLcExec) -> StdResult<Response> {
    if deps.storage.get(b"attestor").as_deref() != Some(info.sender.as_bytes()) {
        return Err(StdError::msg("only the attestor records inclusions"));
    }
    let MockLcExec::Record(r) = m;
    let key = r.inclusion.app_data_hash.to_ascii_lowercase();
    deps.storage.set(key.as_bytes(), &cosmwasm_std::to_json_vec(&r)?);
    Ok(Response::new().add_attribute("app_data_hash", key))
}

fn lc_query(deps: Deps, _: Env, q: PaymentClientQuery) -> StdResult<Binary> {
    let PaymentClientQuery::PaymentInclusion { app_data_hash_hex } = q;
    let r: Option<PaymentInclusionResponse> = deps
        .storage
        .get(app_data_hash_hex.to_ascii_lowercase().as_bytes())
        .map(|v| cosmwasm_std::from_json(&v))
        .transpose()?;
    to_json_binary(&r)
}

/// Offline stand-in for `cw-lc-zcash` (same `PaymentInclusion` wire format).
#[cw_orch::interface(MockLcInit, MockLcExec, PaymentClientQuery, Empty, id = "mock_zcash_lc")]
pub struct MockZcashLc;

impl<Chain> Uploadable for MockZcashLc<Chain> {
    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new_with_empty(lc_execute, lc_instantiate, lc_query))
    }
}

/// Where finalize looks for the seller payment.
pub enum PaymentClient<Chain: CwEnv> {
    Mock(MockZcashLc<Chain>),
    /// B's `cw-lc-zcash` on a live chain.
    External(Addr),
}

/// A seller payment as the zap-1 attestation names it.
pub struct SellerPayment {
    pub note_cm: [u8; 32],
    pub amount_zat: u64,
    /// The bid's escrow key (`reclaim_vk`), which spent the escrow.
    pub frost_group: [u8; 32],
    pub memo_cm: [u8; 32],
}

impl SellerPayment {
    pub fn leaf(&self) -> [u8; 32] {
        payment_leaf(&self.memo_cm)
    }
    /// zap-1 `app_data_hash` (cw-ics08-wasm-zap1), the key `payment_ref` names.
    pub fn app_data_hash(&self) -> [u8; 32] {
        cw_ics08_wasm_zap1::app_data_hash(&self.note_cm, self.amount_zat, &self.frost_group, &self.leaf())
    }
}

impl<Chain: CwEnv> PaymentClient<Chain> {
    pub fn deploy_mock(chain: &Chain) -> Result<Self> {
        let lc = MockZcashLc::new(chain.clone());
        lc.upload()?;
        lc.instantiate(&MockLcInit {}, None, &[])?;
        Ok(Self::Mock(lc))
    }
    pub fn addr(&self) -> Result<Addr> {
        match self {
            Self::Mock(lc) => Ok(lc.address()?),
            Self::External(a) => Ok(a.clone()),
        }
    }
    /// Make the payment visible at `status`; returns `payment_ref` (the app_data_hash).
    pub fn attest(&self, p: &SellerPayment, status: InclusionStatus) -> Result<HexBinary> {
        let key = p.app_data_hash();
        let hex = |b: &[u8]| HexBinary::from(b).to_hex();
        match self {
            Self::Mock(lc) => {
                lc.execute(
                    &MockLcExec::Record(PaymentInclusionResponse {
                        inclusion: PaymentInclusion {
                            app_data_hash: hex(&key),
                            height: 1,
                            block_hash: String::new(),
                            payment_root: String::new(),
                            source_client: "mock-zap1".into(),
                            sequence: 1,
                            recorded_at: 0,
                            leaf: hex(&p.leaf()),
                            note: hex(&p.note_cm),
                            amount_zat: p.amount_zat,
                            frost_group: hex(&p.frost_group),
                        },
                        status,
                    }),
                    &[],
                )?;
                Ok(HexBinary::from(key.to_vec()))
            }
            // TODO(workstream B): relay the zap-1 witness (leaf, Merkle path, IBC v2 packet
            // fields) to cw-lc-zcash `VerifyPaymentInclusion` once its header committee runs
            // on the live harness; the auction then reads it through `PaymentInclusion`.
            Self::External(a) => Err(anyhow!(
                "live payment attestation to {a} needs B's zap-1 relayer (TODO)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bid(ovk: u8, amount_zat: u64) -> BidOpening {
        BidOpening {
            ovk: [ovk; 32],
            amount_zat,
            salt: [ovk ^ 0x55; 32],
        }
    }

    #[test]
    fn seller_opens_every_bid_and_ranks_them() {
        let seller = ViewKey::random();
        let pk = seller.public();
        let sealed: Vec<([u8; 32], BidOpening, Vec<u8>)> = [(1, 4_000_000), (2, 4_600_000), (3, 4_300_000)]
            .into_iter()
            .map(|(i, amt)| {
                let memo = [i; 32];
                let o = bid(i, amt);
                let env = seal(&pk, &memo, &o).unwrap();
                (memo, o, env)
            })
            .collect();
        let opened: Vec<BidOpening> = sealed.iter().map(|(m, _, e)| seller.open(m, e).unwrap()).collect();
        assert_eq!(opened, sealed.iter().map(|s| s.1.clone()).collect::<Vec<_>>());
        let best = opened.iter().max_by_key(|o| o.amount_zat).unwrap();
        assert_eq!(best.ovk, [2; 32]);
        // The bidder re-opens its own envelope from its OVK.
        let (m, o, e) = &sealed[1];
        assert_eq!(&open_own(&o.ovk, &pk, m, e).unwrap(), o);
    }

    /// The auction's local mirror and B's cw-lc-zcash types are the same wire format.
    #[test]
    fn payment_client_wire_matches_cw_lc_zcash() {
        use cw_lc_zcash::msg as lc;
        let q = cosmwasm_std::to_json_vec(&PaymentClientQuery::PaymentInclusion {
            app_data_hash_hex: "aa".into(),
        })
        .unwrap();
        let lc::QueryMsg::PaymentInclusion { app_data_hash_hex } = cosmwasm_std::from_json(&q).unwrap()
        else {
            panic!("query shape")
        };
        assert_eq!(app_data_hash_hex, "aa");
        for (status, mine) in [
            (lc::InclusionStatus::Confirmed, InclusionStatus::Confirmed),
            (lc::InclusionStatus::Preconfirmed, InclusionStatus::Preconfirmed),
            (lc::InclusionStatus::Reorged, InclusionStatus::Reorged),
        ] {
            let real = lc::PaymentInclusionResponse {
                inclusion: cw_lc_zcash::state::Inclusion {
                    app_data_hash: "aa".into(),
                    height: 7,
                    block_hash: "bb".into(),
                    payment_root: "cc".into(),
                    leaf: "dd".into(),
                    source_client: "07-zap1-0".into(),
                    sequence: 3,
                    note: "ee".into(),
                    amount_zat: 9,
                    frost_group: "ff".into(),
                    recorded_at: 11,
                },
                status,
            };
            let got: PaymentInclusionResponse =
                cosmwasm_std::from_json(cosmwasm_std::to_json_vec(&real).unwrap()).unwrap();
            assert_eq!((got.status, got.inclusion.note, got.inclusion.amount_zat), (mine, "ee".into(), 9));
        }
    }

    #[test]
    fn third_party_and_wrong_label_cannot_open() {
        let seller = ViewKey::random();
        let memo = [7u8; 32];
        let o = bid(9, 1_000);
        let env = seal(&seller.public(), &memo, &o).unwrap();
        assert!(ViewKey::random().open(&memo, &env).is_err(), "third party");
        assert!(open_own(&[8; 32], &seller.public(), &memo, &env).is_err(), "other ovk");
        // Same envelope presented under another bid's label: AEAD rejects it.
        assert!(seller.open(&[6u8; 32], &env).is_err(), "relabelled");
        let mut tampered = env.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(seller.open(&memo, &tampered).is_err(), "tampered");
    }
}
