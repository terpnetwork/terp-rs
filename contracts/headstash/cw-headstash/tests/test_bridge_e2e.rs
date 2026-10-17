//! Round-3 multi-test e2e for the private-bridge mint surface on `cw-headstash`.
//!
//! Exercises the **real** execute/query path via `cw-multi-test` ContractWrapper
//! (not pure `authorize_bridge_mint_pure` alone):
//!
//! 1. Instantiate headstash (ExistingFungible + funds — no tokenfactory Stargate)
//! 2. Owner: SetBridgeCfg (mock_verify=true), SetReflectionSnapshot, RegisterAsset
//! 3. BridgeMintNote happy path → IsBridgeMinted true + SEAM attrs
//! 4. Double mint same ν → reject
//! 5. H-1 spent-only reject
//! 6. Unregistered asset reject
//!
//! Proof verify is the Round-2 mock (`BridgeCfg.mock_verify`); no SP1 / anvil.

use cosmwasm_std::{Addr, Binary, Coin, Uint128};
use cw_headstash::bridge::{
    AssetEntry, AssetStatus, BridgeCfg, BridgeMintClaimPublic, NoteOutResult, ReflectionSnapshot,
    DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG, ORIGIN_BRIDGE_MINT, NF_BRIDGE_BURN,
    derive_claim_id_with_dest, derive_domain_binding, hash32_label, terp_asset_id_from_tacit,
    Hash32,
};
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::{WavsAuthMetadata, WavsOpAuth, WavsProofOfOwnership};
use cw_multi_test::{App, ContractWrapper, Executor};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn bin32(h: Hash32) -> Binary {
    Binary::from(h.to_vec())
}

/// Deterministic valid WAVS PoO (1 operator) so instantiate's pairing check passes.
fn valid_wavs_proof_one() -> WavsProofOfOwnership {
    use ark_bls12_381::{Fr, G1Affine, G2Affine};
    use ark_ec::AffineRepr;
    use ark_ff::{PrimeField, Zero};
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use cosmwasm_std::{
        testing::MockApi, Api, BLS12_381_G2_GENERATOR as G2, HashFunction,
    };

    let api = MockApi::default();
    // Fixed non-zero scalar — no OsRng; keeps e2e deterministic and dep-light.
    let sk = Fr::from_be_bytes_mod_order(&[0x42; 32]);
    assert!(!sk.is_zero());

    let pk: G1Affine = (G1Affine::generator() * sk).into();
    let mut pk_bytes = vec![];
    pk.serialize_compressed(&mut pk_bytes).unwrap();

    let pop_hash = api
        .bls12_381_hash_to_g2(HashFunction::Sha256, &pk_bytes, &G2)
        .unwrap();
    let h_point = G2Affine::deserialize_compressed(&pop_hash[..]).unwrap();
    let pop_sig: G2Affine = (h_point * sk).into();
    let mut sig_bytes = vec![];
    pop_sig.serialize_compressed(&mut sig_bytes).unwrap();

    let mut agg_pk_bytes = vec![];
    pk.serialize_compressed(&mut agg_pk_bytes).unwrap();

    WavsProofOfOwnership {
        poos: vec![WavsOpAuth {
            key: hex::encode(&pk_bytes),
            poo: hex::encode(&sig_bytes),
        }],
        msg: WavsAuthMetadata {
            aggregate_key: hex::encode(agg_pk_bytes),
            threshold: 1,
            total_operators: 1,
            nonce: 0,
        },
    }
}

