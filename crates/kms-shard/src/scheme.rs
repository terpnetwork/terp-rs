use std::collections::BTreeMap;

use frost_ed25519 as frost;
use frost_ed25519::keys::dkg;

pub const SCHEME_ID: &str = "frost-ed25519-v1";

pub type ParticipantId = u16;
pub type DkgR1Secret = dkg::round1::SecretPackage;
pub type DkgR2Secret = dkg::round2::SecretPackage;
pub type SigningNonces = frost::round1::SigningNonces;
pub type CommitmentMap = BTreeMap<frost::Identifier, frost::round1::SigningCommitments>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("frost: {0}")]
    Frost(#[from] frost::Error),
    #[error("unknown participant {0}")]
    Participant(ParticipantId),
    #[error("combine produced an invalid Ed25519 signature")]
    BadSignature,
}

/// Threshold consensus scheme. v1 impl is FROST-Ed25519 only.
pub trait ConsensusScheme {
    type GroupPublic;
    type Share;
    type Partial;
    type Signature;
    type DkgRound1Pub;
    type DkgRound2Pub;

    fn scheme_id() -> &'static str;

    fn dkg_r1(
        id: ParticipantId,
        n: u16,
        t: u16,
        rng: &mut impl rand_core::CryptoRngCore,
    ) -> Result<(DkgR1Secret, Self::DkgRound1Pub), Error>;

    fn dkg_r2(
        sec: DkgR1Secret,
        r1: &BTreeMap<ParticipantId, Self::DkgRound1Pub>,
    ) -> Result<(DkgR2Secret, BTreeMap<ParticipantId, Self::DkgRound2Pub>), Error>;

    fn dkg_finish(
        sec: DkgR2Secret,
        r1: &BTreeMap<ParticipantId, Self::DkgRound1Pub>,
        r2: &BTreeMap<ParticipantId, Self::DkgRound2Pub>,
    ) -> Result<(Self::Share, Self::GroupPublic), Error>;

    fn commit(
        share: &Self::Share,
        rng: &mut impl rand_core::CryptoRngCore,
    ) -> Result<(SigningNonces, frost::round1::SigningCommitments), Error>;

    fn sign_share(
        share: &Self::Share,
        nonces: SigningNonces,
        commit_map: &CommitmentMap,
        msg: &[u8],
    ) -> Result<Self::Partial, Error>;

    fn combine(
        commit_map: &CommitmentMap,
        partials: &BTreeMap<ParticipantId, Self::Partial>,
        msg: &[u8],
        group: &Self::GroupPublic,
    ) -> Result<Self::Signature, Error>;

    fn verify(group: &Self::GroupPublic, msg: &[u8], sig: &Self::Signature) -> bool;

    fn group_pk_bytes(group: &Self::GroupPublic) -> Result<[u8; 32], Error>;
    fn signature_bytes(sig: &Self::Signature) -> Result<[u8; 64], Error>;
}

pub fn identifier(id: ParticipantId) -> Result<frost::Identifier, Error> {
    frost::Identifier::try_from(id).map_err(|_| Error::Participant(id))
}
