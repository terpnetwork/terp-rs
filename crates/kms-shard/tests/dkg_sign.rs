use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use kms_shard::{dkg_all, ConsensusScheme, FrostEd25519, SCHEME_FROST_ED25519_V1};
use rand::rngs::OsRng;

#[test]
fn scheme_id_is_frost_ed25519_v1() {
    assert_eq!(FrostEd25519::scheme_id(), SCHEME_FROST_ED25519_V1);
}

fn sign_with(
    shares: &[(u16, frost_ed25519::keys::KeyPackage)],
    group: &frost_ed25519::keys::PublicKeyPackage,
    ids: &[u16],
    msg: &[u8],
    rng: &mut OsRng,
) -> [u8; 64] {
    let mut commit_map = BTreeMap::new();
    let mut nonces = BTreeMap::new();
    for id in ids {
        let share = shares.iter().find(|(i, _)| i == id).unwrap().1.clone();
        let (n, c) = FrostEd25519::commit(&share, rng).unwrap();
        let ident = frost_ed25519::Identifier::try_from(*id).unwrap();
        commit_map.insert(ident, c);
        nonces.insert(*id, (share, n));
    }
    let mut partials = BTreeMap::new();
    for id in ids {
        let (share, n) = nonces.remove(id).unwrap();
        let p = FrostEd25519::sign_share(&share, n, &commit_map, msg).unwrap();
        partials.insert(*id, p);
    }
    let sig = FrostEd25519::combine(&commit_map, &partials, msg, group).unwrap();
    assert!(FrostEd25519::verify(group, msg, &sig));
    FrostEd25519::signature_bytes(&sig).unwrap()
}

#[test]
fn dkg_2_of_3_signs_and_verifies_as_ed25519() {
    let mut rng = OsRng;
    let (shares, group) = dkg_all::<FrostEd25519>(3, 2, &mut rng).expect("dkg");
    assert_eq!(shares.len(), 3);
    let pk = FrostEd25519::group_pk_bytes(&group).unwrap();
    let vk = VerifyingKey::from_bytes(&pk).expect("32-byte ed25519 pk");

    let msg = b"cometbft-vote-bytes";
    let sig_bytes = sign_with(&shares, &group, &[1, 2], msg, &mut rng);
    vk.verify(msg, &Signature::from_bytes(&sig_bytes))
        .expect("dalek verifies FROST sig");

    let sig_b = sign_with(&shares, &group, &[2, 3], msg, &mut rng);
    vk.verify(msg, &Signature::from_bytes(&sig_b)).unwrap();
}

#[test]
fn below_threshold_cannot_combine() {
    let mut rng = OsRng;
    let (shares, group) = dkg_all::<FrostEd25519>(3, 2, &mut rng).unwrap();
    let msg = b"one-share-must-fail";
    let mut commit_map = BTreeMap::new();
    let share = &shares[0].1;
    let (n, c) = FrostEd25519::commit(share, &mut rng).unwrap();
    let ident = frost_ed25519::Identifier::try_from(shares[0].0).unwrap();
    commit_map.insert(ident, c);
    let signed = FrostEd25519::sign_share(share, n, &commit_map, msg);
    assert!(
        signed.is_err(),
        "<t commitments must fail closed before combine"
    );
    let _ = group;
}

#[test]
fn dkg_3_of_3_group_pk_agrees() {
    let mut rng = OsRng;
    let (shares, group) = dkg_all::<FrostEd25519>(3, 3, &mut rng).unwrap();
    let want = FrostEd25519::group_pk_bytes(&group).unwrap();
    for (_, s) in &shares {
        let got: Vec<u8> = s.verifying_key().serialize().unwrap();
        assert_eq!(got.as_slice(), &want);
    }
}
