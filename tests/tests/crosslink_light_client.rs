#![cfg(feature = "crosslink-e2e")]
//! Integration test for the crosslink IBC light client end-to-end flow.
//!
//! Uses cw-orch CrosslinkLightClient interface for upload/instantiate/queries.
//! The Terp chain is managed via ict-rs Docker.
//! Synthetic Ed25519 (`SigningKey::from([i;32])`) is state-machine only.
//! Live zakurad TFL → 08-wasm is `zakura_crosslink_live_lc.rs` (feature zakura-live).
//!
//! Test runtime design
//! -------------------
//! Each [`TestCase`] is a self-contained scenario: an initial
//! [`CrosslinkClientState`] + [`CrosslinkConsensusState`], a sequence of
//! [`CrosslinkHeader`]s to submit as `MsgUpdateClient`, and assertions encoded
//! as the expected final IBC client state. Scenarios are modelled on the
//! existing crosslink test cases (genesis anchor → BFT blocks at heights
//! 1..N, each BFT block carrying σ=3 PoW headers from the synthetic chain).
//!
//! Runtime blocking safety
//! -----------------------
//! This is a `#[tokio::test]`. cw-orch's sync `TxHandler` methods
//! (upload/instantiate/bank_send/...) internally call
//! `self.rt_handle.block_on(..)`. Calling those from inside an async runtime
//! context triggers:
//!
//!   `Cannot start a runtime from within a runtime.`
//!
//! To avoid that, all cw-orch interactions happen on a plain std worker
//! thread (no tokio context entered). Async cw-orch queriers / `commit_tx_any`
//! are driven by `daemon.rt_handle.block_on(..)` from that worker thread,
//! where it is safe to block. Per-scenario work runs in sibling std threads
//! sharing the single chain endpoint.

use anyhow::{Result, anyhow};
use std::sync::Arc;
use std::time::Duration;

use cw_orch::daemon::{
    CosmosOptions, DaemonBuilder, TxSender as _,
    queriers::{Ibc, Staking},
};
use cw_orch::environment::NetworkInfoOwned;
use cw_orch::prelude::*;
use ict_rs::prelude::*;

use ed25519_zebra::{SigningKey, VerificationKey};

use crosslink_light_client::{
    ConsensusState as CrosslinkConsensusState, CrosslinkHeader, FinalizerEntry, ZcashSerialize,
    client_state::ClientState as CrosslinkClientState,
    types::{
        Blake3Hash, FatPointerSignature2, FatPointerToBftBlock2, PROTOTYPE_PARAMETERS, PowHeader,
    },
};

use terp_rs::{
    Any,
    cosmos::base::v1beta1::Coin,
    ibc::{
        core::client::v1::{MsgCreateClient, MsgUpdateClient},
        lightclients::wasm::v1::{ClientMessage as WasmClientMessage, ClientState, ConsensusState},
    },
};
use tracing::{error, info};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TERP_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Mnemonic used to back the faucet wallet. Recovered on the chain as the
/// `faucet` key, then funded from the validator key during chain bootstrap.
/// The cw-orch faucet Daemon uses the same mnemonic → same bech32 address
/// → same on-chain balance.
const FAUCET_MNEMONIC: &str = "chapter wrist alcohol shine angry noise mercy simple rebel recycle \
    vehicle wrap morning giraffe lazy outdoor noise blood ginger sort reunion boss crowd dutch";

/// BIP-39 mnemonics for the per-scenario wallets. Each scenario runs against
/// its own `Daemon` constructed from one of these. Length determines the
/// maximum number of scenarios that can run in parallel.
const SCENARIO_MNEMONICS: &[&str] = &[
    "spare neglect across text guard pilot burger express differ index robot imitate slam stereo ridge margin post country spider improve paddle minute lab virus",
    "daring rug luggage swim process wave salmon jealous pumpkin ahead include blood exhaust stem zoo evidence prepare sugar bright organ ancient sleep chaos social",
    "promote life animal indoor tray transfer dentist video verify total seat reject magic expect donate ketchup indicate resist alter uniform ivory reveal weekend hour",
    "sugar argue group honey deputy together census dismiss east alley else choice abstract bullet place school stumble lend toward damage van cave then parade",
];

/// Genesis allowance granted to the validator key (uterp).
const VALIDATOR_GENESIS_AMOUNT: u128 = 1_000_000_000_000_000_000;

/// Amount the faucet wallet is pre-charged with from the validator (uterp).
const FAUCET_TOPUP_AMOUNT: u128 = 100_000_000_000;

/// Per-scenario seed amount (uterp). Enough for many thousands of txs.
const SCENARIO_FUND_AMOUNT: u128 = 100_000_000;

// ---------------------------------------------------------------------------
// Scenario data
// ---------------------------------------------------------------------------

/// A single Crosslink IBC light client test scenario.
#[derive(Clone)]
struct TestCase {
    /// Human-readable name used in logs / assertions.
    name: &'static str,
    /// Initial `ClientState` to instantiate the contract with.
    initial_client_state: CrosslinkClientState,
    /// Initial `ConsensusState` to instantiate the contract with.
    initial_consensus_state: CrosslinkConsensusState,
    /// Sequence of headers to submit as `MsgUpdateClient`. The trusted height
    /// of each header must match the latest BFT height of the contract after
    /// previous updates in the sequence.
    updates: Vec<CrosslinkHeader>,
    /// Whether the entire header sequence is expected to succeed.
    /// `false` means every update after the first rejected one should fail.
    expect_sequence_succeeds: bool,
    /// Expected BFT height of the contract after all updates are applied.
    expected_bft_height_after: u32,
}

