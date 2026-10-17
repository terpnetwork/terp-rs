//! Multitest e2e for Option D `BridgeEgressBurn` on `cw-headstash`.
//!
//! Exercises the real execute/query path via `cw-multi-test` ContractWrapper:
//!
//! 1. Instantiate headstash (ExistingFungible + funds)
//! 2. Owner: SetBridgeCfg (mock_verify=true), RegisterAsset (ZEC)
//! 3. BridgeEgressBurn happy path → IsEgressSpent true + burn_evidence attrs
//! 4. Double burn same ν → reject
//! 5. Dest mismatch (owner_binding ≠ dest_commitment) → reject
//! 6. Empty proof under mock_verify → reject
//!
//! Proof verify is lab dual-path (`BridgeCfg.mock_verify`); no Halo2 this wave.

use cosmwasm_std::{Addr, Binary, Coin, Uint128};
use cw_headstash::bridge::{
    hash32_label, AssetStatus, BridgeCfg, DEFAULT_CONFIRMATIONS_K, DEFAULT_MAX_LC_LAG, Hash32,
};
use cw_headstash::egress::{
    egress_nullifier, happy_egress_statement, mock_egress_proof_bytes, DestKind,
    EgressBurnStatement,
};
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use cw_headstash::tokenfactory::{HeadstashTokenObject, TokenStrategy};
use cw_headstash::wavs::{WavsAuthMetadata, WavsOpAuth, WavsProofOfOwnership};
use cw_multi_test::{App, ContractWrapper, Executor};

fn bin32(h: Hash32) -> Binary {
    Binary::from(h.to_vec())
}

