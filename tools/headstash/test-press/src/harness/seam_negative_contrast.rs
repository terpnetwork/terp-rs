//! **Negative-contrast** scenarios for private-DEX / bridge / multi-cycle goals.
//!
//! Product elevation is incomplete without objects that **must fail closed**.
//! Each case names the **goal it contrasts** and asserts the adversarial path
//! is rejected (or leaves no silent credit).
//!
//! SSOT: `DESIGN-HARNESS-NEGATIVE-CONTRAST-2026-07-22.md`

use terp_seams::dex::{
    apply_egress_burn, apply_swap, authorize_escrow_release, build_egress_burn_from_settle,
    check_oracle_bound, oracle_mint_note, oracle_update_reserves, quote_exact_in,
    settle_opening_from_note_fields, AssetId, DestKind, EgressBurnEvidenceV0, NoteIn, OracleBoundParams,
    OracleMid, Pool, PoolStatus, SeamError, SeamState, SwapPublic, ThresholdEscrowAuth,
    MODE_FROST_ESCROW_RELEASE, MODE_THRESHOLD_ESCROW_RELEASE,
};
use terp_seams::dex::{
    authorize_create_pool_reserves, mint_lp_zec_note, DualHomePrepV0, EscrowLiabilityV0,
    LpSeedReceiptV0, PoolSeedPolicy, LP_SEED_SOURCE_BRIDGED_NOTES, LP_SEED_SOURCE_MAGIC_LAB,
};
use sha2::{Digest, Sha256};

use super::account_curation::{curate_lab_accounts, AccountRole, ChainKind};
use super::seam_multi_cycle::{MultiCycleWorld, SeamAsset, CYCLE_GAMMA, CYCLE_GAMMA_DEN};

// ---------------------------------------------------------------------------
// Catalog: contrast id → product goal contrasted
// ---------------------------------------------------------------------------

/// Machine-readable contrast cases for docs / CI matrices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContrastCase {
    pub id: &'static str,
    /// Positive goal this case protects.
    pub goal: &'static str,
    /// Adversarial object / action.
    pub adversarial: &'static str,
    /// Expected fail-closed outcome.
    pub expect: &'static str,
}