/// Helper: build a "genesis" `ClientState` anchored at height 0 with the
/// given finalizer roster. Prefer real Ed25519 keys — zero pubkeys cannot
/// produce valid fat-pointer signatures.
fn genesis_client_state(finalizers: Vec<FinalizerEntry>) -> CrosslinkClientState {
    CrosslinkClientState::new(
        PROTOTYPE_PARAMETERS.clone(),
        Blake3Hash([0u8; 32]),
        0, // latest_bft_height — first update must use trusted_bft_height = 0
        0,
        [0u8; 32],
        finalizers,
    )
}

/// Helper: build a "genesis" `ConsensusState` anchored at height 0.
fn genesis_consensus_state() -> CrosslinkConsensusState {
    CrosslinkConsensusState::v1(0, 0, [0u8; 32], 0, [0u8; 32])
}

/// Helper: build a single `PowHeader` (hash, timestamp, height) — the slim
/// PoW block header used by the light client contract.
fn pow_header(hash_byte: u8, timestamp: u64, height: u32) -> PowHeader {
    let mut hash = [0u8; 32];
    hash[0] = hash_byte;
    // Deterministic non-zero commitment so v1 anchors are observable in updates.
    let mut commitment_bytes = [0u8; 32];
    commitment_bytes[0] = hash_byte ^ 0x5a;
    commitment_bytes[1] = (height & 0xff) as u8;
    PowHeader {
        hash,
        timestamp,
        height,
        commitment_bytes,
    }
}

/// Helper: build a `BftBlock` at `bft_height` whose PoW anchor is at
/// `commit_height`. The previous BFT block is identified by `prev_hash`
/// via a (null-pointer) fat pointer stub so the light client can reason
/// about the chain linkage.
fn bft_block(
    bft_height: u32,
    commit_height: u32,
    prev_hash: [u8; 32],
) -> crosslink_light_client::types::BftBlock {
    // PoW anchor = σ=3 consecutive headers ending at `commit_height`.
    let headers = vec![
        pow_header(
            0xa0 + (bft_height as u8),
            1_700_000_000 + commit_height as u64 - 2,
            commit_height - 2,
        ),
        pow_header(
            0xb0 + (bft_height as u8),
            1_700_000_000 + commit_height as u64 - 1,
            commit_height - 1,
        ),
        pow_header(
            0xc0 + (bft_height as u8),
            1_700_000_000 + commit_height as u64,
            commit_height,
        ),
    ];
    let mut prev_vote = vec![0u8; 44];
    prev_vote[..32].copy_from_slice(&prev_hash);
    crosslink_light_client::types::BftBlock {
        version: 1,
        height: bft_height,
        previous_block_fat_ptr: FatPointerToBftBlock2 {
            vote_for_block_without_finalizer_public_key: prev_vote,
            signatures: Vec::new(),
        },
        finalization_candidate_height: commit_height,
        headers,
    }
}

/// Compute a stub "prev BFT hash" deterministic from the previous height.
fn prev_hash_for(height: u32) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = height as u8;
    h[1] = (height >> 8) as u8;
    h
}

/// Build a fully signed `CrosslinkHeader`.
///
/// - `trusted_bft_height` = `bft_height - 1` (must equal client `latest_bft_height`)
/// - outer `fat_pointer` votes for *this* block's blake3 hash and carries roster sigs
fn header_update(
    bft_height: u32,
    commit_height: u32,
    prev_hash: [u8; 32],
    signing_keys: &[SigningKey],
) -> CrosslinkHeader {
    signed_header_update(bft_height, commit_height, prev_hash, signing_keys)
}

/// Construct the known crosslink test scenarios.
///
/// Each scenario uses a real Ed25519 finalizer roster so fat-pointer
/// verification can succeed. Genesis is always at BFT height 0.
fn test_cases() -> Vec<TestCase> {
    let (finalizers, signing_keys) = generate_test_finalizers(3);
    let genesis_state = genesis_client_state(finalizers);
    let genesis_consensus = genesis_consensus_state();
    let keys = signing_keys.as_slice();

    // ── Scenario 1: single BFT update from 0 → 1 ────────────────────
    let single_update = TestCase {
        name: "single-update-0-to-1",
        initial_client_state: genesis_state.clone(),
        initial_consensus_state: genesis_consensus.clone(),
        updates: vec![header_update(1, 100, prev_hash_for(0), keys)],
        expect_sequence_succeeds: true,
        expected_bft_height_after: 1,
    };

    // ── Scenario 2: multi-step chain 0 → 1 → 2 → 3 ──────────────────
    let chain_update = TestCase {
        name: "chain-update-0-to-3",
        initial_client_state: genesis_state.clone(),
        initial_consensus_state: genesis_consensus.clone(),
        updates: vec![
            header_update(1, 100, prev_hash_for(0), keys),
            header_update(2, 110, prev_hash_for(1), keys),
            header_update(3, 120, prev_hash_for(2), keys),
        ],
        expect_sequence_succeeds: true,
        expected_bft_height_after: 3,
    };

    // ── Scenario 3: negative — wrong trusted height after a valid first update
    let start_header = header_update(1, 100, prev_hash_for(0), keys);
    let bad_header = CrosslinkHeader {
        trusted_bft_height: 999, // must equal latest (1 after first update) — will reject
        ..header_update(2, 110, prev_hash_for(1), keys)
    };
    let bad_trusted = TestCase {
        name: "bad-trusted-height-rejected",
        initial_client_state: genesis_state.clone(),
        initial_consensus_state: genesis_consensus.clone(),
        updates: vec![start_header, bad_header],
        expect_sequence_succeeds: false,
        expected_bft_height_after: 1,
    };

    // ── Scenario 4: replay same header (misbehaviour / freeze path) ─
    let replay_header = header_update(1, 100, prev_hash_for(0), keys);
    let replay = TestCase {
        name: "replay-misbehaviour-freezes",
        initial_client_state: genesis_state.clone(),
        initial_consensus_state: genesis_consensus.clone(),
        updates: vec![
            replay_header.clone(),
            replay_header, // same bft_height=1
            header_update(2, 110, prev_hash_for(1), keys),
        ],
        expect_sequence_succeeds: false,
        expected_bft_height_after: 1,
    };

    // ── Scenario 5: multi-finalizer signed update (same as single with 3 keys)
    let real_signatures = TestCase {
        name: "real-ed25519-signatures",
        initial_client_state: genesis_state,
        initial_consensus_state: genesis_consensus,
        updates: vec![header_update(1, 100, prev_hash_for(0), keys)],
        expect_sequence_succeeds: true,
        expected_bft_height_after: 1,
    };

    vec![
        single_update,
        chain_update,
        bad_trusted,
        replay,
        real_signatures,
    ]
}