/// Bridge fixture aligned with `bridge::tests::fixture` (happy path constants).
fn bridge_fixture() -> (
    ReflectionSnapshot,
    BridgeCfg,
    BridgeMintClaimPublic,
    Hash32,
    AssetEntry,
) {
    let dest = hash32_label("terp-chain-1");
    let tacit = hash32_label("tacit-btc-etch-1");
    let unit_scale = 1u64;
    let terp_asset = terp_asset_id_from_tacit("bitcoin-mainnet", &tacit, unit_scale);
    let nu = hash32_label("nu-hinge-happy");
    let dest_cm = hash32_label("dest-commitment-A");
    let pool_root = hash32_label("pool-root-1");
    let spent_root = hash32_label("spent-root-1");
    let burn_root = hash32_label("burn-root-1");
    let height = 100u64;
    let tip = height + DEFAULT_CONFIRMATIONS_K;
    let claim_id = derive_claim_id_with_dest(&dest, &dest_cm, &nu, &tacit, 1_000_000);
    let src_chain = hash32_label("src-bitcoin-mainnet");
    let dst_chain = dest;
    let lc_client = hash32_label("lc-client-reflection-0");
    let domain_binding = derive_domain_binding(
        &src_chain,
        &dst_chain,
        &lc_client,
        &tacit,
        &nu,
        height,
        &burn_root,
    );
    let rcm = hash32_label("rcm-hinge-happy");

    let snapshot = ReflectionSnapshot {
        pool_root: bin32(pool_root),
        spent_root: bin32(spent_root),
        burn_root: bin32(burn_root),
        source_height: height,
        tip_height: tip,
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        frozen: false,
    };
    let cfg = BridgeCfg {
        dest_domain: bin32(dest),
        confirmations_k: DEFAULT_CONFIRMATIONS_K,
        max_lc_lag: DEFAULT_MAX_LC_LAG,
        lc_client_id: "08-wasm-tacit-reflection-0".into(),
        // Integration tests compile the library without cfg(test) — must set mock_verify.
        mock_verify: true,
        egress_zkid: None,
    };
    let claim = BridgeMintClaimPublic {
        source_chain_tag: "bitcoin-mainnet".into(),
        tacit_asset_id: bin32(tacit),
        value_u64: 1_000_000,
        nullifier: bin32(nu),
        dest_commitment: bin32(dest_cm),
        dest_domain: bin32(dest),
        claim_id: bin32(claim_id),
        source_pool_root: bin32(pool_root),
        source_burn_root: bin32(burn_root),
        source_height: height,
        domain_binding: bin32(domain_binding),
        unit_scale,
        pool_domain: bin32(hash32_label("terp-pool-0")),
        cm_public: bin32(hash32_label("cm-leaf-dest-1")),
        rcm: Some(bin32(rcm)),
        in_burn_set: true,
        in_pool_root: true,
        spent_only: false,
        src_chain_id: bin32(src_chain),
        dst_chain_id: bin32(dst_chain),
        lc_client_id: bin32(lc_client),
        burn_dest_commitment: bin32(dest_cm),
    };
    let asset = AssetEntry {
        asset_id: bin32(terp_asset),
        local_denom: "ubtc".into(),
        origin: Some("tacit:bitcoin-mainnet".into()),
        status: AssetStatus::Active,
    };
    (snapshot, cfg, claim, terp_asset, asset)
}

/// Non-empty mock proof blob (empty rejected outside cfg(test)).
fn mock_proof() -> Binary {
    Binary::from(vec![0xde, 0xad, 0xbe, 0xef, 0x01])
}

struct BridgeEnv {
    app: App,
    owner: Addr,
    user: Addr,
    headstash: Addr,
}