/// Deterministic valid WAVS PoO (1 operator) so instantiate's pairing check passes.
fn valid_wavs_proof_one() -> WavsProofOfOwnership {
    use ark_bls12_381::{Fr, G1Affine, G2Affine};
    use ark_ec::AffineRepr;
    use ark_ff::{PrimeField, Zero};
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use cosmwasm_std::{testing::MockApi, Api, BLS12_381_G2_GENERATOR as G2, HashFunction};

    let api = MockApi::default();
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

struct EgressEnv {
    app: App,
    owner: Addr,
    user: Addr,
    headstash: Addr,
}

impl EgressEnv {
    fn new() -> Self {
        let mut app = App::default();
        let owner = app.api().addr_make("owner");
        let user = app.api().addr_make("egress_user");

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
            genesis_label: Some("egress-e2e".into()),
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
                "cw-headstash-egress-e2e",
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

    fn exec_user(
        &mut self,
        msg: &ExecuteMsg,
    ) -> Result<cw_multi_test::AppResponse, anyhow::Error> {
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

    /// Owner: mock_verify bridge cfg + register ZEC asset from happy statement.
    fn setup_egress_happy(&mut self) -> EgressBurnStatement {
        let statement = happy_egress_statement();
        let dest = hash32_label("terp-chain-1");
        let cfg = BridgeCfg {
            dest_domain: bin32(dest),
            confirmations_k: DEFAULT_CONFIRMATIONS_K,
            max_lc_lag: DEFAULT_MAX_LC_LAG,
            lc_client_id: "08-wasm-tacit-reflection-0".into(),
            // Integration tests compile the library without cfg(test) on the crate.
            mock_verify: true,
            egress_zkid: None,
        };
        self.exec_owner(&ExecuteMsg::SetBridgeCfg { cfg });
        self.exec_owner(&ExecuteMsg::RegisterAsset {
            asset_id: statement.asset_id.clone(),
            local_denom: "uzec".into(),
            origin: Some("registry:zec".into()),
            status: Some(AssetStatus::Active),
        });

        let stored: Option<BridgeCfg> = self.query(&QueryMsg::BridgeConfig {});
        assert!(stored.unwrap().mock_verify);

        statement
    }
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

#[test]
fn e2e_egress_burn_happy_path_is_spent() {
    let mut env = EgressEnv::new();
    let statement = env.setup_egress_happy();

    let spent_before: bool = env.query(&QueryMsg::IsEgressSpent {
        nullifier: statement.nullifier.clone(),
    });
    assert!(!spent_before);

    let res = env
        .exec_user(&ExecuteMsg::BridgeEgressBurn {
            statement: statement.clone(),
            proof: mock_egress_proof_bytes(),
        })
        .expect("BridgeEgressBurn happy");

    let attr = |key: &str, value: &str| {
        res.events.iter().any(|e| {
            e.attributes
                .iter()
                .any(|a| a.key == key && a.value == value)
        })
    };
    assert!(
        attr("action", "bridge_egress_burn"),
        "expected bridge_egress_burn action, events={:?}",
        res.events
    );
    assert!(attr("proof_mode", "mock_verify_lab"));
    assert!(attr("dest_kind", "shielded"));

    let spent_after: bool = env.query(&QueryMsg::IsEgressSpent {
        nullifier: statement.nullifier.clone(),
    });
    assert!(spent_after, "IsEgressSpent must be true after happy burn");

    // burn_evidence payload present
    let has_evidence = res.events.iter().any(|e| {
        e.attributes.iter().any(|a| a.key == "burn_evidence")
    });
    assert!(has_evidence, "expected burn_evidence attribute");
}

#[test]
fn e2e_egress_double_burn_same_nu_reject() {
    let mut env = EgressEnv::new();
    let statement = env.setup_egress_happy();

    env.exec_user(&ExecuteMsg::BridgeEgressBurn {
        statement: statement.clone(),
        proof: mock_egress_proof_bytes(),
    })
    .expect("first burn");

    let err = env
        .exec_user(&ExecuteMsg::BridgeEgressBurn {
            statement: statement.clone(),
            proof: mock_egress_proof_bytes(),
        })
        .expect_err("second burn must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("already spent") || msg.contains("double") || msg.contains("AlreadySpent"),
        "unexpected err: {msg}"
    );

    let spent: bool = env.query(&QueryMsg::IsEgressSpent {
        nullifier: statement.nullifier,
    });
    assert!(spent);
}

#[test]
fn e2e_egress_dest_mismatch_reject() {
    let mut env = EgressEnv::new();
    let mut statement = env.setup_egress_happy();
    // Redirect attack: owner_binding ≠ dest_commitment
    statement.owner_binding = bin32(hash32_label("redirect-attack-dest"));

    let err = env
        .exec_user(&ExecuteMsg::BridgeEgressBurn {
            statement: statement.clone(),
            proof: mock_egress_proof_bytes(),
        })
        .expect_err("dest mismatch must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("dest") || msg.contains("mismatch") || msg.contains("DestMismatch"),
        "unexpected err: {msg}"
    );

    let spent: bool = env.query(&QueryMsg::IsEgressSpent {
        nullifier: statement.nullifier,
    });
    assert!(!spent, "failed burn must not set IsEgressSpent");
}

#[test]
fn e2e_egress_empty_proof_under_mock_reject() {
    let mut env = EgressEnv::new();
    let statement = env.setup_egress_happy();

    let err = env
        .exec_user(&ExecuteMsg::BridgeEgressBurn {
            statement: statement.clone(),
            proof: Binary::default(),
        })
        .expect_err("empty proof under mock must reject");
    let msg = err.to_string();
    assert!(
        msg.contains("proof") || msg.contains("rejected") || msg.contains("ProofRejected"),
        "unexpected err: {msg}"
    );

    let spent: bool = env.query(&QueryMsg::IsEgressSpent {
        nullifier: statement.nullifier,
    });
    assert!(!spent);
}

#[test]
fn e2e_egress_transparent_dest_kind_ok() {
    let mut env = EgressEnv::new();
    let mut statement = env.setup_egress_happy();
    // Re-derive with transparent kind (metadata only).
    statement.dest_kind = DestKind::Transparent;

    let res = env
        .exec_user(&ExecuteMsg::BridgeEgressBurn {
            statement: statement.clone(),
            proof: mock_egress_proof_bytes(),
        })
        .expect("transparent dest kind ok");

    let attr = |key: &str, value: &str| {
        res.events.iter().any(|e| {
            e.attributes
                .iter()
                .any(|a| a.key == key && a.value == value)
        })
    };
    assert!(attr("dest_kind", "transparent"));
    let spent: bool = env.query(&QueryMsg::IsEgressSpent {
        nullifier: statement.nullifier,
    });
    assert!(spent);
}

#[test]
fn e2e_egress_nullifier_domain_is_egress_nf_v0() {
    // Sanity: fixture ν matches pure domain formula.
    let cm = hash32_label("cm-zec-seam-out-1");
    let rcm = hash32_label("rcm-zec-egress-1");
    let nu = egress_nullifier(&cm, &rcm);
    let s = happy_egress_statement();
    assert_eq!(s.nullifier.as_slice(), nu.as_slice());
}
