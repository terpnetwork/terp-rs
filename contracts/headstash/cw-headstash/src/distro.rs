//! Poseidon-v1 public inclusion / distro-tree surface.
//!
//! Pure validation + registry helpers for eligibility roots. Does **not** depend on
//! Halo2 prove/verify greening — claim circuits may lag this state machine.
//!
//! See ADR-POSEIDON-DISTRO-TREE and CLARITY §1 (additive Headstash sets).

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, StdError, StdResult, Storage};
use cw_storage_plus::{Item, Map};

/// Genesis eligibility set always uses this id.
pub const GENESIS_ROOT_ID: u64 = 0;

/// Expected Merkle root / field-element encoding length for distro roots.
pub const DISTRO_ROOT_LEN: usize = 32;

/// Wire / JSON unit names are `poseidon_v1` / `sinsemilla_legacy` (cw_serde identifiers).
/// Human/ADR tags from [`DistroHashDomain::as_str`] are `poseidon-v1` / `sinsemilla-legacy`.
///
/// New Headstashes default to [`DistroHashDomain::PoseidonV1`].
/// [`DistroHashDomain::SinsemillaLegacy`] is recovery-only for pre-Poseidon trees.
#[cw_serde]
#[derive(Copy, Default)]
pub enum DistroHashDomain {
    /// Poseidon over `pallas::Base` — ADR default for public inclusion.
    #[default]
    PoseidonV1,
    /// Orchard-family Sinsemilla — legacy recovery only; not for new drops.
    SinsemillaLegacy,
}

impl DistroHashDomain {
    pub const POSEIDON_V1_STR: &'static str = "poseidon-v1";
    pub const SINSEMILLA_LEGACY_STR: &'static str = "sinsemilla-legacy";

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PoseidonV1 => Self::POSEIDON_V1_STR,
            Self::SinsemillaLegacy => Self::SINSEMILLA_LEGACY_STR,
        }
    }

    /// Parse a domain tag. Unknown tags are rejected.
    pub fn parse(s: &str) -> StdResult<Self> {
        match s {
            Self::POSEIDON_V1_STR | "PoseidonV1" | "poseidon_v1" => Ok(Self::PoseidonV1),
            Self::SINSEMILLA_LEGACY_STR | "SinsemillaLegacy" | "sinsemilla_legacy" => {
                Ok(Self::SinsemillaLegacy)
            }
            other => Err(StdError::msg(format!(
                "unknown distro_hash_domain: {other} (expected poseidon-v1 or sinsemilla-legacy)"
            ))),
        }
    }

    /// Policy for **new** additive roots: only Poseidon-v1 is accepted.
    pub fn allowed_for_new_registration(&self) -> bool {
        matches!(self, Self::PoseidonV1)
    }
}

/// One registered public eligibility root (one additive Headstash set).
#[cw_serde]
pub struct EligibilityRootEntry {
    pub root_id: u64,
    /// 32-byte distro tree root (Poseidon-v1 or legacy encoding).
    pub root: Binary,
    pub domain: DistroHashDomain,
    /// Optional UI / suite label (e.g. `drop-0`, `community-batch-2`).
    pub label: Option<String>,
}

/// Storage for additive eligibility roots: `root_id → entry`.
pub const ELIGIBILITY_ROOTS: Map<u64, EligibilityRootEntry> = Map::new("elig_roots");

/// Monotonic counter; next free `root_id` (starts at 1 after genesis is 0).
pub const NEXT_ROOT_ID: Item<u64> = Item::new("next_root_id");

/// Validate raw root bytes for on-chain registration.
pub fn validate_root_bytes(root: &Binary) -> StdResult<()> {
    if root.is_empty() {
        return Err(StdError::msg("empty eligibility root"));
    }
    if root.len() != DISTRO_ROOT_LEN {
        return Err(StdError::msg(format!(
            "eligibility root must be {DISTRO_ROOT_LEN} bytes, got {}",
            root.len()
        )));
    }
    Ok(())
}

/// Validate a hash domain for **instantiation** (genesis).
///
/// Poseidon-v1 is the default. Sinsemilla-legacy is allowed only when explicitly
/// requested (recovery). Unknown domains never reach here if callers use the enum.
pub fn validate_instantiate_domain(domain: DistroHashDomain) -> StdResult<()> {
    match domain {
        DistroHashDomain::PoseidonV1 | DistroHashDomain::SinsemillaLegacy => Ok(()),
    }
}