/// Normative negative-contrast catalog (extend, do not shrink without decision).
pub const CONTRAST_CATALOG: &[ContrastCase] = &[
    ContrastCase {
        id: "NC-ORACLE-MINT",
        goal: "oracle bound_only — never mints balances",
        adversarial: "oracle_mint_note / oracle_update_reserves",
        expect: "ErrOracleDisabledMint",
    },
    ContrastCase {
        id: "NC-DOUBLE-SPEND",
        goal: "one-shot pool nullifiers",
        adversarial: "second apply_swap with same nullifier",
        expect: "ErrNullifierExists",
    },
    ContrastCase {
        id: "NC-WRONG-ASSET",
        goal: "pool leg orientation",
        adversarial: "note asset ≠ pool asset_in",
        expect: "ErrWrongAsset",
    },
    ContrastCase {
        id: "NC-MIN-OUT",
        goal: "slippage floor fail-closed",
        adversarial: "min_out above curve",
        expect: "ErrMinOut",
    },
    ContrastCase {
        id: "NC-POOL-PAUSED",
        goal: "paused pool rejects trade",
        adversarial: "apply_swap on Paused pool",
        expect: "ErrPoolPaused",
    },
    ContrastCase {
        id: "NC-INSUFFICIENT-NOTE",
        goal: "conservation of note value",
        adversarial: "delta_in > note sum",
        expect: "ErrBadAmount",
    },
    ContrastCase {
        id: "NC-ORACLE-STALE",
        goal: "stale mid rejects swap when required",
        adversarial: "oracle mid older than max_age",
        expect: "ErrOracleStale",
    },
    ContrastCase {
        id: "NC-EGRESS-DEST",
        goal: "single dest seal end-to-end",
        adversarial: "burn dest ≠ sealed dest",
        expect: "ErrDestMismatch",
    },
    ContrastCase {
        id: "NC-EGRESS-DOUBLE",
        goal: "one-shot egress nullifier",
        adversarial: "second apply_egress_burn same ν",
        expect: "ErrNullifierExists",
    },
    ContrastCase {
        id: "NC-RELEASE-NO-BURN",
        goal: "Option D burn before ZEC release",
        adversarial: "authorize_escrow_release empty burn",
        expect: "MissingBurn",
    },
    ContrastCase {
        id: "NC-RELEASE-WRONG-DEST",
        goal: "release only to sealed dest",
        adversarial: "authorize with wrong sealed dest",
        expect: "DestMismatch",
    },
    ContrastCase {
        id: "NC-RELEASE-BAD-AUTH",
        goal: "committee/FROST auth required",
        adversarial: "empty combined_sig",
        expect: "BadThresholdAuth",
    },
    ContrastCase {
        id: "NC-MAGIC-POOL",
        goal: "bridged_lp pool seed — no invent R",
        adversarial: "CreatePool magic under BridgedLp policy",
        expect: "policy reject",
    },
    ContrastCase {
        id: "NC-ESCROW-UNDERFUND",
        goal: "ZEC liability ≤ escrow",
        adversarial: "mint_lp_zec with empty escrow",
        expect: "ErrEscrowUnderfunded",
    },
    ContrastCase {
        id: "NC-NO-NOTE",
        goal: "swap requires prior mint/holding",
        adversarial: "swap without mint_note",
        expect: "no unspent note",
    },
    ContrastCase {
        id: "NC-EGRESS-NO-ZEC",
        goal: "egress requires ZEC SEAM",
        adversarial: "egress without ZEC note",
        expect: "no ZEC note",
    },
    ContrastCase {
        id: "NC-FUNDER-MODE",
        goal: "inventory/host-pay not product",
        adversarial: "lab_inventory_pay / ops_funder labels",
        expect: "reject_funder_only_as_product true",
    },
    ContrastCase {
        id: "NC-MISSING-ACCOUNT",
        goal: "curated multi-chain accounts required",
        adversarial: "require unseeded role",
        expect: "missing curated account",
    },
];

pub fn catalog_ids() -> Vec<&'static str> {
    CONTRAST_CATALOG.iter().map(|c| c.id).collect()
}

// ---------------------------------------------------------------------------
// Pure-host helpers
// ---------------------------------------------------------------------------

fn active_pool(a: AssetId, b: AssetId) -> Pool {
    Pool {
        pool_id: 99,
        asset_a: a,
        asset_b: b,
        r_a: 1_000_000,
        r_b: 2_000_000,
        gamma: CYCLE_GAMMA,
        gamma_den: CYCLE_GAMMA_DEN,
        status: PoolStatus::Active,
    }
}

fn nf_u64(tag: u8) -> u64 {
    let mut h = Sha256::new();
    h.update(b"nc-nf");
    h.update([tag]);
    let d = h.finalize();
    u64::from_le_bytes(d[0..8].try_into().unwrap())
}