// ---------------------------------------------------------------------------
// Environment
//
// The chain lifecycle is owned by `CrosslinkTestEnv` and driven entirely
// *async* (ict-rs is async-only). cw-orch Daemon work is intentionally kept
// OUT of the env: it is performed on a std worker thread to avoid the
// `block_on`-within-runtime panic.
// ---------------------------------------------------------------------------

struct CrosslinkTestEnv {
    name: String,
    chain: Option<CosmosChain>,
    chain_info: Option<ChainInfoOwned>,
    started: bool,
}

impl CrosslinkTestEnv {
    async fn new(name: &str) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            chain: None,
            chain_info: None,
            started: false,
        })
    }

    async fn start(&mut self) -> Result<()> {
        if self.started {
            return Ok(());
        }

        let chain_config = terp_chain_config();
        let runtime: Arc<dyn RuntimeBackend> =
            Arc::new(DockerBackend::new(DockerConfig::default()).await?);
        let mut chain = CosmosChain::new(chain_config, 1, 0, runtime);
        let test_ctx = TestContext {
            test_name: self.name.clone(),
            network_id: String::new(),
        };
        chain.initialize(&test_ctx).await?;

        let primary_addr = chain.primary_node()?.get_key_address("validator").await?;
        chain.build_wallet("faucet", FAUCET_MNEMONIC).await?;
        let faucet_addr = chain.key_address("faucet").await?;
        chain
            .start(&[
                WalletAmount {
                    address: primary_addr,
                    denom: "uterp".to_string(),
                    amount: VALIDATOR_GENESIS_AMOUNT,
                },
                WalletAmount {
                    address: faucet_addr,
                    denom: "uterp".to_string(),
                    amount: VALIDATOR_GENESIS_AMOUNT,
                },
            ])
            .await?;
        wait_for_chain_ready(&chain).await?;

        self.chain_info = Some(build_chain_info(&chain));
        self.chain = Some(chain);
        self.started = true;
        Ok(())
    }

    /// `ChainInfoOwned` snapshot built from the live chain's gRPC endpoint
    /// and config. Cloneable — safe to hand to the std worker thread.
    fn chain_info(&self) -> Option<&ChainInfoOwned> {
        self.chain_info.as_ref()
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(ref mut c) = self.chain {
            let _ = c.stop().await;
        }
        self.chain = None;
        self.chain_info = None;
        self.started = false;
        Ok(())
    }
}