/// Validate a hash domain for **additive** registration (new drops).
pub fn validate_additive_domain(domain: DistroHashDomain) -> StdResult<()> {
    if domain.allowed_for_new_registration() {
        Ok(())
    } else {
        Err(StdError::msg(format!(
            "additive roots require poseidon-v1; got {}",
            domain.as_str()
        )))
    }
}

/// Register genesis root at [`GENESIS_ROOT_ID`] and initialize the id counter.
pub fn register_genesis_root(
    storage: &mut dyn Storage,
    root: Binary,
    domain: DistroHashDomain,
    label: Option<String>,
) -> StdResult<EligibilityRootEntry> {
    validate_root_bytes(&root)?;
    validate_instantiate_domain(domain)?;

    let entry = EligibilityRootEntry {
        root_id: GENESIS_ROOT_ID,
        root: root.clone(),
        domain,
        label,
    };
    ELIGIBILITY_ROOTS.save(storage, GENESIS_ROOT_ID, &entry)?;
    NEXT_ROOT_ID.save(storage, &1)?;
    Ok(entry)
}

/// Add an eligibility root (additive Headstash). Returns the assigned `root_id`.
pub fn register_eligibility_root(
    storage: &mut dyn Storage,
    root: Binary,
    domain: DistroHashDomain,
    label: Option<String>,
) -> StdResult<EligibilityRootEntry> {
    validate_root_bytes(&root)?;
    validate_additive_domain(domain)?;

    let root_id = NEXT_ROOT_ID.may_load(storage)?.unwrap_or(1);
    let entry = EligibilityRootEntry {
        root_id,
        root,
        domain,
        label,
    };
    ELIGIBILITY_ROOTS.save(storage, root_id, &entry)?;
    NEXT_ROOT_ID.save(storage, &(root_id + 1))?;
    Ok(entry)
}

/// Load a registered root or error if unknown.
pub fn require_root(storage: &dyn Storage, root_id: u64) -> StdResult<EligibilityRootEntry> {
    ELIGIBILITY_ROOTS
        .may_load(storage, root_id)?
        .ok_or_else(|| StdError::msg(format!("unregistered eligibility root_id: {root_id}")))
}

/// Claim must bind to a registered root. The public instance **anchor** must
/// equal the registered root bytes (Poseidon-v1 depth-32 path root for new drops).
///
/// Empty anchors are rejected. Domain is returned so callers can assert policy
/// (new claims should use `poseidon-v1` roots registered on-chain).
pub fn assert_claim_root(
    storage: &dyn Storage,
    root_id: u64,
    claim_anchor: Option<&Binary>,
) -> StdResult<EligibilityRootEntry> {
    let entry = require_root(storage, root_id)?;
    let Some(anchor) = claim_anchor else {
        return Err(StdError::msg(format!(
            "claim missing anchor for root_id {root_id}"
        )));
    };
    if anchor.is_empty() {
        return Err(StdError::msg(format!(
            "claim anchor empty for root_id {root_id}"
        )));
    }
    if anchor.as_slice() != entry.root.as_slice() {
        return Err(StdError::msg(format!(
            "claim anchor does not match registered root_id {root_id}"
        )));
    }
    Ok(entry)
}

/// Policy helper: new claim paths should only spend under Poseidon-v1 roots.
pub fn assert_claim_domain_poseidon_v1(entry: &EligibilityRootEntry) -> StdResult<()> {
    if entry.domain != DistroHashDomain::PoseidonV1 {
        return Err(StdError::msg(format!(
            "claim root_id {} uses {}; expected poseidon-v1 for new claims",
            entry.root_id,
            entry.domain.as_str()
        )));
    }
    Ok(())
}

/// Domain-separated nullifier storage key: claim in set `i` cannot be replayed as set `j`.
///
/// Format: `{root_id}:{nullifier_hex}`
pub fn nullifier_storage_key(root_id: u64, nullifier_hex: &str) -> String {
    format!("{root_id}:{nullifier_hex}")
}

/// In-memory / pure registry used by unit tests without CosmWasm storage backends.
#[derive(Clone, Debug, Default)]
pub struct DistroRegistry {
    pub roots: std::collections::BTreeMap<u64, EligibilityRootEntry>,
    pub next_id: u64,
    /// Domain-separated nullifiers already spent: `root_id:nf_hex`.
    pub nullifiers: std::collections::BTreeSet<String>,
}