// ---------------------------------------------------------------------------
// Tests — each maps to a catalog id
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use terp_seams::dex::reject_funder_only_as_product;
    use threshold_committee::{lab_sign_with_threshold, LabCommittee};

    #[test]
    fn catalog_covers_core_product_goals() {
        let ids = catalog_ids();
        assert!(ids.contains(&"NC-ORACLE-MINT"));
        assert!(ids.contains(&"NC-DOUBLE-SPEND"));
        assert!(ids.contains(&"NC-RELEASE-NO-BURN"));
        assert!(ids.contains(&"NC-MAGIC-POOL"));
        assert!(ids.contains(&"NC-FUNDER-MODE"));
        assert!(CONTRAST_CATALOG.len() >= 15);
    }

    #[test]
    fn nc_oracle_mint() {
        let mid = OracleMid {
            pair_key: "x".into(),
            mid: 1,
            observed_height: 1,
        };
        assert_eq!(
            oracle_mint_note(&mid, AssetId::Hub, 1),
            Err(SeamError::ErrOracleDisabledMint)
        );
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        assert_eq!(
            oracle_update_reserves(&mid, &mut pool),
            Err(SeamError::ErrOracleDisabledMint)
        );
    }

    #[test]
    fn nc_double_spend_nullifier() {
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        let mut state = SeamState::default();
        let nu = nf_u64(1);
        let notes = [NoteIn {
            asset_id: AssetId::Hub,
            value: 10_000,
            nullifier: nu,
        }];
        let public = SwapPublic {
            asset_in: AssetId::Hub,
            asset_out: AssetId::AssetB,
            delta_in: 10_000,
            min_out: 1,
            nullifiers: vec![nu],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };
        apply_swap(&mut pool, &mut state, &notes, &public).unwrap();
        let err = apply_swap(&mut pool, &mut state, &notes, &public).unwrap_err();
        assert_eq!(err, SeamError::ErrNullifierExists);
    }

    #[test]
    fn nc_wrong_asset() {
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        let mut state = SeamState::default();
        let nu = nf_u64(2);
        let notes = [NoteIn {
            asset_id: AssetId::AssetB, // wrong — pool expects Hub in
            value: 10_000,
            nullifier: nu,
        }];
        let public = SwapPublic {
            asset_in: AssetId::Hub,
            asset_out: AssetId::AssetB,
            delta_in: 10_000,
            min_out: 1,
            nullifiers: vec![nu],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };
        assert_eq!(
            apply_swap(&mut pool, &mut state, &notes, &public),
            Err(SeamError::ErrWrongAsset)
        );
    }

    #[test]
    fn nc_min_out() {
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        let mut state = SeamState::default();
        let nu = nf_u64(3);
        let delta_in = 10_000u128;
        let fair = quote_exact_in(pool.r_a, pool.r_b, delta_in, pool.gamma, pool.gamma_den).unwrap();
        let notes = [NoteIn {
            asset_id: AssetId::Hub,
            value: delta_in,
            nullifier: nu,
        }];
        let public = SwapPublic {
            asset_in: AssetId::Hub,
            asset_out: AssetId::AssetB,
            delta_in,
            min_out: fair + 1,
            nullifiers: vec![nu],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };
        assert_eq!(
            apply_swap(&mut pool, &mut state, &notes, &public),
            Err(SeamError::ErrMinOut)
        );
    }

    #[test]
    fn nc_pool_paused() {
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        pool.status = PoolStatus::Paused;
        let mut state = SeamState::default();
        let nu = nf_u64(4);
        let notes = [NoteIn {
            asset_id: AssetId::Hub,
            value: 1000,
            nullifier: nu,
        }];
        let public = SwapPublic {
            asset_in: AssetId::Hub,
            asset_out: AssetId::AssetB,
            delta_in: 1000,
            min_out: 1,
            nullifiers: vec![nu],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };
        assert_eq!(
            apply_swap(&mut pool, &mut state, &notes, &public),
            Err(SeamError::ErrPoolPaused)
        );
    }

    #[test]
    fn nc_insufficient_note_value() {
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        let mut state = SeamState::default();
        let nu = nf_u64(5);
        let notes = [NoteIn {
            asset_id: AssetId::Hub,
            value: 100,
            nullifier: nu,
        }];
        let public = SwapPublic {
            asset_in: AssetId::Hub,
            asset_out: AssetId::AssetB,
            delta_in: 10_000,
            min_out: 1,
            nullifiers: vec![nu],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };
        assert_eq!(
            apply_swap(&mut pool, &mut state, &notes, &public),
            Err(SeamError::ErrBadAmount)
        );
    }

    #[test]
    fn nc_oracle_stale() {
        let mid = OracleMid {
            pair_key: "HUB-B".into(),
            mid: 1_000_000,
            observed_height: 1,
        };
        let params = OracleBoundParams {
            max_age_blocks: 5,
            max_slippage_bps: 10_000,
            require_oracle: true,
        };
        // now=100 → age 99 > 5
        let err = check_oracle_bound(Some(&mid), &params, 100, 1000, 1000).unwrap_err();
        assert_eq!(err, SeamError::ErrOracleStale);
    }

    #[test]
    fn nc_egress_dest_mismatch() {
        let dest = [0xD1u8; 32];
        let wrong = [0xEEu8; 32];
        let opening = settle_opening_from_note_fields(
            {
                let mut a = [0u8; 32];
                a[0] = b'Z';
                a
            },
            1000,
            dest,
            [0xB2; 32],
            DestKind::Transparent,
            [0x11; 32],
            Some(1),
            0,
        );
        let burn = build_egress_burn_from_settle(&opening).unwrap();
        let mut st = terp_seams::dex::EgressSeamState::default();
        let err = apply_egress_burn(&mut st, &burn.public, &burn.witness, Some(&wrong)).unwrap_err();
        assert!(matches!(err, terp_seams::dex::EgressError::ErrDestMismatch));
    }

    #[test]
    fn nc_egress_double_nullifier() {
        let dest = [0xD2u8; 32];
        let opening = settle_opening_from_note_fields(
            {
                let mut a = [0u8; 32];
                a[0] = b'Z';
                a
            },
            2000,
            dest,
            [0xB3; 32],
            DestKind::Transparent,
            [0x11; 32],
            Some(1),
            0,
        );
        let burn = build_egress_burn_from_settle(&opening).unwrap();
        let mut st = terp_seams::dex::EgressSeamState::default();
        apply_egress_burn(&mut st, &burn.public, &burn.witness, Some(&dest)).unwrap();
        let err = apply_egress_burn(&mut st, &burn.public, &burn.witness, Some(&dest)).unwrap_err();
        assert!(matches!(err, terp_seams::dex::EgressError::ErrNullifierExists));
    }

    #[test]
    fn nc_release_no_burn_wrong_dest_bad_auth() {
        let dest = [0xAAu8; 32];
        let opening = settle_opening_from_note_fields(
            {
                let mut a = [0u8; 32];
                a[0] = b'Z';
                a
            },
            500,
            dest,
            [0xB4; 32],
            DestKind::Transparent,
            [0x11; 32],
            Some(1),
            0,
        );
        let burn = build_egress_burn_from_settle(&opening).unwrap();
        let evidence = EgressBurnEvidenceV0 {
            terp_tx_hash: None,
            burn: burn.public.clone(),
            proof_mode: "mock_verify_lab".into(),
            settle_receipt_ref: None,
        };
        let committee = LabCommittee::generate(2, 3).unwrap();
        let digest = threshold_committee::escrow_release_digest(
            &evidence.burn.nullifier,
            &evidence.burn.dest_commitment,
            evidence.burn.value,
            &evidence.burn.asset_id,
        );
        let combined = lab_sign_with_threshold(&committee, &digest).unwrap();
        let auth = ThresholdEscrowAuth {
            committee_pk: committee.roster.encode_public().unwrap(),
            combined_sig: combined.encode(),
        };
        // Happy sanity
        let ok = authorize_escrow_release(&evidence, &dest, &auth).unwrap();
        assert!(
            ok.mode == MODE_THRESHOLD_ESCROW_RELEASE || ok.mode == MODE_FROST_ESCROW_RELEASE
        );

        // No burn
        let mut empty = evidence.clone();
        empty.burn.nullifier = [0u8; 32];
        assert!(matches!(
            authorize_escrow_release(&empty, &dest, &auth),
            Err(terp_seams::dex::EscrowReleaseError::MissingBurn)
        ));
        // Wrong dest
        assert!(matches!(
            authorize_escrow_release(&evidence, &[0xEE; 32], &auth),
            Err(terp_seams::dex::EscrowReleaseError::DestMismatch)
        ));
        // Bad auth
        let bad = ThresholdEscrowAuth {
            committee_pk: auth.committee_pk.clone(),
            combined_sig: vec![],
        };
        assert!(matches!(
            authorize_escrow_release(&evidence, &dest, &bad),
            Err(terp_seams::dex::EscrowReleaseError::BadThresholdAuth)
        ));
    }

    #[test]
    fn nc_magic_pool_and_escrow_underfund() {
        // Magic reserves under bridged_lp without bridged receipt
        let err = authorize_create_pool_reserves(
            PoolSeedPolicy::BridgedLp,
            1_000_000,
            2_000_000,
            None,
        );
        assert!(err.is_err());

        // Receipt source magic under bridged policy
        let rec = LpSeedReceiptV0 {
            btc_mint_nf_hex: "00".into(),
            zec_mint_nf_hex: "00".into(),
            lp_btc_value: 1,
            lp_zec_value: 1,
            r_btc: 1,
            r_zec: 1,
            source: LP_SEED_SOURCE_MAGIC_LAB.into(),
            pool_id: 1,
            lp_nullifiers_hex: vec![],
            conservation_btc: "x".into(),
            conservation_zec: "x".into(),
            proof_mode: "mock_verify_lab".into(),
            policy: "bridged_lp".into(),
        };
        let err2 = authorize_create_pool_reserves(PoolSeedPolicy::BridgedLp, 1, 1, Some(&rec));
        assert!(err2.is_err());
        // Bridged source with matching R OK
        let rec_ok = LpSeedReceiptV0 {
            source: LP_SEED_SOURCE_BRIDGED_NOTES.into(),
            ..rec
        };
        authorize_create_pool_reserves(PoolSeedPolicy::BridgedLp, 1, 1, Some(&rec_ok)).unwrap();

        // Escrow underfund LP ZEC mint
        let mut liab = EscrowLiabilityV0::new(1); // only 1 zat escrow
        let asset = [0x5Au8; 32];
        assert!(mint_lp_zec_note(asset, 1000, [1u8; 32], [2u8; 32], &mut liab, "mock_verify_lab")
            .is_err());
        let _ = DualHomePrepV0::omni_fixture(); // still part of dual-home SSOT surface
    }

    #[test]
    fn nc_multi_cycle_missing_note_and_egress() {
        let mut w = MultiCycleWorld::seed_lab();
        // Swap without mint
        let err = w.swap_cycle(AccountRole::User, 1, SeamAsset::Btc, SeamAsset::Zec, 100);
        assert!(err.unwrap_err().contains("no unspent"));
        // Egress without ZEC note
        let err2 = w.egress_zec_to_user(AccountRole::User);
        assert!(err2.unwrap_err().contains("no ZEC"));
    }

    #[test]
    fn nc_funder_modes_and_missing_account() {
        assert!(reject_funder_only_as_product("lab_inventory_pay"));
        assert!(reject_funder_only_as_product("lab_inventory_pay_simulated"));
        assert!(reject_funder_only_as_product("ops_funder"));
        assert!(reject_funder_only_as_product("host_pay"));
        assert!(!reject_funder_only_as_product(MODE_THRESHOLD_ESCROW_RELEASE));
        assert!(!reject_funder_only_as_product("frost_escrow_spend"));

        let reg = curate_lab_accounts();
        // Bitcoin has no DexAdmin
        assert!(reg
            .require(ChainKind::Bitcoin, AccountRole::DexAdmin)
            .unwrap_err()
            .contains("missing curated account"));
    }

    #[test]
    fn nc_insufficient_reserve_empty_pool() {
        let mut pool = active_pool(AssetId::Hub, AssetId::AssetB);
        pool.r_b = 1; // nearly empty out leg
        let mut state = SeamState::default();
        let nu = nf_u64(9);
        // Huge trade that would empty / fail reserve
        let notes = [NoteIn {
            asset_id: AssetId::Hub,
            value: 10_000_000,
            nullifier: nu,
        }];
        let public = SwapPublic {
            asset_in: AssetId::Hub,
            asset_out: AssetId::AssetB,
            delta_in: 10_000_000,
            min_out: 1,
            nullifiers: vec![nu],
            oracle_mid: None,
            oracle_params: None,
            now_height: 1,
        };
        let err = apply_swap(&mut pool, &mut state, &notes, &public);
        // Either insufficient reserve or bad amount — must not succeed
        assert!(err.is_err());
    }
}