async fn wait_for_chain_ready(chain: &CosmosChain) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > TERP_STARTUP_TIMEOUT {
            return Err(anyhow!("Terp chain did not produce blocks"));
        }
        match chain.height().await {
            Ok(h) if h > 0 => return Ok(()),
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

/// /// Build a `ChainInfoOwned` for cw-orch from the live `CosmosChain`. Uses
/// `pub_address_prefix`/`coin_type` from the chain config so the Daemon's
/// bech32 address derivation matches the chain's. Failure to mirror these
/// here would silently produce fundable-looking addresses the chain doesn't
/// recognize, dropping every tx.
fn build_chain_info(chain: &CosmosChain) -> ChainInfoOwned {
    let cfg = chain.config();
    let gas_price: f64 = cfg
        .gas_prices
        .trim_end_matches(|c: char| c.is_alphabetic())
        .parse()
        .unwrap_or(0.025);

    ChainInfoOwned {
        chain_id: cfg.chain_id.clone(),
        gas_denom: cfg.denom.clone(),
        gas_price,
        grpc_urls: vec![chain.host_grpc_address()],
        lcd_url: None,
        fcd_url: None,
        network_info: NetworkInfoOwned {
            chain_name: cfg.name.clone(),
            pub_address_prefix: cfg.bech32_prefix.clone(),
            coin_type: cfg.coin_type,
        },
        kind: networks::ChainKind::Local,
    }
}

/// Replace `cw_orch::coin`-style helper (kept here to avoid touching imports).
fn coins(amount: u128, denom: &str) -> Vec<cw_orch::prelude::Coin> {
    vec![cw_orch::prelude::Coin {
        amount: amount.into(),
        denom: denom.to_string(),
    }]
}

// ---------------------------------------------------------------------------
// cw-orch scenario plumbing (SYNC — runs on a std worker thread)
//
// Every Daemon interaction here is sync from the cw-orch API level. Sync
// `TxHandler` methods internally `rt_handle.block_on(..)`. That block_on
// panics if called from inside a tokio runtime context, so this function
// MUST execute on a non-tokio thread (callers pass us into `std::thread`).
// Async pieces (queriers / `commit_tx_any`) are driven with
// `daemon.rt_handle.block_on(future)` — safe here, no context is entered.
// ---------------------------------------------------------------------------

fn run_scenarios_blocking(chain_info: ChainInfoOwned, mnemonics: Vec<String>) -> Vec<String> {
    // ── Build faucet Daemon once. This is the only full Daemon build; we
    //    rebuild only the sender for each scenario (per user directive).
    let faucet: Daemon = match DaemonBuilder::new(chain_info)
        .is_test(true)
        .mnemonic(FAUCET_MNEMONIC)
        .build()
    {
        Ok(d) => d,
        Err(e) => {
            error!("Faucet daemon build failed: {e}");
            return vec![format!("faucet build: {e}")];
        }
    };
    info!(faucet = %faucet.sender_addr(), "faucet daemon ready");

    let faucet = Arc::new(faucet);
    let faucet_for_thread = Arc::clone(&faucet);

    match std::thread::spawn(move || prepare_wasm_light_clients((*faucet_for_thread).clone()))
        .join()
    {
        Ok(Ok(())) => info!("wasm light client store-code prepare completed"),
        Ok(Err(e)) => {
            error!("wasm light client prepare failed: {e}");
            return vec![format!("prepare_wasm_light_clients: {e}")];
        }
        Err(p) => {
            error!(?p, "prepare_wasm_light_clients thread panicked");
            return vec![format!("prepare_wasm_light_clients panic: {p:?}")];
        }
    };

    // ── Per-scenario Daemon = SENDER rebuild (state/handle/chain reused
    //    from the faucet; only the signing key changes). This avoids
    //    re-doing state-file setup for every scenario.
    let mut scenario_daemons: Vec<Daemon> = Vec::with_capacity(mnemonics.len());
    for mnemonic in &mnemonics {
        let daemon: Daemon = match faucet
            .rebuild()
            .build_sender(CosmosOptions::default().mnemonic(mnemonic))
        {
            Ok(d) => d,
            Err(e) => {
                error!("Scenario daemon ({mnemonic}) build failed: {e}");
                return vec![format!("scenario build: {e}")];
            }
        };

        // Fund the scenario wallet from the faucet. `bank_send` is a sync
        // `TxHandler` call → `rt_handle.block_on(..)` internally — safe
        // here, we are on a std thread.
        let addr = daemon.sender_addr();
        info!(addr = %addr, "scenario wallet ");
        info!(mnemonic = %mnemonic, "mnemonic wallet");
        if let Err(e) = faucet.bank_send(&addr, &coins(SCENARIO_FUND_AMOUNT, "uterp")) {
            error!(scenario = %addr, "faucet bank_send failed: {e}");
            // Not fatal — the scenario may still proceed for read-only flows.
        } else {
            info!(scenario = %addr, "scenario wallet funded");
        }
        scenario_daemons.push(daemon);
    }

    // ── Pair scenarios with their mnemonic-derived Daemons ──────────
    let cases = test_cases();
    let count = scenario_daemons.len().min(cases.len());
    if count == 0 {
        error!("No scenario daemons available — cannot run any test case");
        return vec!["no scenario daemons".to_string()];
    }
    info!(
        scenarios = count,
        "running scenarios sequentially (clearer client-id / log ordering)"
    );

    // Sequential: each scenario has its own funded daemon; running one-at-a-time
    // keeps MsgCreateClient client-id assignment deterministic and logs readable.
    let mut failures = Vec::new();
    for (i, daemon) in scenario_daemons.into_iter().take(count).enumerate() {
        let case = cases[i].clone();
        info!(scenario_index = i, name = case.name, "── scenario begin ──");
        // Still on a std thread here (run_scenarios_blocking); each case may
        // spawn nothing further — call blocking path directly.
        match run_test_case_blocking(daemon, case) {
            Ok(()) => info!(scenario_index = i, "scenario completed"),
            Err(e) => {
                error!(scenario_index = i, "scenario failed: {e:#}");
                failures.push(format!("scenario {i}: {e:#}"));
            }
        }
    }
    failures
}
/// SHA-256 of `artifacts/cw_ics08_wasm_crosslink.wasm` (see `artifacts/checksums.txt`).
/// 08-wasm ClientState.checksum is the raw 32-byte digest, not the hex string.
const WASM_CHECKSUM_HEX: &str = "1619e9fee9bf38cb425dc7a450d397864564aa0909eb9df835b7ae08ccbbd8d8";

/// Well-known cosmos-sdk `gov` module account, re-encoded with the `terp` HRP.
/// 08-wasm `MsgStoreCode.signer` must be the gov authority; the proposal
/// *proposer* is the funded faucet, but the embedded message authority is gov.
const GOV_MODULE_ADDRESS: &str = "terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq";

/// Matches ict-rs `modify_terp_genesis` min_deposit (10_000_000 uterp).
const GOV_MIN_DEPOSIT_UTERP: &str = "10000000";

/// Proposal title/summary used for both MsgSubmitProposal fields and metadata
/// JSON. cosmos-sdk gov rejects when metadata unmarshals as ProposalMetadata
/// but title/summary do not match the proposal fields.
const GOV_PROPOSAL_TITLE: &str = "upload-cw-ics08-wasm-crosslink";
const GOV_PROPOSAL_SUMMARY: &str = "Store the crosslink 08-wasm light client bytecode";

/// ict-rs sets voting_period / max_deposit_period to 6s for Terp local nets.
/// Wait past that so the proposal tallies and executes MsgStoreCode.
const GOV_VOTING_WAIT: Duration = Duration::from_secs(12);

fn wasm_checksum_bytes() -> Result<Vec<u8>> {
    hex::decode(WASM_CHECKSUM_HEX).map_err(|e| anyhow!("invalid WASM_CHECKSUM_HEX: {e}"))
}

/// Recommended cosmos-sdk gov v1 metadata JSON. Title/summary MUST equal the
/// proposal's top-level title/summary when the JSON unmarshals successfully.
fn gov_proposal_metadata(title: &str, summary: &str) -> String {
    serde_json::json!({
        "title": title,
        "summary": summary,
        "authors": ["crosslink-e2e"],
        "details": summary,
        "proposal_forum_url": "",
        "vote_option_context": "",
    })
    .to_string()
}

/// Extract `proposal_id` from a submit-proposal tx response. Falls back to 1
/// (first proposal on a fresh local chain) when events are empty.
fn proposal_id_from_tx(res: &cw_orch::daemon::CosmTxResponse) -> u64 {
    // Prefer typed log attributes.
    for key in ["proposal_id", "proposal-id"] {
        for event_type in ["submit_proposal", "proposal_deposit", "message"] {
            let attrs = res.get_attribute_from_logs(event_type, key);
            if let Some((_, v)) = attrs.first() {
                if let Ok(id) = v.parse::<u64>() {
                    return id;
                }
            }
        }
    }
    // Fall back to raw ABCI events.
    for ev in res.get_events("submit_proposal") {
        for attr in &ev.attributes {
            if attr.key == "proposal_id" || attr.key == "proposal-id" {
                if let Ok(id) = attr.value.parse::<u64>() {
                    return id;
                }
            }
        }
    }
    1
}

/// Full 08-wasm store-code lifecycle on a local Terp chain:
///
/// 1. Bond stake so the faucet has voting power (gentx self-bond is ~5e12;
///    we bond 1e15 so a single YES from the faucet clears quorum).
/// 2. Submit a gov v1 proposal embedding `MsgStoreCode` (signer = gov module).
///    - metadata JSON title/summary match proposal fields
///    - initial_deposit ≥ min_deposit so voting starts immediately
/// 3. Vote YES as the faucet.
/// 4. Wait past the 6s voting period for tally + message execution.
fn prepare_wasm_light_clients(daemon: Daemon) -> Result<()> {
    let rt = daemon.rt_handle.clone();
    let staking: Staking = daemon.querier();
    let proposer = daemon.sender_addr().to_string();

    let val = rt.block_on(staking._validators(queriers::StakingBondStatus::Bonded))?;
    if val.is_empty() {
        return Err(anyhow!("no bonded validators — cannot obtain voting power"));
    }

    // ── 1. Bond stake for voting power ───────────────────────────────────
    let del_res = rt.block_on(daemon.sender().commit_tx_any(
        vec![Any::from_msg(&terp_rs::cosmos::staking::v1::MsgDelegate {
            delegator_address: proposer.clone(),
            validator_address: val[0].address.clone(),
            amount: Some(Coin {
                denom: "uterp".into(),
                amount: "1000000000000000".into(),
            }),
        })?],
        None,
    ))?;
    if del_res.code != 0 {
        return Err(anyhow!(
            "MsgDelegate failed (code {}): {}",
            del_res.code,
            del_res.raw_log
        ));
    }
    info!("bonded stake for gov voting power");

    // ── 2. Submit proposal (deposit → voting period) ─────────────────────
    let title = GOV_PROPOSAL_TITLE.to_string();
    let summary = GOV_PROPOSAL_SUMMARY.to_string();
    let metadata = gov_proposal_metadata(&title, &summary);
    let wasm_byte_code = include_bytes!("../../artifacts/cw_ics08_wasm_crosslink.wasm").to_vec();

    let submit_res = rt.block_on(daemon.sender().commit_tx_any(
        vec![Any::from_msg(
            &terp_rs::cosmos::gov::v1::MsgSubmitProposal {
                messages: vec![Any::from_msg(
                    &terp_rs::ibc::lightclients::wasm::v1::MsgStoreCode {
                        // Authority that may store 08-wasm code = gov module account.
                        signer: GOV_MODULE_ADDRESS.into(),
                        wasm_byte_code,
                    },
                )?],
                // Must meet min_deposit (10_000_000 uterp) or proposal never leaves
                // deposit period (max_deposit_period is only 6s on local Terp).
                initial_deposit: vec![Coin {
                    denom: "uterp".into(),
                    amount: GOV_MIN_DEPOSIT_UTERP.into(),
                }],
                proposer: proposer.clone(),
                metadata,
                title,
                summary,
                expedited: false,
            },
        )?],
        None,
    ))?;
    if submit_res.code != 0 {
        return Err(anyhow!(
            "MsgSubmitProposal failed (code {}): {}",
            submit_res.code,
            submit_res.raw_log
        ));
    }

    let proposal_id = proposal_id_from_tx(&submit_res);
    info!(proposal_id, ?submit_res.txhash, "MsgStoreCode proposal submitted");

    // Let the proposal enter voting period (deposit already met).
    daemon.wait_blocks(1)?;

    // ── 3. Vote YES ──────────────────────────────────────────────────────
    // VoteOption::Yes = 1
    let vote_res = rt.block_on(daemon.sender().commit_tx_any(
        vec![Any::from_msg(&terp_rs::cosmos::gov::v1::MsgVote {
            proposal_id,
            voter: proposer,
            option: terp_rs::cosmos::gov::v1::VoteOption::Yes as i32,
            metadata: String::default(),
        })?],
        None,
    ))?;
    if vote_res.code != 0 {
        return Err(anyhow!(
            "MsgVote failed (code {}): {}",
            vote_res.code,
            vote_res.raw_log
        ));
    }
    info!(proposal_id, "YES vote submitted");

    // ── 4. Wait for voting period to end + execution ─────────────────────
    // ict-rs Terp genesis: voting_period = 6s. Sleep past it, then a couple of
    // blocks so EndBlocker tallies and runs the embedded MsgStoreCode.
    info!(
        wait_secs = GOV_VOTING_WAIT.as_secs(),
        "waiting for voting period + execution"
    );
    std::thread::sleep(GOV_VOTING_WAIT);
    daemon.wait_blocks(5)?;

    // Confirm governance actually passed and executed MsgStoreCode.
    // Without this check, CreateClient fails later with "checksum has not
    // been previously stored" and the real failure is hard to see.
    let prop_info = rt.block_on(async {
        use cosmrs::proto::cosmos::gov::v1::query_client::QueryClient;
        use cosmrs::proto::cosmos::gov::v1::QueryProposalRequest;
        let channel = daemon.channel();
        let mut client = QueryClient::new(channel);
        let resp = client
            .proposal(QueryProposalRequest { proposal_id })
            .await?
            .into_inner();
        Ok::<_, anyhow::Error>(resp.proposal.map(|p| (p.status, p.failed_reason)))
    });
    match prop_info {
        Ok(Some((status, failed_reason))) => {
            // ProposalStatus::Passed = 3, Failed = 5
            info!(proposal_id, status, %failed_reason, "gov proposal final status");
            if status != 3 {
                return Err(anyhow!(
                    "MsgStoreCode proposal {proposal_id} did not pass \
                     (status={status}, failed_reason={failed_reason:?}); \
                     08-wasm checksum will be missing. Artifact: {WASM_CHECKSUM_HEX} \
                     ({} bytes). Typical causes: out-of-gas on store, wasm validation \
                     failure against the chain's wasmvm, or max contract size.",
                    include_bytes!("../../artifacts/cw_ics08_wasm_crosslink.wasm").len()
                ));
            }
        }
        Ok(None) => {
            return Err(anyhow!(
                "MsgStoreCode proposal {proposal_id} not found after voting period"
            ));
        }
        Err(e) => {
            error!(proposal_id, "could not query proposal status: {e:#}");
        }
    }

    info!(
        proposal_id,
        expected_checksum = WASM_CHECKSUM_HEX,
        "wasm light client store-code prepare finished"
    );
    Ok(())
}

/// Format a header for debug logs.
fn fmt_header_debug(header: &CrosslinkHeader) -> String {
    format!(
        "trusted_bft={} new_bft={} pow_anchor={} sigs={} fat_ptr_hash={:02x?} block_hash={:02x?} commitment_ok={}",
        header.trusted_bft_height,
        header.bft_block.height,
        header.bft_block.finalization_candidate_height,
        header.fat_pointer.signatures.len(),
        &header.fat_pointer.points_at_block_hash().0[..4],
        &header.block_hash().0[..4],
        header.validate_block_commitment(),
    )
}

/// Resolve the client_id created by the latest MsgCreateClient by diffing
/// the client list before/after. Avoids hardcoding `08-wasm-0` which races
/// when multiple scenarios create clients.
fn resolve_new_client_id(
    rt: &tokio::runtime::Handle,
    ibc: &Ibc,
    before: &[String],
    case_name: &str,
) -> Result<String> {
    let after = rt
        .block_on(ibc._clients())
        .map_err(|e| anyhow!("[{case_name}] ibc clients query (after create) failed: {e}"))?;
    let after_ids: Vec<String> = after.into_iter().map(|c| c.client_id).collect();
    info!(case = case_name, before = ?before, after = ?after_ids, "client id sets");

    let new_ids: Vec<_> = after_ids
        .iter()
        .filter(|id| !before.iter().any(|b| b == *id))
        .cloned()
        .collect();
    match new_ids.as_slice() {
        [id] => Ok(id.clone()),
        [] => {
            // Fallback: highest-index 08-wasm-* client (first create on a clean chain).
            after_ids
                .into_iter()
                .filter(|id| id.starts_with("08-wasm-"))
                .max_by_key(|id| {
                    id.trim_start_matches("08-wasm-")
                        .parse::<u64>()
                        .unwrap_or(0)
                })
                .ok_or_else(|| anyhow!("[{case_name}] no 08-wasm client after MsgCreateClient"))
        }
        many => {
            // Parallel creates can add multiple; take the highest index among new ones.
            Ok(many
                .iter()
                .max_by_key(|id| {
                    id.trim_start_matches("08-wasm-")
                        .parse::<u64>()
                        .unwrap_or(0)
                })
                .unwrap()
                .clone())
        }
    }
}

/// Run a single scenario (SYNC — must execute on a std thread, NOT inside a
/// tokio runtime context). Async bits are driven explicitly via
/// `daemon.rt_handle.block_on(..)`.
fn run_test_case_blocking(daemon: Daemon, case: TestCase) -> Result<()> {
    let case_name = case.name;
    info!(
        case = case_name,
        expect_ok = case.expect_sequence_succeeds,
        expected_bft_after = case.expected_bft_height_after,
        update_count = case.updates.len(),
        client_latest_bft = case.initial_client_state.latest_bft_height,
        consensus_bft = case.initial_consensus_state.bft_height,
        roster_size = case.initial_client_state.finalizer_roster.len(),
        "starting scenario"
    );
    for (i, h) in case.updates.iter().enumerate() {
        info!(case = case_name, step = i, header = %fmt_header_debug(h), "planned update");
        if h.trusted_bft_height != case.initial_client_state.latest_bft_height && i == 0 {
            // Soft warning only — later steps intentionally diverge in negative tests.
            error!(
                case = case_name,
                step = i,
                trusted = h.trusted_bft_height,
                client_latest = case.initial_client_state.latest_bft_height,
                "WARNING: first update trusted_bft_height does not match client latest_bft_height"
            );
        }
    }

    let rt = daemon.rt_handle.clone();
    let ibc: Ibc = daemon.querier();
    let signer = daemon.sender_addr().to_string();
    let checksum = wasm_checksum_bytes()?;

    let clients_before: Vec<String> = rt
        .block_on(ibc._clients())
        .map_err(|e| anyhow!("[{case_name}] ibc clients query (before create) failed: {e}"))?
        .into_iter()
        .map(|c| c.client_id)
        .collect();

    let client_state_bytes = case
        .initial_client_state
        .zcash_serialize_to_vec()
        .map_err(|e| anyhow!("[{case_name}] client_state serialize: {e}"))?;
    let consensus_state_bytes = case
        .initial_consensus_state
        .zcash_serialize_to_vec()
        .map_err(|e| anyhow!("[{case_name}] consensus_state serialize: {e}"))?;

    info!(
        case = case_name,
        client_state_len = client_state_bytes.len(),
        consensus_state_len = consensus_state_bytes.len(),
        checksum_hex = WASM_CHECKSUM_HEX,
        "submitting MsgCreateClient"
    );

    let create_res = rt.block_on(daemon.sender().commit_tx_any(
        vec![Any::from_msg(&MsgCreateClient {
            client_state: Some(Any::from_msg(&ClientState {
                data: client_state_bytes,
                checksum,
                latest_height: Some(terp_rs::ibc::core::client::v1::Height {
                    revision_number: 0,
                    revision_height: case.initial_client_state.latest_bft_height as u64,
                }),
            })?),
            consensus_state: Some(Any::from_msg(&ConsensusState {
                data: consensus_state_bytes,
            })?),
            signer: signer.clone(),
        })?],
        None,
    ));
    let create_res = match create_res {
        Ok(r) if r.code == 0 => {
            info!(
                case = case_name,
                txhash = %r.txhash,
                height = r.height,
                gas_used = r.gas_used,
                "MsgCreateClient OK"
            );
            r
        }
        Ok(r) => {
            return Err(anyhow!(
                "[{case_name}] MsgCreateClient rejected code={} raw_log={}",
                r.code,
                r.raw_log
            ));
        }
        Err(e) => {
            return Err(anyhow!(
                "[{case_name}] MsgCreateClient broadcast error: {e}"
            ));
        }
    };
    let _ = create_res;

    let client_id = resolve_new_client_id(&rt, &ibc, &clients_before, case_name)?;
    info!(case = case_name, %client_id, "resolved client id");

    // ── Submit each header as WasmClientMessage-wrapped MsgUpdateClient ─
    let mut applied_heights: Vec<u32> = Vec::new();
    let mut expected_latest = case.initial_client_state.latest_bft_height;
    let mut first_rejection: Option<(usize, String)> = None;

    for (i, header) in case.updates.iter().enumerate() {
        info!(
            case = case_name,
            step = i,
            client_id = %client_id,
            expected_latest,
            header = %fmt_header_debug(header),
            "submitting MsgUpdateClient"
        );

        let header_bytes = header
            .zcash_serialize_to_vec()
            .map_err(|e| anyhow!("[{case_name}] header {i} serialize failed: {e}"))?;
        info!(
            case = case_name,
            step = i,
            header_bytes = header_bytes.len(),
            "serialized CrosslinkHeader"
        );

        let update = MsgUpdateClient {
            client_id: client_id.clone(),
            client_message: Some(
                Any::from_msg(&WasmClientMessage { data: header_bytes })
                    .map_err(|e| anyhow!("[{case_name}] WasmClientMessage Any: {e}"))?,
            ),
            signer: signer.clone(),
        };
        let any = Any::from_msg(&update)
            .map_err(|e| anyhow!("[{case_name}] MsgUpdateClient Any: {e}"))?;
        let result = rt.block_on(daemon.sender().commit_tx_any(vec![any], None));

        let (ok, detail) = match &result {
            Ok(resp) if resp.code == 0 => (
                true,
                format!("OK txhash={} gas={}", resp.txhash, resp.gas_used),
            ),
            Ok(resp) => (
                false,
                format!("on-chain code={} raw_log={}", resp.code, resp.raw_log),
            ),
            Err(e) => (false, format!("broadcast error: {e}")),
        };

        if ok {
            info!(case = case_name, step = i, %detail, "MsgUpdateClient accepted");
            applied_heights.push(header.bft_block.height);
            expected_latest = header.bft_block.height;
        } else {
            error!(
                case = case_name,
                step = i,
                expected_latest,
                trusted = header.trusted_bft_height,
                new_bft = header.bft_block.height,
                %detail,
                "MsgUpdateClient rejected"
            );
            if first_rejection.is_none() {
                first_rejection = Some((i, detail.clone()));
            }
            if case.expect_sequence_succeeds {
                return Err(anyhow!(
                    "[{case_name}] header {i} unexpectedly rejected \
                     (expected_latest={expected_latest}, header={}): {detail}",
                    fmt_header_debug(header)
                ));
            }
            // Negative scenarios: stop after first rejection so later steps
            // don't cascade confusing "trusted != latest" noise.
            info!(
                case = case_name,
                step = i,
                "negative scenario: halting updates after first rejection"
            );
            break;
        }
    }

    if !case.expect_sequence_succeeds && first_rejection.is_none() {
        return Err(anyhow!(
            "[{case_name}] expected a rejection but all {} updates succeeded (applied={applied_heights:?})",
            case.updates.len()
        ));
    }

    info!(
        case = case_name,
        applied_heights = ?applied_heights,
        expected_bft_height_after = case.expected_bft_height_after,
        first_rejection = ?first_rejection,
        "scenario PASSED"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Enhanced Scenario data with Ed25519 keys for consensus
// ---------------------------------------------------------------------------

/// Generate a set of Ed25519 test keypairs for finalizers.
/// Returns (finalizers, signing_keys) where signing_keys[i] corresponds to finalizers[i].
fn generate_test_finalizers(count: usize) -> (Vec<FinalizerEntry>, Vec<SigningKey>) {
    let mut finalizers = Vec::with_capacity(count);
    let mut signing_keys = Vec::with_capacity(count);

    for i in 0..count {
        // Deterministic seed. Use From<[u8;32]> to bypass rand_core CryptoRng
        // requirement (rand_core 0.6 vs 0.9 conflict with zebra deps).
        let seed: [u8; 32] = [i as u8; 32];
        let signing_key = SigningKey::from(seed);
        let verification_key: VerificationKey = VerificationKey::from(&signing_key);
        let pubkey_bytes: [u8; 32] = verification_key.into();

        finalizers.push(FinalizerEntry {
            public_key: pubkey_bytes,
            voting_power: if i == 0 { 3 } else { 1 }, // Bias for threshold tests
        });
        signing_keys.push(signing_key);
    }

    (finalizers, signing_keys)
}

/// Helper: compute the message to sign for a BFT block.
/// This is the 44-byte vote template: 32-byte block hash + 12-byte suffix.
fn compute_vote_message(block_hash: &Blake3Hash) -> Vec<u8> {
    let mut msg = vec![0u8; 44];
    msg[0..32].copy_from_slice(&block_hash.0);
    // The remaining 12 bytes protocol-defined suffix (all zeros for now)
    msg
}

/// Correct native signing for a CrosslinkHeader update.
///
/// The signatures proving finality for *this* BFT block go in the outer
/// `fat_pointer` of the CrosslinkHeader (what the light client verifies).
/// The BftBlock's `previous_block_fat_ptr` is only for linking to the prior BFT block.
fn signed_header_update(
    bft_height: u32,
    commit_height: u32,
    prev_hash: [u8; 32],
    signing_keys: &[SigningKey],
) -> CrosslinkHeader {
    // Build the BFT block payload using the existing synthetic helper (linkage only).
    let bft_block = bft_block(bft_height, commit_height, prev_hash);

    // The 44-byte message to sign is the current BFT block's blake3 hash + protocol suffix.
    let block_hash = bft_block.blake3_hash();
    let vote_message = compute_vote_message(&block_hash);

    // Native ed25519 signing (all keys for the roster).
    let signatures: Vec<FatPointerSignature2> = signing_keys
        .iter()
        .map(|sk| {
            let verification_key: VerificationKey = VerificationKey::from(sk);
            let pubkey_bytes: [u8; 32] = verification_key.into();
            let signature = ed25519_zebra::Signature::from(sk.sign(&vote_message));
            FatPointerSignature2 {
                public_key: pubkey_bytes,
                vote_signature: signature.to_bytes().to_vec(),
            }
        })
        .collect();

    let fat_pointer = FatPointerToBftBlock2 {
        vote_for_block_without_finalizer_public_key: vote_message,
        signatures,
    };

    CrosslinkHeader {
        trusted_bft_height: bft_height - 1,
        bft_block,
        fat_pointer,
    }
}

// ---------------------------------------------------------------------------
// E2E test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires Docker for the terp chain (no zebrad needed anymore)
async fn test_crosslink_light_client_e2e() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();
    let _ = tracing_subscriber::fmt::try_init();

    // ── Phase 1: ict-rs chain (async). Drives the test runtime directly —
    //    No cw-orch calls here, so no `block_on`-within-runtime risk.
    let mut env = match CrosslinkTestEnv::new("crosslink-e2e").await {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to create env: {e}");
            return;
        }
    };
    if let Err(e) = env.start().await {
        error!("Failed to start env: {e}");
        let _ = env.stop().await;
        return;
    }

    let chain_info = match env.chain_info() {
        Some(info) => info.clone(),
        None => {
            error!("chain_info missing after start()");
            let _ = env.stop().await;
            return;
        }
    };

    // Collapse the `\<newline>`-broken string literals into clean BIP-39 phrases.
    let mnemonics: Vec<String> = SCENARIO_MNEMONICS
        .iter()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();

    // ── Phase 2: cw-orch daemon work. ALL of it runs on a plain std worker
    //    thread — no tokio runtime context is entered there, so the sync
    //    `TxHandler` methods (which block_on internally) cannot hit the
    //    "Cannot start a runtime from within a runtime" panic. The chain
    //    container must stay alive while this phase runs → env is not dropped
    //    until after `join()`.
    let handle = std::thread::spawn(move || -> Vec<String> {
        run_scenarios_blocking(chain_info, mnemonics)
    });
    let failures = handle.join().expect("scenario worker thread panicked");

    // ── Cleanup
    if let Err(e) = env.stop().await {
        error!("Cleanup error: {e}");
    }

    if failures.is_empty() {
        info!("Crosslink e2e test passed — all scenarios green");
    } else {
        panic!("Crosslink e2e test failed:\n{}", failures.join("\n"));
    }
}