impl DistroRegistry {
    pub fn new_with_genesis(
        root: Binary,
        domain: DistroHashDomain,
        label: Option<String>,
    ) -> StdResult<Self> {
        validate_root_bytes(&root)?;
        validate_instantiate_domain(domain)?;
        let mut roots = std::collections::BTreeMap::new();
        roots.insert(
            GENESIS_ROOT_ID,
            EligibilityRootEntry {
                root_id: GENESIS_ROOT_ID,
                root,
                domain,
                label,
            },
        );
        Ok(Self {
            roots,
            next_id: 1,
            nullifiers: Default::default(),
        })
    }

    pub fn register_root(
        &mut self,
        root: Binary,
        domain: DistroHashDomain,
        label: Option<String>,
    ) -> StdResult<EligibilityRootEntry> {
        validate_root_bytes(&root)?;
        validate_additive_domain(domain)?;
        let root_id = self.next_id;
        let entry = EligibilityRootEntry {
            root_id,
            root,
            domain,
            label,
        };
        self.roots.insert(root_id, entry.clone());
        self.next_id = root_id + 1;
        Ok(entry)
    }

    pub fn require_root(&self, root_id: u64) -> StdResult<&EligibilityRootEntry> {
        self.roots
            .get(&root_id)
            .ok_or_else(|| StdError::msg(format!("unregistered eligibility root_id: {root_id}")))
    }

