//! L0 harness helpers for private-bridge / Headstash compose.
//!
//! Always available (no `interface` feature). Pure seams + DEMO-PATH claim
//! fixture schema + Domain B bridge mint packet fixture. Suite methods under
//! `suites::private_bridge` re-export these.
//!
//! L1 Binary world builders (`bridge_l1`) require the `interface` feature.
//!
//! Product pure E2E (burn→SEAM→swap apply): [`compose_l0`] → `compose_seams`.
//!
//! SSOT: `docs/plans/spectrum/E2E-HARNESS-PLAN.md`

/// Multi-chain account curation + tokenfactory plans (BTC/ZEC/Terp).
pub mod account_curation;
pub mod bridge_l0;
#[cfg(feature = "interface")]
pub mod bridge_l1;
pub mod bridge_mint_fixture;
/// Cash App → ZEC corridor workflow W0–W7 (Simulated backend default).
pub mod cashapp_zec_corridor;
pub mod claim_fixture;
/// G1 continuous deposit → BridgeMintClaim builder (provisional ν).
pub mod claim_from_deposit;
pub mod compose_l0;
/// Option D continuous corridor: settle → egress burn → lab ZEC pay (HARNESS-D).
pub mod egress_d;
/// Option D lab Zcash inventory pay gated on Terp burn evidence.
pub mod lab_pay_zec;
/// Mint evidence + settle receipt artifacts (G3).
pub mod mint_evidence;
/// Product call site: encrypt SEAM → PUT / local HeadstashStore-shaped store.
pub mod note_persist_client;
/// Headstash note envelope persist L0 (opaque ciphertext; hash-market path SSOT).
pub mod note_persist_l0;
/// Multi-net dual-home prep (WAVE-2 MN-DUAL-HOME): bitcoind LP+user + Zakura escrow.
pub mod omni_dual_home;
/// OmniBridge-shaped LP bootstrap (GUIDE Phases 1–3): dual-home → mint LP → seed pool.
pub mod omni_lp_seed;
/// Multi-cycle private-DEX + bridge seam routes under curated accounts.
pub mod seam_multi_cycle;
/// Negative-contrast (adversarial / fail-closed) catalog + tests.
pub mod seam_negative_contrast;
/// Pure → CW swap statement map + lab settle handoff (G3).
pub mod swap_statement_cw;
/// Local Zakura RPC + dest_owner_binding helper (D6).
pub mod zakura_local;

