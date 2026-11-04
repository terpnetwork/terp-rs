use std::collections::BTreeMap;

use frost_ed25519 as frost;
use frost_ed25519::keys::dkg;
use rand_core::CryptoRngCore;

use crate::scheme::{
    identifier, CommitmentMap, ConsensusScheme, DkgR1Secret, DkgR2Secret, Error, ParticipantId,
    SigningNonces, SCHEME_ID,
};

/// Every participant's key share plus the group public key, as produced by DKG.
pub type DkgOutput<S> = (
    Vec<(ParticipantId, <S as ConsensusScheme>::Share)>,
    <S as ConsensusScheme>::GroupPublic,
);

/// Run  t-of-n DKG among ids 1..=n. Host shuttles public packages only.
pub fn dkg_all<
    S: ConsensusScheme<
        DkgRound1Pub = dkg::round1::Package,
        DkgRound2Pub = dkg::round2::Package,
        Share = frost::keys::KeyPackage,
        GroupPublic = frost::keys::PublicKeyPackage,
    >,
>(
    n: u16,
    t: u16,
    rng: &mut impl CryptoRngCore,
) -> Result<DkgOutput<S>, Error> {
    let ids: Vec<ParticipantId> = (1..=n).collect();
    let mut r1_sec = BTreeMap::new();
    let mut r1_pub = BTreeMap::new();
    for id in &ids {
        let (sec, pkg) = S::dkg_r1(*id, n, t, rng)?;
        r1_sec.insert(*id, sec);
        r1_pub.insert(*id, pkg);
    }

    let mut r2_sec = BTreeMap::new();
    let mut r2_inbox: BTreeMap<ParticipantId, BTreeMap<ParticipantId, S::DkgRound2Pub>> =
        BTreeMap::new();
    for id in &ids {
        let mut others = r1_pub.clone();
        others.remove(id);
        let (sec, by_peer) = S::dkg_r2(r1_sec.remove(id).expect("r1"), &others)?;
        r2_sec.insert(*id, sec);
        for (peer, pkg) in by_peer {
            r2_inbox.entry(peer).or_default().insert(*id, pkg);
        }
    }

    let mut shares = Vec::new();
    let mut group = None;
    for id in &ids {
        let mut r1_others = r1_pub.clone();
        r1_others.remove(id);
        let r2_for_me = r2_inbox.remove(id).unwrap_or_default();
        let (share, pk) = S::dkg_finish(r2_sec.remove(id).expect("r2"), &r1_others, &r2_for_me)?;
        if let Some(ref g) = group {
            let a = S::group_pk_bytes(g)?;
            let b = S::group_pk_bytes(&pk)?;
            if a != b {
                return Err(Error::BadSignature);
            }
        } else {
            group = Some(pk.clone());
        }
        shares.push((*id, share));
    }
    Ok((shares, group.expect("n>=1")))
}

/// v1 production scheme. No BLS.
pub struct FrostEd25519;

impl ConsensusScheme for FrostEd25519 {
    type GroupPublic = frost::keys::PublicKeyPackage;
    type Share = frost::keys::KeyPackage;
    type Partial = frost::round2::SignatureShare;
    type Signature = frost::Signature;
    type DkgRound1Pub = dkg::round1::Package;
    type DkgRound2Pub = dkg::round2::Package;

    fn scheme_id() -> &'static str {
        SCHEME_ID
    }

    fn dkg_r1(
        id: ParticipantId,
        n: u16,
        t: u16,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(DkgR1Secret, Self::DkgRound1Pub), Error> {
        Ok(dkg::part1(identifier(id)?, n, t, rng)?)
    }

    fn dkg_r2(
        sec: DkgR1Secret,
        r1: &BTreeMap<ParticipantId, Self::DkgRound1Pub>,
    ) -> Result<(DkgR2Secret, BTreeMap<ParticipantId, Self::DkgRound2Pub>), Error> {
        let mut pkgs = BTreeMap::new();
        for (id, pkg) in r1 {
            pkgs.insert(identifier(*id)?, pkg.clone());
        }
        let (sec2, by_peer) = dkg::part2(sec, &pkgs)?;
        let mut out = BTreeMap::new();
        for (ident, pkg) in by_peer {
            let pid = r1
                .keys()
                .copied()
                .find(|id| identifier(*id).ok() == Some(ident))
                .ok_or(Error::Participant(0))?;
            out.insert(pid, pkg);
        }
        Ok((sec2, out))
    }

    fn dkg_finish(
        sec: DkgR2Secret,
        r1: &BTreeMap<ParticipantId, Self::DkgRound1Pub>,
        r2: &BTreeMap<ParticipantId, Self::DkgRound2Pub>,
    ) -> Result<(Self::Share, Self::GroupPublic), Error> {
        let mut r1m = BTreeMap::new();
        for (id, pkg) in r1 {
            r1m.insert(identifier(*id)?, pkg.clone());
        }
        let mut r2m = BTreeMap::new();
        for (id, pkg) in r2 {
            r2m.insert(identifier(*id)?, pkg.clone());
        }
        Ok(dkg::part3(&sec, &r1m, &r2m)?)
    }

    fn commit(
        share: &Self::Share,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(SigningNonces, frost::round1::SigningCommitments), Error> {
        Ok(frost::round1::commit(share.signing_share(), rng))
    }

    fn sign_share(
        share: &Self::Share,
        nonces: SigningNonces,
        commit_map: &CommitmentMap,
        msg: &[u8],
    ) -> Result<Self::Partial, Error> {
        let signing_package = frost::SigningPackage::new(commit_map.clone(), msg);
        Ok(frost::round2::sign(&signing_package, &nonces, share)?)
    }

    fn combine(
        commit_map: &CommitmentMap,
        partials: &BTreeMap<ParticipantId, Self::Partial>,
        msg: &[u8],
        group: &Self::GroupPublic,
    ) -> Result<Self::Signature, Error> {
        let signing_package = frost::SigningPackage::new(commit_map.clone(), msg);
        let mut shares = BTreeMap::new();
        for (id, p) in partials {
            shares.insert(identifier(*id)?, *p);
        }
        Ok(frost::aggregate(&signing_package, &shares, group)?)
    }

    fn verify(group: &Self::GroupPublic, msg: &[u8], sig: &Self::Signature) -> bool {
        group.verifying_key().verify(msg, sig).is_ok()
    }

    fn group_pk_bytes(group: &Self::GroupPublic) -> Result<[u8; 32], Error> {
        let v: Vec<u8> = group.verifying_key().serialize()?;
        let arr: [u8; 32] = v.try_into().map_err(|_| Error::BadSignature)?;
        Ok(arr)
    }

    fn signature_bytes(sig: &Self::Signature) -> Result<[u8; 64], Error> {
        let v: Vec<u8> = sig.serialize()?;
        let arr: [u8; 64] = v.try_into().map_err(|_| Error::BadSignature)?;
        Ok(arr)
    }
}