    /// Record a claim nullifier under `root_id`. Rejects doubles under the same scope.
    pub fn spend_nullifier(&mut self, root_id: u64, nullifier_hex: &str) -> StdResult<()> {
        self.require_root(root_id)?;
        let key = nullifier_storage_key(root_id, nullifier_hex);
        if !self.nullifiers.insert(key) {
            return Err(StdError::msg("nullifier already exists"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::MockStorage;

    fn root32(byte: u8) -> Binary {
        Binary::from(vec![byte; DISTRO_ROOT_LEN])
    }

    #[test]
    fn reject_empty_root() {
        let err = validate_root_bytes(&Binary::default()).unwrap_err();
        assert!(err.to_string().contains("empty"));

        let err = DistroRegistry::new_with_genesis(
            Binary::default(),
            DistroHashDomain::PoseidonV1,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn reject_wrong_root_len() {
        let err = validate_root_bytes(&Binary::from(vec![1, 2, 3, 4])).unwrap_err();
        assert!(err.to_string().contains("32 bytes"));
    }

    #[test]
    fn reject_unknown_hash_domain() {
        let err = DistroHashDomain::parse("blake3-v9").unwrap_err();
        assert!(err.to_string().contains("unknown distro_hash_domain"));
        assert!(err.to_string().contains("blake3-v9"));
    }

    #[test]
    fn parse_known_domains() {
        assert_eq!(
            DistroHashDomain::parse("poseidon-v1").unwrap(),
            DistroHashDomain::PoseidonV1
        );
        assert_eq!(
            DistroHashDomain::parse("sinsemilla-legacy").unwrap(),
            DistroHashDomain::SinsemillaLegacy
        );
    }

    #[test]
    fn additive_rejects_sinsemilla_legacy() {
        let mut reg =
            DistroRegistry::new_with_genesis(root32(1), DistroHashDomain::PoseidonV1, None)
                .unwrap();
        let err = reg
            .register_root(root32(2), DistroHashDomain::SinsemillaLegacy, None)
            .unwrap_err();
        assert!(err.to_string().contains("poseidon-v1"));
    }

    #[test]
    fn register_second_root_additive() {
        let mut reg = DistroRegistry::new_with_genesis(
            root32(0xAA),
            DistroHashDomain::PoseidonV1,
            Some("drop-0".into()),
        )
        .unwrap();
        let second = reg
            .register_root(
                root32(0xBB),
                DistroHashDomain::PoseidonV1,
                Some("drop-1".into()),
            )
            .unwrap();
        assert_eq!(second.root_id, 1);
        assert_eq!(reg.roots.len(), 2);
        assert_eq!(reg.require_root(0).unwrap().root, root32(0xAA));
        assert_eq!(reg.require_root(1).unwrap().root, root32(0xBB));
    }

    #[test]
    fn double_nullifier_rejected() {
        let mut reg =
            DistroRegistry::new_with_genesis(root32(1), DistroHashDomain::PoseidonV1, None)
                .unwrap();
        reg.spend_nullifier(0, "deadbeef").unwrap();
        let err = reg.spend_nullifier(0, "deadbeef").unwrap_err();
        assert!(err.to_string().contains("nullifier already exists"));
    }

    #[test]
    fn same_nullifier_different_roots_allowed() {
        // Domain separation: set i and set j are independent claim scopes.
        let mut reg =
            DistroRegistry::new_with_genesis(root32(1), DistroHashDomain::PoseidonV1, None)
                .unwrap();
        reg.register_root(root32(2), DistroHashDomain::PoseidonV1, None)
            .unwrap();
        reg.spend_nullifier(0, "aabbcc").unwrap();
        reg.spend_nullifier(1, "aabbcc").unwrap();
    }

    #[test]
    fn claim_unregistered_root_rejected() {
        let reg =
            DistroRegistry::new_with_genesis(root32(1), DistroHashDomain::PoseidonV1, None)
                .unwrap();
        let err = reg.require_root(99).unwrap_err();
        assert!(err.to_string().contains("unregistered eligibility root_id"));
    }

    #[test]
    fn storage_register_and_require() {
        let mut storage = MockStorage::new();
        register_genesis_root(
            &mut storage,
            root32(0x11),
            DistroHashDomain::PoseidonV1,
            None,
        )
        .unwrap();
        let e2 = register_eligibility_root(
            &mut storage,
            root32(0x22),
            DistroHashDomain::PoseidonV1,
            Some("batch-2".into()),
        )
        .unwrap();
        assert_eq!(e2.root_id, 1);
        assert_eq!(require_root(&storage, 0).unwrap().root, root32(0x11));
        assert_eq!(require_root(&storage, 1).unwrap().label.as_deref(), Some("batch-2"));
        let err = require_root(&storage, 7).unwrap_err();
        assert!(err.to_string().contains("unregistered"));
    }

    #[test]
    fn assert_claim_root_anchor_mismatch() {
        let mut storage = MockStorage::new();
        register_genesis_root(
            &mut storage,
            root32(0x11),
            DistroHashDomain::PoseidonV1,
            None,
        )
        .unwrap();
        let bad = root32(0xFF);
        let err = assert_claim_root(&storage, 0, Some(&bad)).unwrap_err();
        assert!(err.to_string().contains("does not match"));
    }

    #[test]
    fn assert_claim_root_rejects_empty_anchor() {
        let mut storage = MockStorage::new();
        register_genesis_root(
            &mut storage,
            root32(0x11),
            DistroHashDomain::PoseidonV1,
            None,
        )
        .unwrap();
        let err = assert_claim_root(&storage, 0, Some(&Binary::default())).unwrap_err();
        assert!(err.to_string().contains("empty"));
        let err = assert_claim_root(&storage, 0, None).unwrap_err();
        assert!(err.to_string().contains("missing anchor"));
    }

    #[test]
    fn poseidon_domain_policy_for_claims() {
        let entry = EligibilityRootEntry {
            root_id: 0,
            root: root32(0x11),
            domain: DistroHashDomain::PoseidonV1,
            label: None,
        };
        assert!(assert_claim_domain_poseidon_v1(&entry).is_ok());
        let legacy = EligibilityRootEntry {
            domain: DistroHashDomain::SinsemillaLegacy,
            ..entry.clone()
        };
        let err = assert_claim_domain_poseidon_v1(&legacy).unwrap_err();
        assert!(err.to_string().contains("poseidon-v1"));
    }

    #[test]
    fn multi_claim_batch_nullifier_domain_sep() {
        let mut reg =
            DistroRegistry::new_with_genesis(root32(1), DistroHashDomain::PoseidonV1, None)
                .unwrap();
        reg.register_root(root32(2), DistroHashDomain::PoseidonV1, None)
            .unwrap();
        reg.spend_nullifier(0, "ab").unwrap();
        reg.spend_nullifier(1, "ab").unwrap(); // ok: different root scope
        let err = reg.spend_nullifier(0, "ab").unwrap_err();
        assert!(err.to_string().contains("already"));
    }

    #[test]
    fn poseidon_root_e2e_claim_path() {
        // Registered Poseidon root bytes must equal claim anchor for process path.
        let mut storage = MockStorage::new();
        let root = root32(0xA5);
        register_genesis_root(
            &mut storage,
            root.clone(),
            DistroHashDomain::PoseidonV1,
            Some("e2e".into()),
        )
        .unwrap();
        let entry = assert_claim_root(&storage, 0, Some(&root)).unwrap();
        assert_eq!(entry.domain, DistroHashDomain::PoseidonV1);
        assert_claim_domain_poseidon_v1(&entry).unwrap();
        assert_eq!(entry.root, root);
    }
}