pub use account_curation::{
    AccountRegistry, AccountRole, ChainKind, LAB_BTC_LP_SATS, LAB_BTC_USER_SATS, LAB_TF_MINT,
    LAB_ZEC_ESCROW_ZATS, SeededAccount, TokenFactoryDenomPlan, asset_id_from_denom,
    credit_tokenfactory_holding, curate_lab_accounts, lab_owner_binding,
};
pub use bridge_l0::{
    L0Error, assert_double_mint_reject, assert_h1_spent_only_reject,
    assert_policy_bridge_mint_happy, compose_bridge_mint_to_seam_bytes,
};
#[cfg(feature = "interface")]
pub use bridge_l1::BridgeL1World;
pub use bridge_mint_fixture::{
    BridgeMintFixtureClaim, BridgeMintFixtureDoc, BridgeMintFixtureError,
    assert_fixture_matches_hinge_happy, authorize_from_bridge_mint_fixture,
    build_happy_bridge_mint_fixture, default_golden_bridge_mint_path, load_bridge_mint_fixture,
    load_or_build_happy_bridge_mint_fixture,
};
pub use cashapp_zec_corridor::{
    CorridorAssetBackend, CorridorReceiptV0, CorridorScenario, CorridorWorkflowOutcome,
    DepositIntentV0, LcMode, intent_allows_mint, intent_allows_timeout_recovery,
    run_cashapp_zec_corridor_w0_w7,
};
#[cfg(feature = "l0-seams")]
pub use cashapp_zec_corridor::{
    put_and_recover_after_mint, seam_note_from_mint_openings, try_persist_mint_note,
};
pub use claim_fixture::{
    CLAIM_INSTANCE_BYTES_LEN, ClaimFixture, ClaimFixtureInstance, ClaimFixturePartialNote,
    build_policy_claim_fixture, load_claim_fixture,
};
pub use claim_from_deposit::{
    BridgeMintClaimPure, ClaimFromDepositError, CorridorLabMintPolicy, DepositObservationView,
    DepositWatchView, TERP_BTC_DEPOSIT_NU_V0, claim_from_deposit_watch,
    corridor_allow_happy_fixture, deposit_nullifier_v0, lab_snapshot_for_policy,
    lab_terp_asset_for_policy, parse_txid32, try_claim_from_env,
};
pub use compose_l0::{
    CorridorSwapSpendParams, MintSpendEvidence, SwapStatementPublicView,
    assert_product_path_burn_to_swap_oracle_zec, assert_product_path_burn_to_swap_sketch,
    mint_evidence_to_swap_action, seam_note_out_to_swap_action, seam_note_out_to_swap_openings,
    swap_action_public_to_statement,
};
pub use egress_d::{
    BURN_SURFACE_CW_BRIDGE_EGRESS, BURN_SURFACE_PURE_RECORD_LAB, ENV_CORRIDOR_ZEC_EGRESS_D,
    EgressBurnEvidenceJson, EgressBurnPublicJson, EgressDError, EgressDStageInputs,
    EgressDStageOutcome, ZecEgressReceiptJson, apply_pure_egress_burn_labeled,
    assert_lab_degrade_labels, product_celebrate_forbidden, refuse_pay_without_evidence,
    run_option_d_after_settle, sealed_dest_for_option_d, settle_opening_from_handoff,
    zec_egress_d_enabled,
};
#[cfg(feature = "interface")]
pub use egress_d::{cw_egress_statement_from_opening, lab_mock_egress_proof};
pub use lab_pay_zec::{
    AUTH_SOURCE_HASHMERCHANT_LIVE, AUTH_SOURCE_IN_PROCESS_LAB, COMMITTEE_CRYPTO_LAB_T_OF_N,
    ENV_CORRIDOR_ALLOW_LAB_INVENTORY_RESIDUAL, ENV_CORRIDOR_HASH_MARKET_URL, ENV_CORRIDOR_PROFILE,
    ENV_CORRIDOR_REQUIRE_LIVE_THRESHOLD, ENV_CORRIDOR_ZEC_RELEASE, ENV_HASH_MARKET_URL,
    ESCROW_SIGN_PATH_P_LAB, LAB_SIMULATED_TXID_PREFIX, LabPayError, MODE_LAB_INVENTORY_PAY,
    MODE_LAB_INVENTORY_PAY_SIMULATED, MODE_LC_MINT, OPEN_CONFIRM_RPC_OK, OPEN_CONFIRM_SKIP_NO_RPC,
    OPEN_CONFIRM_SKIP_NO_WALLET, OPEN_CONFIRM_SKIP_VALIDATE_ONLY, OpenConfirmResult,
    THRESHOLD_SIMULATED_TXID_PREFIX, ThresholdAuthMeta, ZecReleasePolicy,
    allow_lab_inventory_residual, assert_product_release_mode,
    authorize_threshold_escrow_for_evidence, confirm_open_at_sealed_dest,
    confirm_open_at_sealed_dest_with_cfg, corridor_profile, dest_kind_from_display,
    egress_burn_evidence_path, hash_market_base_url, in_process_threshold_escrow_auth,
    is_product_profile, is_real_zec_txid, lab_committee_params, lab_evidence_for_sealed,
    lab_pay_zec_after_burn, lab_pay_zec_after_burn_with_cfg, lab_threshold_escrow_auth,
    live_hashmerchant_custody_ready, live_threshold_escrow_auth, peek_last_threshold_auth_meta,
    require_live_threshold, synthetic_lab_txid, take_last_threshold_auth_meta, try_getbalance_zat,
    try_getreceivedbyaddress_zat, validate_lab_pay_gates, wallet_rpc_available,
    write_zec_egress_receipt_json, zec_egress_receipt_path, zec_release_policy,
};
pub use mint_evidence::{
    MintEvidenceError, MintEvidenceV0, PROOF_MODE_MOCK_VERIFY_LAB, PROOF_MODE_ZK_API,
    SettleReceiptV0, mint_evidence_from_claim_fields, mint_evidence_path, settle_receipt_path,
};
pub use note_persist_client::{NotePersistReceipt, NotesPersistConfig};
#[cfg(feature = "l0-seams")]
pub use note_persist_client::{
    get_and_decrypt_note, l2_smoke_put_get_decrypt, put_and_film_recover_after_mint,
    put_note_after_mint, recover_note, recover_note_film,
};
#[cfg(all(feature = "l0-seams", feature = "note-http"))]
pub use note_persist_client::{list_note_addrs, recover_note_via_pir};
pub use note_persist_l0::{
    MiniNoteStore, NoteEnvelope, SEAM_NOTE_OUT_LEN, assert_bridge_note_persist_l0,
    assert_compose_seam_envelope_meta, bridge_note_addr_from_cm_public,
};
pub use omni_dual_home::{
    DualHomeMultiNetPrepV0, ENV_CORRIDOR_DUAL_HOME_PREP_PATH, ENV_CORRIDOR_ESCROW_ADDR,
    ENV_CORRIDOR_OMNI_REQUIRE_PRODUCT_GREEN, EscrowObserveV0, OmniDualHomeError,
    PrefundEscrowResult, dual_home_prep_path, dual_home_supports_multinet_lp, fixture_amounts,
    load_dual_home_prep, load_dual_home_prep_or_fixture, observe_escrow_balance,
    require_omni_product_green, resolve_dual_home_for_bootstrap, try_prefund_escrow_wallet,
    write_dual_home_run_pack,
};
pub use omni_lp_seed::{
    DualHomePrepJson, ENV_CORRIDOR_POOL_SEED, LpSeedInnerJson, LpSeedReceiptJson,
    OmniLiquidityBootstrap, OmniLpError, POOL_SEED_BRIDGED_LP, POOL_SEED_MAGIC_LAB,
    bootstrap_omni_liquidity, bootstrap_omni_liquidity_from_multinet,
    bootstrap_omni_liquidity_with, bridged_notes_source_label, corridor_pool_seed_policy,
    corridor_profile_is_omni, lp_mint_log_line, lp_seed_receipt_path, magic_lab_source_label,
    refuse_magic_pool_if_bridged_policy, require_bridged_lp_seed, resolve_pool_reserves,
};
pub use seam_multi_cycle::{
    CYCLE_GAMMA, CYCLE_GAMMA_DEN, MultiCycleWorld, SeamAsset, assert_oracle_cannot_mint,
    route_corridor_btc_to_zec, route_multi_cycle_rebalance, route_tokenfactory_into_corridor,
};
pub use seam_negative_contrast::{CONTRAST_CATALOG, ContrastCase, catalog_ids};
pub use swap_statement_cw::{
    LAB_MOCK_SWAP_PROOF, ProofModeLabel, SwapHandoffError, SwapSpendHandoffV0,
    assert_mint_handoff_continuous, build_swap_spend_handoff_from_mint,
    build_swap_spend_handoff_from_mint_with_reserves, lab_mock_swap_proof,
    quote_delta_out_with_reserves, seam_sketch_from_mint_evidence, swap_action_public_to_cw,
};
pub use zakura_local::{
    DEFAULT_ZAKURA_RPC, DEST_BINDING_DOMAIN_PREFIX, DestSealError, GoldenDestBindingDoc,
    GoldenDestError, GoldenDestSample, REGTEST_MINER_DEST, REGTEST_MINER_OWNER_BINDING_HEX,
    SealedDestSource, SealedDestV0, ZAKURA_METRICS_PORT, ZAKURA_P2P_PORT, ZAKURA_RPC_PORT,
    ZakuraDest, ZakuraLocalConfig, assert_dest_binding_equal, assert_golden_dest_binding,
    default_dest_display, default_golden_dest_binding_path, fetch_corridor_dest,
    is_placeholder_owner_binding_hex, load_golden_dest_binding, owner_binding_from_dest_display,
    owner_binding_hex, primary_golden_dest, reject_empty_dest, reject_empty_dest_seal,
    reject_placeholder_binding_hex, require_binding_hex_width, rpc_ready, seal_funded_dest,
    soft_validate_dest_prefix,
};

// Pure Option D types (SSOT private_dex_seams) used by lab pay surface.
pub use terp_seams::dex::{
    DestKind, EgressBurnEvidenceV0, EgressBurnPublic, ThresholdEscrowAuth, ZecEgressReceiptV0,
};
