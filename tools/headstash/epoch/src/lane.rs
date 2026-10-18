//! Ingress, per-item verify, and the in-memory batch.
//!
//! A reject does not enter the digest list. The contract lane can record
//! one; this route does not, so a fixed proof can be resubmitted.
//! Receipts are counted before `verify_claim`. A public `index` field is a
//! reject and is not a circuit input.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use ed25519_dalek::SigningKey;
use headstash_claim::statement::{
    canonical_claim_message, canonical_sign_message, decode_destination, destination_bech32,
    pack_words, signature_opens_leaf,
};
use headstash_claim::{
    ClaimProof, decode_claim_proof, key_com, public_key_com, tree, verify_claim,
};

use crate::roots::{
    ALGO_ACCEPTED, ALGO_PENDING, BATCH_ATTEST_HEIGHT, BATCH_ATTEST_TIMESTAMP, PayoutRow,
    SealedVote, accepted_root, attested_root, receipt_message, sha256, sign_custody,
    verify_ed25519,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Flock,
    Halo2,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IngressError {
    #[error("halo2 campaigns are not verified in this slice")]
    Backend,
    #[error("halo2 claim carries key material")]
    KeyMaterial,
    #[error("campaign parameters are not usable")]
    Params,
    #[error("proof length does not match the campaign")]
    Length,
    #[error("a public leaf index is not a claim field")]
    PublicIndex,
    #[error("proof bytes do not decode")]
    Decode,
    #[error("public words are not a claim")]
    Public,
    #[error("context does not match the campaign")]
    Context,
    #[error("allocation root does not match the campaign")]
    Root,
    #[error("denom does not match the campaign")]
    Denom,
    #[error("destination is not canonical bech32")]
    Destination,
    #[error("signed message is not the canonical body")]
    Message,
    #[error("signature does not open the leaf")]
    LeafKey,
    #[error("amount is not in the published set")]
    Amount,
    #[error("amount class is full")]
    Cap,
    #[error("nullifier is reserved for the public path")]
    Reserved,
    #[error("nullifier is already spent")]
    Spent,
    #[error("proof digest is already in the lane")]
    Digest,
    #[error("batch is full")]
    Full,
    #[error("proof did not verify")]
    Proof,
    #[error("trusted receipts are below the threshold")]
    Receipts,
    #[error("verifying key does not match the campaign")]
    Vk,
}

#[derive(Clone, Debug)]
pub struct CampaignParams {
    pub backend: Backend,
    pub ctx: [u8; 32],
    pub allocation_root: [u8; 32],
    pub bech32_hrp: String,
    pub denom: [u8; 32],
    pub proof_len: usize,
    pub cutoff_height: u64,
    /// Payout amount, in base units, to the number of claims allowed for it.
    pub amounts: BTreeMap<u64, u32>,
    pub trusted: Vec<[u8; 32]>,
    pub trusted_verify_threshold: usize,
    /// Stored for the creator. This slice does not enforce it: the keeper
    /// checks one OracleSource signature.
    pub trusted_sign_threshold: usize,
    pub max_rows: usize,
    pub reserved: Vec<[u8; 32]>,
    /// Halo2 registry id. Flock may leave this at 0.
    pub vk_id: u64,
    /// SHA-256 of the whole circuit blob. HTTP install checks it.
    /// The open campaign keeps `vk_id` only.
    pub vk_hash: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct PeerReceipt {
    pub signer: [u8; 32],
    pub signature: [u8; 64],
}

#[derive(Clone, Debug)]
pub struct SubmittedClaim {
    pub proof: Vec<u8>,
    pub message: Vec<u8>,
    pub signature: Vec<u8>,
    pub leaf_x: [u8; 32],
    pub denom: [u8; 32],
    /// Present when the caller put a leaf index on the submission.
    pub index: Option<u32>,
    pub receipts: Vec<PeerReceipt>,
    /// Halo2 registry id. Flock claims leave this at 0.
    pub vk_id: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BatchReport {
    pub accepted: Vec<usize>,
    pub rejected: Vec<(usize, IngressError)>,
}

struct Prepared {
    digest: [u8; 32],
    nullifier: [u8; 32],
    destination: String,
    amount: u64,
}

/// In-memory triple tree for one campaign. One campaign keeps one backend.
#[derive(Debug)]
pub struct Campaign<const D: usize> {
    backend: Backend,
    ctx: [u8; 32],
    allocation_root: [u8; 32],
    hrp: String,
    denom: [u8; 32],
    proof_len: usize,
    cutoff_height: u64,
    amounts: BTreeMap<u64, u32>,
    used: BTreeMap<u64, u32>,
    trusted: HashSet<[u8; 32]>,
    verify_threshold: usize,
    sign_threshold: usize,
    max_rows: usize,
    reserved: HashSet<[u8; 32]>,
    digests: Vec<[u8; 32]>,
    seen: HashSet<[u8; 32]>,
    rows: Vec<PayoutRow>,
    spent: HashSet<[u8; 32]>,
    verifies: AtomicU64,
    vk_id: u64,
}

impl<const D: usize> Campaign<D> {
    pub fn open(params: CampaignParams) -> Result<Self, IngressError> {
        if params.proof_len == 0 || params.max_rows == 0 || params.bech32_hrp.is_empty() {
            return Err(IngressError::Params);
        }
        destination_bech32(&params.bech32_hrp, &[0u32; 5]).map_err(|_| IngressError::Params)?;
        if params.trusted_verify_threshold == 0
            || params.trusted_verify_threshold > params.trusted.len()
        {
            return Err(IngressError::Params);
        }
        let mut trusted = HashSet::with_capacity(params.trusted.len());
        for pk in &params.trusted {
            if ed25519_dalek::VerifyingKey::from_bytes(pk).is_err() || !trusted.insert(*pk) {
                return Err(IngressError::Params);
            }
        }
        let mut reserved = HashSet::with_capacity(params.reserved.len());
        for nf in params.reserved {
            if !reserved.insert(nf) {
                return Err(IngressError::Params);
            }
        }
        if params.backend == Backend::Halo2 && params.vk_id == 0 {
            return Err(IngressError::Params);
        }
        Ok(Self {
            backend: params.backend,
            ctx: params.ctx,
            allocation_root: params.allocation_root,
            hrp: params.bech32_hrp,
            denom: params.denom,
            proof_len: params.proof_len,
            cutoff_height: params.cutoff_height,
            amounts: params.amounts,
            used: BTreeMap::new(),
            trusted,
            verify_threshold: params.trusted_verify_threshold,
            sign_threshold: params.trusted_sign_threshold,
            max_rows: params.max_rows,
            reserved,
            digests: Vec::new(),
            seen: HashSet::new(),
            rows: Vec::new(),
            spent: HashSet::new(),
            verifies: AtomicU64::new(0),
            vk_id: params.vk_id,
        })
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn trusted_sign_threshold(&self) -> usize {
        self.sign_threshold
    }

    pub fn verifies(&self) -> u64 {
        self.verifies.load(Ordering::Relaxed)
    }

    pub fn digests(&self) -> &[[u8; 32]] {
        &self.digests
    }

    pub fn rows(&self) -> &[PayoutRow] {
        &self.rows
    }

    pub fn pending_root(&self) -> [u8; 32] {
        crate::roots::pending_root(&self.digests)
    }

    pub fn accepted_root(&self) -> [u8; 32] {
        accepted_root(&self.allocation_root, &self.rows)
    }

    /// Insert items that pass. Report the others. A bad item does not roll
    /// back a good one, and it does not enter the tree.
    pub fn accept_batch(
        &mut self,
        items: &[SubmittedClaim],
        local: Option<&SigningKey>,
    ) -> BatchReport {
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        for (index, item) in items.iter().enumerate() {
            match self.accept_one(item, local) {
                Ok(()) => accepted.push(index),
                Err(err) => rejected.push((index, err)),
            }
        }
        BatchReport { accepted, rejected }
    }

    /// Lowest rejecting index. Does not insert.
    ///
    /// Length and a public index are decided before any proof check, so a
    /// bad-length sibling does not verify the good item.
    pub fn bisect_first_reject(&self, items: &[SubmittedClaim]) -> Option<usize> {
        if let Some(index) = items
            .iter()
            .position(|item| structural(self, item).is_err())
        {
            return Some(index);
        }
        let mut lo = 0usize;
        let mut hi = items.len();
        let mut found = None;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.evaluate(&items[mid], None).is_err() {
                found = Some(mid);
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        found
    }

    /// Below `cutoff_height` the algo is pending. At the cutoff it is accepted.
    /// The key is the OracleSource key. It is not stored on the campaign.
    pub fn seal(&self, height: u64, custody: &SigningKey) -> SealedVote {
        let (algo, batch_hash) = if height >= self.cutoff_height {
            (ALGO_ACCEPTED, self.accepted_root())
        } else {
            (ALGO_PENDING, self.pending_root())
        };
        SealedVote {
            algo: algo.to_string(),
            batch_hash,
            attested: attested_root(&batch_hash),
            custody_signature: sign_custody(custody, &batch_hash),
            custody_public: custody.verifying_key().to_bytes(),
            attestation_height: BATCH_ATTEST_HEIGHT,
            attestation_timestamp: BATCH_ATTEST_TIMESTAMP,
            foreign_height: height,
        }
    }

    fn accept_one(
        &mut self,
        item: &SubmittedClaim,
        local: Option<&SigningKey>,
    ) -> Result<(), IngressError> {
        let prepared = self.evaluate(item, local)?;
        self.seen.insert(prepared.digest);
        self.digests.push(prepared.digest);
        self.spent.insert(prepared.nullifier);
        *self.used.entry(prepared.amount).or_insert(0) += 1;
        self.rows.push(PayoutRow {
            nullifier: prepared.nullifier,
            denom: self.denom,
            destination: prepared.destination,
            amount: prepared.amount,
        });
        Ok(())
    }

    fn evaluate(
        &self,
        item: &SubmittedClaim,
        local: Option<&SigningKey>,
    ) -> Result<Prepared, IngressError> {
        structural(self, item)?;
        if self.backend == Backend::Halo2 {
            return self.evaluate_halo2(item, local);
        }
        let digest = sha256(&item.proof);
        if self.seen.contains(&digest) {
            return Err(IngressError::Digest);
        }
        let proved = decode_claim_proof(&item.proof).ok_or(IngressError::Decode)?;
        let claim =
            headstash_claim::claim_from_public(&proved.public).ok_or(IngressError::Public)?;
        if pack_words(&claim.ctx) != self.ctx {
            return Err(IngressError::Context);
        }
        if pack_words(&claim.root) != self.allocation_root {
            return Err(IngressError::Root);
        }
        if item.denom != self.denom {
            return Err(IngressError::Denom);
        }
        let destination = destination_bech32(&self.hrp, &claim.destination)
            .map_err(|_| IngressError::Destination)?;
        if destination.is_empty() || destination.len() > 128 {
            return Err(IngressError::Destination);
        }
        let expected =
            canonical_claim_message(&claim, &self.hrp).map_err(|_| IngressError::Message)?;
        if item.message != expected {
            return Err(IngressError::Message);
        }
        if !signature_opens_leaf(&item.message, &item.signature, &item.leaf_x) {
            return Err(IngressError::LeafKey);
        }
        if claim.amount == 0 {
            return Err(IngressError::Amount);
        }
        let cap = self
            .amounts
            .get(&claim.amount)
            .copied()
            .ok_or(IngressError::Amount)?;
        let used = self.used.get(&claim.amount).copied().unwrap_or(0);
        if used >= cap {
            return Err(IngressError::Cap);
        }
        let nullifier = pack_words(&claim.nullifier);
        if self.reserved.contains(&nullifier) {
            return Err(IngressError::Reserved);
        }
        if self.spent.contains(&nullifier) {
            return Err(IngressError::Spent);
        }
        if self.rows.len() >= self.max_rows {
            return Err(IngressError::Full);
        }
        let body = receipt_message(
            &self.ctx,
            &self.allocation_root,
            &nullifier,
            &destination,
            claim.amount,
            &digest,
        );
        if self.receipt_count(&body, item, local) < self.verify_threshold {
            return Err(IngressError::Receipts);
        }
        self.verify_proof(&proved, &item.leaf_x)?;
        Ok(Prepared {
            digest,
            nullifier,
            destination,
            amount: claim.amount,
        })
    }

    fn evaluate_halo2(
        &self,
        item: &SubmittedClaim,
        local: Option<&SigningKey>,
    ) -> Result<Prepared, IngressError> {
        if !item.signature.is_empty() || item.leaf_x != [0u8; 32] {
            return Err(IngressError::KeyMaterial);
        }
        let digest = sha256(&item.proof);
        if self.seen.contains(&digest) {
            return Err(IngressError::Digest);
        }
        let parsed = parse_canonical_message(&item.message).ok_or(IngressError::Message)?;
        if parsed.ctx != self.ctx {
            return Err(IngressError::Context);
        }
        if parsed.root != self.allocation_root {
            return Err(IngressError::Root);
        }
        if item.denom != self.denom {
            return Err(IngressError::Denom);
        }
        let raw = decode_destination(&self.hrp, &parsed.destination)
            .map_err(|_| IngressError::Destination)?;
        if parsed.destination.len() > 128 {
            return Err(IngressError::Destination);
        }
        let expected = canonical_sign_message(
            &parsed.ctx,
            &parsed.root,
            &parsed.nullifier,
            &parsed.destination,
            parsed.amount,
        );
        if item.message != expected {
            return Err(IngressError::Message);
        }
        if parsed.amount == 0 {
            return Err(IngressError::Amount);
        }
        let cap = self
            .amounts
            .get(&parsed.amount)
            .copied()
            .ok_or(IngressError::Amount)?;
        // A replay of a spent nullifier is spent even when the class is full.
        if self.spent.contains(&parsed.nullifier) {
            return Err(IngressError::Spent);
        }
        let used = self.used.get(&parsed.amount).copied().unwrap_or(0);
        if used >= cap {
            return Err(IngressError::Cap);
        }
        if self.reserved.contains(&parsed.nullifier) {
            return Err(IngressError::Reserved);
        }
        if self.rows.len() >= self.max_rows {
            return Err(IngressError::Full);
        }
        let body = receipt_message(
            &self.ctx,
            &self.allocation_root,
            &parsed.nullifier,
            &parsed.destination,
            parsed.amount,
            &digest,
        );
        if self.receipt_count(&body, item, local) < self.verify_threshold {
            return Err(IngressError::Receipts);
        }
        if item.vk_id != self.vk_id {
            return Err(IngressError::Vk);
        }
        let mut recipient = [0u8; 32];
        recipient[..20].copy_from_slice(&raw);
        self.verifies.fetch_add(1, Ordering::Relaxed);
        let statement = zk_headstash::circuit::EpochStatement {
            ctx: parsed.ctx,
            root: parsed.root,
            nullifier: parsed.nullifier,
            destination: parsed.destination.clone(),
            amount: parsed.amount,
            recipient,
        };
        let instances = zk_headstash::circuit::host_public_inputs(&statement).map_err(|err| {
            match err {
                zk_headstash::circuit::EpochVerifyError::Public => IngressError::Public,
                zk_headstash::circuit::EpochVerifyError::Proof => IngressError::Proof,
            }
        })?;
        crate::host::verify_with_host(item.vk_id, &item.proof, &instances)
            .map_err(|_| IngressError::Proof)?;
        Ok(Prepared {
            digest,
            nullifier: parsed.nullifier,
            destination: parsed.destination,
            amount: parsed.amount,
        })
    }

    fn verify_proof(&self, proved: &ClaimProof, leaf_x: &[u8; 32]) -> Result<(), IngressError> {
        self.verifies.fetch_add(1, Ordering::Relaxed);
        verify_claim::<D>(proved).map_err(|_| IngressError::Proof)?;
        let published = public_key_com(&proved.public).ok_or(IngressError::Public)?;
        if published != key_com(&tree::epk_words(leaf_x)) {
            return Err(IngressError::LeafKey);
        }
        Ok(())
    }

    fn receipt_count(
        &self,
        body: &[u8],
        item: &SubmittedClaim,
        local: Option<&SigningKey>,
    ) -> usize {
        let mut seen = HashSet::new();
        if let Some(key) = local {
            let pk = key.verifying_key().to_bytes();
            if self.trusted.contains(&pk) {
                seen.insert(pk);
            }
        }
        for receipt in &item.receipts {
            if !self.trusted.contains(&receipt.signer) {
                continue;
            }
            if verify_ed25519(&receipt.signer, &receipt.signature, body) {
                seen.insert(receipt.signer);
            }
        }
        seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ALGO_ACCEPTED, ALGO_PENDING, BATCH_ATTEST_TIMESTAMP, attested_root, receipt_message, sha256,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use headstash_claim::statement::{canonical_claim_message, canonical_sign_message, pack_words};
    use headstash_claim::tree::{self, SecretNote};
    use headstash_claim::{FIXTURE_DEPTH, Opening, derive, prove_claim};
    use headstash_wire::{recover_pubkey_x, sign_personal};
    use std::sync::OnceLock;

    struct Fixture {
        ctx: [u8; 32],
        root: [u8; 32],
        denom: [u8; 32],
        proof_len: usize,
        notes: [SubmittedClaim; 2],
        nullifiers: [[u8; 32]; 2],
    }

    fn fixture() -> &'static Fixture {
        static ONCE: OnceLock<Fixture> = OnceLock::new();
        ONCE.get_or_init(build_fixture)
    }

    fn build_fixture() -> Fixture {
        let leaf = leaf_x();
        let ctx_words = [4u32, 5, 6, 7, 8, 9, 10, 11];
        let secrets = [secret_note(1, &leaf, 15), secret_note(9, &leaf, 15)];
        let (built, path0) = tree::siblings(&secrets, 0);
        let (_, path1) = tree::siblings(&secrets, 1);
        assert_eq!(built.depth, FIXTURE_DEPTH as u32);
        let open0 = opening(
            &secrets[0],
            0,
            array_path(path0),
            [0x1111, 0x2222, 0x3333, 0x4444, 0x5555],
            ctx_words,
        );
        let open1 = opening(
            &secrets[1],
            1,
            array_path(path1),
            [0x0101, 0x0202, 0x0303, 0x0404, 0x0505],
            ctx_words,
        );
        let claim0 = derive(&open0);
        let claim1 = derive(&open1);
        assert_eq!(claim0.root, built.root);
        assert_eq!(claim1.root, built.root);
        assert_ne!(claim0.nullifier, claim1.nullifier);
        let denom = [9u8; 32];
        let note0 = submitted(&open0, &leaf, denom);
        let note1 = submitted(&open1, &leaf, denom);
        assert_eq!(note0.proof.len(), note1.proof.len());
        Fixture {
            ctx: pack_words(&ctx_words),
            root: pack_words(&built.root),
            denom,
            proof_len: note0.proof.len(),
            nullifiers: [pack_words(&claim0.nullifier), pack_words(&claim1.nullifier)],
            notes: [note0, note1],
        }
    }

    fn leaf_x() -> [u8; 32] {
        let signed = sign_personal(&[4u8; 32], b"headstash-claim-v1").unwrap();
        recover_pubkey_x(b"headstash-claim-v1", &signed).unwrap()
    }

    fn secret_note(rseed0: u32, leaf_x: &[u8; 32], v: u64) -> SecretNote {
        SecretNote {
            rseed: [rseed0, 2, 3, 4, 5, 6, 7, 8],
            epk_x: *leaf_x,
            epk_y: [2u8; 32],
            epk_words: tree::epk_words(leaf_x),
            nd: [13, 14],
            v,
            fdi: 1,
        }
    }

    fn array_path(path: Vec<[u32; 8]>) -> [[u32; 8]; FIXTURE_DEPTH] {
        assert_eq!(path.len(), FIXTURE_DEPTH);
        let mut out = [[0u32; 8]; FIXTURE_DEPTH];
        for (i, row) in path.into_iter().enumerate() {
            out[i] = row;
        }
        out
    }

    fn opening(
        note: &SecretNote,
        index: u32,
        siblings: [[u32; 8]; FIXTURE_DEPTH],
        destination: [u32; 5],
        ctx: [u32; 8],
    ) -> Opening<FIXTURE_DEPTH> {
        Opening {
            rseed: note.rseed,
            ctx,
            epk_x: note.epk_words,
            nd: note.nd,
            v: note.v,
            fdi: note.fdi,
            index,
            siblings,
            destination,
        }
    }

    fn submitted(
        opening: &Opening<FIXTURE_DEPTH>,
        leaf_x: &[u8; 32],
        denom: [u8; 32],
    ) -> SubmittedClaim {
        let proved = prove_claim(opening);
        let claim = derive(opening);
        let message = canonical_claim_message(&claim, "terp").unwrap();
        let signature = sign_personal(&[4u8; 32], &message).unwrap().to_vec();
        SubmittedClaim {
            proof: headstash_claim::encode_claim_proof(&proved),
            message,
            signature,
            leaf_x: *leaf_x,
            denom,
            index: None,
            receipts: Vec::new(),
            vk_id: 0,
        }
    }

    fn local_key() -> SigningKey {
        SigningKey::from_bytes(&[8u8; 32])
    }

    fn peer_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn open_at(
        fix: &Fixture,
        ctx: [u8; 32],
        root: [u8; 32],
        cap: u32,
        max_rows: usize,
        threshold: usize,
        reserved: Vec<[u8; 32]>,
    ) -> Campaign<FIXTURE_DEPTH> {
        let mut trusted = vec![local_key().verifying_key().to_bytes()];
        if threshold > 1 {
            trusted.push(peer_key().verifying_key().to_bytes());
        }
        Campaign::open(CampaignParams {
            backend: Backend::Flock,
            ctx,
            allocation_root: root,
            bech32_hrp: "terp".into(),
            denom: fix.denom,
            proof_len: fix.proof_len,
            cutoff_height: 50,
            amounts: BTreeMap::from([(15u64, cap)]),
            trusted,
            trusted_verify_threshold: threshold,
            trusted_sign_threshold: threshold,
            max_rows,
            reserved,
            vk_id: 0,
            vk_hash: [0u8; 32],
        })
        .unwrap()
    }

    fn open(fix: &Fixture) -> Campaign<FIXTURE_DEPTH> {
        open_at(fix, fix.ctx, fix.root, 4, 8, 1, Vec::new())
    }

    fn halo2_campaign(ctx: [u8; 32], root: [u8; 32], denom: [u8; 32]) -> Campaign<FIXTURE_DEPTH> {
        Campaign::open(CampaignParams {
            backend: Backend::Halo2,
            ctx,
            allocation_root: root,
            bech32_hrp: "terp".into(),
            denom,
            proof_len: 4,
            cutoff_height: 50,
            amounts: BTreeMap::from([(15u64, 1u32)]),
            trusted: vec![local_key().verifying_key().to_bytes()],
            trusted_verify_threshold: 1,
            trusted_sign_threshold: 1,
            max_rows: 4,
            reserved: Vec::new(),
            vk_id: 1,
            vk_hash: [0u8; 32],
        })
        .unwrap()
    }

    #[test]
    fn epoch_claim_halo2_opens_and_refuses_key_material() {
        let ctx = [7u8; 32];
        let root = [8u8; 32];
        let denom = [9u8; 32];
        let mut campaign = halo2_campaign(ctx, root, denom);
        assert_eq!(campaign.backend(), Backend::Halo2);
        let destination = "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq0g3g2k";
        let message = canonical_sign_message(&ctx, &root, &[3u8; 32], destination, 15);
        let halo = zk_headstash::claim_auth::canonical_claim_body(
            &ctx,
            &root,
            &[3u8; 32],
            destination,
            15,
        );
        assert_eq!(halo, message);
        let signed = SubmittedClaim {
            proof: vec![1, 2, 3, 4],
            message: message.clone(),
            signature: vec![9],
            leaf_x: [0u8; 32],
            denom,
            index: None,
            receipts: Vec::new(),
            vk_id: 0,
        };
        let report = campaign.accept_batch(&[signed], Some(&local_key()));
        assert_eq!(report.rejected, vec![(0, IngressError::KeyMaterial)]);
        assert_eq!(campaign.verifies(), 0);
        let leaf = SubmittedClaim {
            proof: vec![1, 2, 3, 4],
            message,
            signature: Vec::new(),
            leaf_x: [1u8; 32],
            denom,
            index: None,
            receipts: Vec::new(),
            vk_id: 0,
        };
        let report = campaign.accept_batch(&[leaf], Some(&local_key()));
        assert_eq!(report.rejected[0].1, IngressError::KeyMaterial);
        assert!(campaign.rows().is_empty());
    }

    #[test]
    fn epoch_claim_halo2_proof_is_verified() {
        let ctx = [7u8; 32];
        let root = [0u8; 32];
        let mut nullifier = [0u8; 32];
        nullifier[0] = 3;
        let denom = [9u8; 32];
        let mut campaign = halo2_campaign(ctx, root, denom);
        let message = canonical_sign_message(
            &ctx,
            &root,
            &nullifier,
            "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw",
            15,
        );
        let item = SubmittedClaim {
            proof: vec![1, 2, 3, 4],
            message,
            signature: Vec::new(),
            leaf_x: [0u8; 32],
            denom,
            index: None,
            receipts: Vec::new(),
            vk_id: 1,
        };
        let report = campaign.accept_batch(&[item], Some(&local_key()));
        assert_eq!(report.rejected, vec![(0, IngressError::Proof)]);
        assert_eq!(campaign.verifies(), 1);
        assert!(campaign.rows().is_empty());
    }

    #[test]
    fn epoch_claim_halo2_rejects_a_different_vk_id_before_verify() {
        let ctx = [7u8; 32];
        let root = [0u8; 32];
        let mut nullifier = [0u8; 32];
        nullifier[0] = 3;
        let denom = [9u8; 32];
        let mut campaign = halo2_campaign(ctx, root, denom);
        let message = canonical_sign_message(
            &ctx,
            &root,
            &nullifier,
            "terp1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5ffndw",
            15,
        );
        let item = SubmittedClaim {
            proof: vec![1, 2, 3, 4],
            message,
            signature: Vec::new(),
            leaf_x: [0u8; 32],
            denom,
            index: None,
            receipts: Vec::new(),
            vk_id: 2,
        };
        let report = campaign.accept_batch(&[item], Some(&local_key()));
        assert_eq!(report.rejected, vec![(0, IngressError::Vk)]);
        assert_eq!(campaign.verifies(), 0);
        assert!(campaign.rows().is_empty());
    }

    #[test]
    fn a_good_claim_enters_and_a_bad_length_sibling_does_not_verify() {
        let fix = fixture();
        let mut campaign = open(fix);
        let mut bad = fix.notes[1].clone();
        bad.proof.pop();
        assert_eq!(
            campaign.bisect_first_reject(&[fix.notes[0].clone(), bad.clone()]),
            Some(1)
        );
        assert_eq!(campaign.verifies(), 0);
        assert!(campaign.rows().is_empty());

        let mut indexed = fix.notes[0].clone();
        indexed.index = Some(0);
        let report =
            campaign.accept_batch(&[indexed, fix.notes[0].clone(), bad], Some(&local_key()));
        assert_eq!(report.accepted, vec![1]);
        assert_eq!(report.rejected[0], (0, IngressError::PublicIndex));
        assert_eq!(report.rejected[1], (2, IngressError::Length));
        assert_eq!(campaign.rows().len(), 1);
        assert_eq!(campaign.verifies(), 1);
        assert_eq!(campaign.rows()[0].denom, fix.denom);
        assert_eq!(campaign.rows()[0].nullifier, fix.nullifiers[0]);

        let replay = campaign.accept_batch(&[fix.notes[0].clone()], Some(&local_key()));
        assert_eq!(replay.rejected[0].1, IngressError::Digest);
        assert_eq!(campaign.verifies(), 1);
    }

    #[test]
    fn cheap_rejects_do_not_verify() {
        let fix = fixture();
        let key = local_key();

        let mut wrong_msg = open(fix);
        let mut item = fix.notes[0].clone();
        item.message.push(b'!');
        assert_eq!(
            wrong_msg.accept_batch(&[item], Some(&key)).rejected[0].1,
            IngressError::Message
        );
        assert_eq!(wrong_msg.verifies(), 0);

        let mut wrong_sig = open(fix);
        let mut item = fix.notes[0].clone();
        item.signature[0] ^= 0xff;
        assert_eq!(
            wrong_sig.accept_batch(&[item], Some(&key)).rejected[0].1,
            IngressError::LeafKey
        );
        assert_eq!(wrong_sig.verifies(), 0);

        let mut wrong_denom = open(fix);
        let mut item = fix.notes[0].clone();
        item.denom[0] ^= 1;
        assert_eq!(
            wrong_denom.accept_batch(&[item], Some(&key)).rejected[0].1,
            IngressError::Denom
        );
        assert_eq!(wrong_denom.verifies(), 0);

        let mut wrong_ctx = open_at(
            fix,
            {
                let mut ctx = fix.ctx;
                ctx[0] ^= 1;
                ctx
            },
            fix.root,
            4,
            8,
            1,
            Vec::new(),
        );
        assert_eq!(
            wrong_ctx
                .accept_batch(&[fix.notes[0].clone()], Some(&key))
                .rejected[0]
                .1,
            IngressError::Context
        );
        assert_eq!(wrong_ctx.verifies(), 0);

        let mut wrong_root = open_at(
            fix,
            fix.ctx,
            {
                let mut root = fix.root;
                root[0] ^= 1;
                root
            },
            4,
            8,
            1,
            Vec::new(),
        );
        assert_eq!(
            wrong_root
                .accept_batch(&[fix.notes[0].clone()], Some(&key))
                .rejected[0]
                .1,
            IngressError::Root
        );
        assert_eq!(wrong_root.verifies(), 0);

        let mut unknown_amount = open_at(fix, fix.ctx, fix.root, 4, 8, 1, Vec::new());
        unknown_amount.amounts.remove(&15);
        assert_eq!(
            unknown_amount
                .accept_batch(&[fix.notes[0].clone()], Some(&key))
                .rejected[0]
                .1,
            IngressError::Amount
        );
        assert_eq!(unknown_amount.verifies(), 0);

        let mut reserved = open_at(fix, fix.ctx, fix.root, 4, 8, 1, vec![fix.nullifiers[0]]);
        assert_eq!(
            reserved
                .accept_batch(&[fix.notes[0].clone()], Some(&key))
                .rejected[0]
                .1,
            IngressError::Reserved
        );
        assert_eq!(reserved.verifies(), 0);
    }

    #[test]
    fn the_second_note_fills_the_class_and_a_short_receipt_set_does_not_verify() {
        let fix = fixture();
        let key = local_key();
        let mut capped = open_at(fix, fix.ctx, fix.root, 1, 8, 1, Vec::new());
        let first = capped.accept_batch(&[fix.notes[0].clone()], Some(&key));
        assert_eq!(first.accepted, vec![0]);
        let second = capped.accept_batch(&[fix.notes[1].clone()], Some(&key));
        assert_eq!(second.rejected[0].1, IngressError::Cap);
        assert_eq!(capped.verifies(), 1);
        assert_eq!(capped.rows().len(), 1);

        let mut gated = open_at(fix, fix.ctx, fix.root, 4, 8, 2, Vec::new());
        let missing = gated.accept_batch(&[fix.notes[1].clone()], Some(&key));
        assert_eq!(missing.rejected[0].1, IngressError::Receipts);
        assert_eq!(gated.verifies(), 0);

        let mut with_peer = fix.notes[1].clone();
        with_peer.receipts.push(peer_receipt(&with_peer));
        let opened = gated.accept_batch(&[with_peer], Some(&key));
        assert_eq!(opened.accepted, vec![0]);
        assert_eq!(gated.verifies(), 1);
        assert_eq!(gated.rows()[0].nullifier, fix.nullifiers[1]);
    }

    #[test]
    fn a_spent_nullifier_with_a_fresh_digest_slot_is_spent() {
        let fix = fixture();
        let mut campaign = open(fix);
        let ok = campaign.accept_batch(&[fix.notes[0].clone()], Some(&local_key()));
        assert_eq!(ok.accepted, vec![0]);
        let digest = sha256(&fix.notes[0].proof);
        assert!(campaign.seen.remove(&digest));
        let again = campaign.accept_batch(&[fix.notes[0].clone()], Some(&local_key()));
        assert_eq!(again.rejected[0].1, IngressError::Spent);
        assert_eq!(campaign.verifies(), 1);
    }

    #[test]
    fn seal_signs_pending_below_the_cutoff_and_accepted_at_it() {
        let fix = fixture();
        let mut campaign = open(fix);
        let key = local_key();
        let report =
            campaign.accept_batch(&[fix.notes[0].clone(), fix.notes[1].clone()], Some(&key));
        assert_eq!(report.accepted, vec![0, 1]);
        assert_eq!(campaign.trusted_sign_threshold(), 1);

        let custody = SigningKey::from_bytes(&[3u8; 32]);
        let pending = campaign.seal(49, &custody);
        assert_eq!(pending.algo, ALGO_PENDING);
        assert_eq!(pending.batch_hash, campaign.pending_root());
        assert_eq!(pending.attested, attested_root(&pending.batch_hash));
        assert_eq!(pending.attestation_height, 1);
        assert_eq!(pending.foreign_height, 49);
        assert!(pending.custody_ok());

        let accepted = campaign.seal(50, &custody);
        assert_eq!(accepted.algo, ALGO_ACCEPTED);
        assert_eq!(accepted.batch_hash, campaign.accepted_root());
        assert_eq!(accepted.attestation_timestamp, BATCH_ATTEST_TIMESTAMP);
        assert!(accepted.custody_ok());
        assert_ne!(accepted.batch_hash, pending.batch_hash);
        assert_eq!(campaign.rows()[0].amount, 15);
        assert!(
            campaign
                .rows()
                .iter()
                .all(|row| row.destination.starts_with("terp1"))
        );
    }

    #[test]
    fn an_amount_above_u32_max_is_stored_on_the_row() {
        let wide = u64::from(u32::MAX) + 1;
        let leaf = leaf_x();
        let ctx_words = [4u32, 5, 6, 7, 8, 9, 10, 11];
        let notes = [secret_note(3, &leaf, wide)];
        let (built, path) = tree::siblings(&notes, 0);
        let opening = opening(
            &notes[0],
            0,
            array_path(path),
            [0x1111, 0x2222, 0x3333, 0x4444, 0x5555],
            ctx_words,
        );
        let claim = derive(&opening);
        assert_eq!(claim.amount, wide);
        assert_eq!(claim.root, built.root);
        let denom = [9u8; 32];
        let item = submitted(&opening, &leaf, denom);
        let text = String::from_utf8(item.message.clone()).unwrap();
        assert!(text.ends_with(&format!(":{wide}")));
        let mut campaign = Campaign::<FIXTURE_DEPTH>::open(CampaignParams {
            backend: Backend::Flock,
            ctx: pack_words(&ctx_words),
            allocation_root: pack_words(&built.root),
            bech32_hrp: "terp".into(),
            denom,
            proof_len: item.proof.len(),
            cutoff_height: 50,
            amounts: BTreeMap::from([(wide, 1u32)]),
            trusted: vec![local_key().verifying_key().to_bytes()],
            trusted_verify_threshold: 1,
            trusted_sign_threshold: 1,
            max_rows: 4,
            reserved: Vec::new(),
            vk_id: 0,
            vk_hash: [0u8; 32],
        })
        .unwrap();
        let report = campaign.accept_batch(&[item], Some(&local_key()));
        assert_eq!(report.accepted, vec![0]);
        assert!(report.rejected.is_empty());
        assert_eq!(campaign.rows()[0].amount, wide);
    }

    fn peer_receipt(item: &SubmittedClaim) -> PeerReceipt {
        let text = std::str::from_utf8(&item.message).unwrap();
        let parts: Vec<&str> = text.split(':').collect();
        assert_eq!(parts.len(), 6, "canonical body has five fields");
        let ctx: [u8; 32] = hex::decode(parts[1]).unwrap().try_into().unwrap();
        let root: [u8; 32] = hex::decode(parts[2]).unwrap().try_into().unwrap();
        let nullifier: [u8; 32] = hex::decode(parts[3]).unwrap().try_into().unwrap();
        let amount: u64 = parts[5].parse().unwrap();
        let digest = sha256(&item.proof);
        let body = receipt_message(&ctx, &root, &nullifier, parts[4], amount, &digest);
        let key = peer_key();
        PeerReceipt {
            signer: key.verifying_key().to_bytes(),
            signature: key.sign(&body).to_bytes(),
        }
    }
}

struct ParsedBody {
    ctx: [u8; 32],
    root: [u8; 32],
    nullifier: [u8; 32],
    destination: String,
    amount: u64,
}

fn parse_canonical_message(message: &[u8]) -> Option<ParsedBody> {
    let text = std::str::from_utf8(message).ok()?;
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 6 || parts[0] != "headstash-claim-v1" {
        return None;
    }
    let ctx = hex32(parts[1])?;
    let root = hex32(parts[2])?;
    let nullifier = hex32(parts[3])?;
    if parts[4].is_empty() || parts[4].len() > 128 {
        return None;
    }
    let amount = parts[5].parse().ok()?;
    Some(ParsedBody {
        ctx,
        root,
        nullifier,
        destination: parts[4].to_string(),
        amount,
    })
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let bytes = hex::decode(text).ok()?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn structural<const D: usize>(
    campaign: &Campaign<D>,
    item: &SubmittedClaim,
) -> Result<(), IngressError> {
    if item.index.is_some() {
        return Err(IngressError::PublicIndex);
    }
    if item.proof.len() != campaign.proof_len {
        return Err(IngressError::Length);
    }
    Ok(())
}