impl BridgeEnv {
    /// Deploy actual cw-headstash code via multi-test wrapper and instantiate.
    fn new() -> Self {
        let mut app = App::default();
        let owner = app.api().addr_make("owner");
        let user = app.api().addr_make("bridge_user");

        // Prefund owner with existing denom for ExistingFungible instantiate.
        app.init_modules(|router, _, storage| {
            router
                .bank
                .init_balance(
                    storage,
                    &owner,
                    vec![
                        Coin::new(Uint128::new(1_000_000), "uosmo"),
                        Coin::new(Uint128::new(10_000), "ubridge"),
                    ],
                )
                .unwrap();
            router
                .bank
                .init_balance(
                    storage,
                    &user,
                    vec![Coin::new(Uint128::new(1_000), "uosmo")],
                )
                .unwrap();
        });

        let code = Box::new(
            ContractWrapper::new(
                cw_headstash::execute,
                cw_headstash::instantiate,
                cw_headstash::query,
            )
            .with_reply(cw_headstash::reply),
        );
        let code_id = app.store_code(code);

        let init = InstantiateMsg {
            genesis_root: Binary::from(vec![0xab; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: Some("bridge-e2e".into()),
            token_strategy: TokenStrategy::ExistingFungible(HeadstashTokenObject::new(
                "ubridge".into(),
            )),
            wavs: valid_wavs_proof_one(),
            circuit_id: None,
        };

        let headstash = app
            .instantiate_contract(
                code_id,
                owner.clone(),
                &init,
                &[Coin::new(Uint128::new(10_000), "ubridge")],
                "cw-headstash-bridge-e2e",
                Some(owner.to_string()),
            )
            .expect("instantiate headstash");

        Self {
            app,
            owner,
            user,
            headstash,
        }
    }

    fn exec_owner(&mut self, msg: &ExecuteMsg) {
        self.app
            .execute_contract(self.owner.clone(), self.headstash.clone(), msg, &[])
            .expect("owner execute");
    }

    fn exec_user(&mut self, msg: &ExecuteMsg) -> Result<cw_multi_test::AppResponse, anyhow::Error> {
        self.app
            .execute_contract(self.user.clone(), self.headstash.clone(), msg, &[])
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }

    fn query<T: serde::de::DeserializeOwned>(&self, msg: &QueryMsg) -> T {
        self.app
            .wrap()
            .query_wasm_smart(&self.headstash, msg)
            .expect("query")
    }

    /// Owner corridor: cfg + snapshot + register mapped terp asset.
    fn setup_bridge_happy(&mut self) -> BridgeMintClaimPublic {
        let (snapshot, cfg, claim, terp_asset, _asset) = bridge_fixture();

        self.exec_owner(&ExecuteMsg::SetBridgeCfg { cfg });
        self.exec_owner(&ExecuteMsg::SetReflectionSnapshot { snapshot });
        self.exec_owner(&ExecuteMsg::RegisterAsset {
            asset_id: bin32(terp_asset),
            local_denom: "ubtc".into(),
            origin: Some("tacit:bitcoin-mainnet".into()),
            status: Some(AssetStatus::Active),
        });

        // Sanity queries after owner setup.
        let tip: Option<ReflectionSnapshot> = self.query(&QueryMsg::ReflectionTip {});
        assert!(tip.is_some());
        assert!(!tip.unwrap().frozen);

        let stored_cfg: Option<BridgeCfg> = self.query(&QueryMsg::BridgeConfig {});
        assert!(stored_cfg.unwrap().mock_verify);

        let entry: Option<AssetEntry> = self.query(&QueryMsg::BridgeAsset {
            asset_id: bin32(terp_asset),
        });
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().status, AssetStatus::Active));

        claim
    }
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

#[test]
fn e2e_bridge_mint_happy_path_is_minted() {
    let mut env = BridgeEnv::new();
    let claim = env.setup_bridge_happy();

    let minted_before: bool = env.query(&QueryMsg::IsBridgeMinted {
        nullifier: claim.nullifier.clone(),
    });
    assert!(!minted_before, "ν not minted yet");

    let res = env
        .exec_user(&ExecuteMsg::BridgeMintNote {
            claim: claim.clone(),
            proof: mock_proof(),
        })
        .expect("BridgeMintNote happy");

    // Real execute path attributes (SEAM-NOTE-OUT surface on wasm events).
    let attr = |key: &str, value: &str| {
        res.events.iter().any(|e| {
            e.attributes
                .iter()
                .any(|a| a.key == key && a.value == value)
        })
    };
    assert!(
        attr("action", "bridge_mint_note"),
        "expected bridge_mint_note action attr, events={:?}",
        res.events
    );
    assert!(attr("dex_consumable", "true"), "expected dex_consumable=true");
    assert!(attr("rcm_flag", "1"), "expected rcm_flag=1");

    // Decode note_out payload when present (base64 JSON NoteOutResult).
    let note_b64 = res
        .events
        .iter()
        .flat_map(|e| e.attributes.iter())
        .find(|a| a.key == "note_out")
        .map(|a| a.value.as_str())
        .expect("note_out attribute");
    let note_bin = Binary::from_base64(note_b64).expect("note_out base64");
    let note: NoteOutResult = cosmwasm_std::from_json(&note_bin).expect("note_out json");
    assert_eq!(note.origin, ORIGIN_BRIDGE_MINT);
    assert_eq!(note.nullifier_domain, NF_BRIDGE_BURN);
    assert_eq!(note.rcm_flag, 1);
    assert!(note.is_dex_consumable());
    assert_eq!(note.value, 1_000_000);

    let minted_after: bool = env.query(&QueryMsg::IsBridgeMinted {
        nullifier: claim.nullifier.clone(),
    });
    assert!(minted_after, "IsBridgeMinted must be true after happy mint");
}

#[test]
fn e2e_bridge_double_mint_same_nu_reject() {
    let mut env = BridgeEnv::new();
    let claim = env.setup_bridge_happy();

    env.exec_user(&ExecuteMsg::BridgeMintNote {
        claim: claim.clone(),
        proof: mock_proof(),
    })
    .expect("first mint");

    let err = env
        .exec_user(&ExecuteMsg::BridgeMintNote {
            claim: claim.clone(),
            proof: mock_proof(),
        })
        .expect_err("second mint must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("already minted") || msg.contains("AlreadyMinted"),
        "unexpected err: {msg}"
    );

    let minted: bool = env.query(&QueryMsg::IsBridgeMinted {
        nullifier: claim.nullifier,
    });
    assert!(minted);
}

#[test]
fn e2e_bridge_h1_spent_only_reject() {
    let mut env = BridgeEnv::new();
    let mut claim = env.setup_bridge_happy();
    claim.in_burn_set = false;
    claim.spent_only = true;

    let err = env
        .exec_user(&ExecuteMsg::BridgeMintNote {
            claim: claim.clone(),
            proof: mock_proof(),
        })
        .expect_err("H-1 spent-only must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("H-1") || msg.contains("burn set") || msg.contains("NotInBurnSet"),
        "unexpected err: {msg}"
    );

    let minted: bool = env.query(&QueryMsg::IsBridgeMinted {
        nullifier: claim.nullifier,
    });
    assert!(!minted, "failed mint must not set IsBridgeMinted");
}

#[test]
fn e2e_bridge_unregistered_asset_reject() {
    let mut env = BridgeEnv::new();
    let (snapshot, cfg, claim, _terp, _) = bridge_fixture();

    // cfg + snapshot only — deliberately skip RegisterAsset
    env.exec_owner(&ExecuteMsg::SetBridgeCfg { cfg });
    env.exec_owner(&ExecuteMsg::SetReflectionSnapshot { snapshot });

    let err = env
        .exec_user(&ExecuteMsg::BridgeMintNote {
            claim: claim.clone(),
            proof: mock_proof(),
        })
        .expect_err("unregistered asset must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("unregistered") || msg.contains("unmapped") || msg.contains("UnmappedAsset"),
        "unexpected err: {msg}"
    );

    let minted: bool = env.query(&QueryMsg::IsBridgeMinted {
        nullifier: claim.nullifier,
    });
    assert!(!minted);
}

#[test]
fn e2e_bridge_update_reflection_then_mint() {
    let mut env = BridgeEnv::new();
    let (snapshot, cfg, claim, terp_asset, _) = bridge_fixture();

    env.exec_owner(&ExecuteMsg::SetBridgeCfg { cfg });
    // Use UpdateReflection (partial) rather than full snapshot — still owner path.
    env.exec_owner(&ExecuteMsg::UpdateReflection {
        pool_root: snapshot.pool_root.clone(),
        spent_root: snapshot.spent_root.clone(),
        burn_root: snapshot.burn_root.clone(),
        source_height: snapshot.source_height,
        tip_height: snapshot.tip_height,
    });
    env.exec_owner(&ExecuteMsg::RegisterAsset {
        asset_id: bin32(terp_asset),
        local_denom: "ubtc".into(),
        origin: Some("tacit:bitcoin-mainnet".into()),
        status: None,
    });

    let tip: Option<ReflectionSnapshot> = env.query(&QueryMsg::ReflectionTip {});
    let tip = tip.expect("tip after UpdateReflection");
    assert_eq!(tip.source_height, 100);
    assert_eq!(tip.tip_height, 100 + DEFAULT_CONFIRMATIONS_K);

    env.exec_user(&ExecuteMsg::BridgeMintNote {
        claim: claim.clone(),
        proof: mock_proof(),
    })
    .expect("mint after UpdateReflection");

    let minted: bool = env.query(&QueryMsg::IsBridgeMinted {
        nullifier: claim.nullifier,
    });
    assert!(minted);
}

#[test]
fn e2e_bridge_not_configured_reject() {
    let mut env = BridgeEnv::new();
    let (_, _, claim, _, _) = bridge_fixture();

    // No SetBridgeCfg / snapshot — mint must fail BridgeNotConfigured.
    let err = env
        .exec_user(&ExecuteMsg::BridgeMintNote {
            claim,
            proof: mock_proof(),
        })
        .expect_err("unconfigured bridge");
    let msg = err.to_string();
    assert!(
        msg.contains("not configured") || msg.contains("BridgeNotConfigured"),
        "unexpected err: {msg}"
    );
}
