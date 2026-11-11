//! FROST-Ed25519 helpers shared by the workstream-A modules (weighted DKG, signing).
use anyhow::{Result, anyhow};
use cosmwasm_std::HexBinary;
use frost_ed25519 as frost;
use rand08::rngs::OsRng;
use std::collections::BTreeMap;

/// One FROST group: `n` shares, threshold `min`, produced by a real 3-round DKG.
pub struct Group {
    pub keys: BTreeMap<frost::Identifier, frost::keys::KeyPackage>,
    pub pubkey: frost::keys::PublicKeyPackage,
}

impl Group {
    pub fn dkg(n: u16, min: u16) -> Result<Self> {
        let rng = OsRng;
        let ids: Vec<frost::Identifier> =
            (1..=n).map(|i| i.try_into()).collect::<Result<_, _>>()?;
        let (mut r1s, mut r1p) = (BTreeMap::new(), BTreeMap::new());
        for id in &ids {
            let (s, p) = frost::keys::dkg::part1(*id, n, min, rng)?;
            r1s.insert(*id, s);
            r1p.insert(*id, p);
        }
        let others = |id: &frost::Identifier| -> BTreeMap<_, _> {
            r1p.iter()
                .filter(|(k, _)| *k != id)
                .map(|(k, v)| (*k, v.clone()))
                .collect()
        };
        let mut r2s = BTreeMap::new();
        let mut r2p: BTreeMap<frost::Identifier, BTreeMap<_, _>> = BTreeMap::new();
        for id in &ids {
            let s1 = r1s.remove(id).ok_or_else(|| anyhow!("dkg round1"))?;
            let (s, out) = frost::keys::dkg::part2(s1, &others(id))?;
            r2s.insert(*id, s);
            for (to, pkg) in out {
                r2p.entry(to).or_default().insert(*id, pkg);
            }
        }
        let mut keys = BTreeMap::new();
        let mut pubkey = None;
        for id in &ids {
            let (kp, pk) = frost::keys::dkg::part3(&r2s[id], &others(id), &r2p[id])?;
            keys.insert(*id, kp);
            pubkey = Some(pk);
        }
        Ok(Group {
            keys,
            pubkey: pubkey.ok_or_else(|| anyhow!("empty dkg"))?,
        })
    }

    /// Trusted-dealer group (used for the election-authority committee).
    pub fn dealer(n: u16, min: u16) -> Result<Self> {
        let (shares, pubkey) =
            frost::keys::generate_with_dealer(n, min, frost::keys::IdentifierList::Default, OsRng)?;
        let keys = shares
            .into_iter()
            .map(|(id, s)| Ok((id, frost::keys::KeyPackage::try_from(s)?)))
            .collect::<Result<_, frost::Error>>()?;
        Ok(Group { keys, pubkey })
    }

    pub fn vk(&self) -> Result<HexBinary> {
        Ok(HexBinary::from(self.pubkey.verifying_key().serialize()?))
    }

    /// Sign with exactly the given share identifiers. Errors (no signature) when the
    /// signers are below threshold: that is the property the auction relies on.
    pub fn sign(&self, msg: &[u8], signers: &[u16]) -> Result<HexBinary, frost::Error> {
        let mut rng = OsRng;
        let ids: Vec<frost::Identifier> = signers
            .iter()
            .map(|i| (*i).try_into())
            .collect::<Result<_, _>>()?;
        let (mut nonces, mut commits) = (BTreeMap::new(), BTreeMap::new());
        for id in &ids {
            let kp = self.keys.get(id).ok_or(frost::Error::UnknownIdentifier)?;
            let (n, c) = frost::round1::commit(kp.signing_share(), &mut rng);
            nonces.insert(*id, n);
            commits.insert(*id, c);
        }
        let pkg = frost::SigningPackage::new(commits, msg);
        let mut shares = BTreeMap::new();
        for id in &ids {
            shares.insert(*id, frost::round2::sign(&pkg, &nonces[id], &self.keys[id])?);
        }
        Ok(HexBinary::from(
            frost::aggregate(&pkg, &shares, &self.pubkey)?.serialize()?,
        ))
    }
}
