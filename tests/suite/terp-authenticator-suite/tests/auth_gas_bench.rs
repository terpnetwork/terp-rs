//! Gas and employment of authenticator sudo in front of a cw721-svg mint.
//!
//! Mock decides whether the authenticator accepts the mint bytes and whether a
//! tampered `sign_mode_direct` is rejected. `cosmwasm-vm` meters the same sudo
//! on the artifact wasm. Output: `bench/auth_gas_bench.json`.
//!
//! ```text
//! rustup run 1.95.0 cargo test -p terp-authenticator-suite --features gas-bench --test auth_gas_bench
//! ```

#[cfg(not(feature = "gas-bench"))]
#[test]
#[ignore = "rustup run 1.95.0 cargo test -p terp-authenticator-suite --features gas-bench --test auth_gas_bench"]
fn auth_gas_bench_requires_gas_bench_feature() {}

#[cfg(feature = "gas-bench")]
mod bench {
    use std::fs;
    use std::path::PathBuf;

    use cosmwasm_std::testing::{message_info, mock_env};
    use cosmwasm_std::{Addr, Binary, Coin, ContractResult, Empty, Env};
    use cosmwasm_vm::testing::{mock_instance_with_options, MockApi, MockInstanceOptions};
    use cosmwasm_vm::{call_execute, call_instantiate, call_sudo, GasReport, Instance, VmResult, CAP_BULK_MEMORY};
    use cw721_svg::{ExecuteMsg, InstantiateMsg, SvgCollectionMetadata, SvgExecuteMsgExt};
    use cw_orch::prelude::*;
    use cw_svg::{compute_template_slots, MintMsg, VariableDef, VariableKind};
    use ed25519_zebra::SigningKey;
    use k256::ecdsa::SigningKey as EthSigningKey;
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use sha3::Keccak256;
    use terp_authenticator_suite::fixtures::{dummy_on_auth_added, dummy_on_auth_removed};
    use terp_authenticator_suite::traits::AuthenticatorSudoExt;
    use terp_authenticator_suite::{
        TerpAuthenticatorDeployData, TerpAuthenticatorSuite, ETH_GOLDEN_SIGNER,
    };
    use terp_auth::{Any, AuthenticationRequest, AuthSudoMsg, SignModeTxData, SignatureData, TxData};
    use terp_authenticator_suite::passkey_fixture::{passkey_auth_payload, passkey_registration};
    use terp_recovery::{
        break_glass_challenge, rehash, GuardianApproval, GuardianConfig, RecoveryAuthPayload,
        RecoveryConfig, RecoveryHashAlg, DEFAULT_DOMAIN,
    };
    use terp_vsck::{ExecuteMsg as VsckExecuteMsg, VsckAuthPayload, VsckCircuit, VsckRole};
    use terp_zkjwt::{build_public_inputs, circuit_ids, compute_msg_bind, ZkJwtAuthPayload};

    const SCHEMA: &str = "terp-auth-gas-bench/v1";
    const CHAIN_ID: &str = "terp-test-1";
    const AUTH_ID: &str = "1";
    const GAS_LIMIT: u64 = 4_000_000_000;
    const ANVIL0_PRIV: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SVG_TEMPLATE: &str = "<svg xmlns='http://www.w3.org/2000/svg'><circle fill='${color}'/></svg>";
    const SESSION_ID: &str = "bench-session";

    #[derive(Serialize)]
    struct BenchFile<'a> {
        schema: &'a str,
        action: Action,
        employed: Vec<&'a str>,
        not_employed: Vec<&'a str>,
        fail_closed: Vec<&'a str>,
        rows: Vec<Row>,
    }

    #[derive(Serialize)]
    struct Action {
        kind: &'static str,
        contract: &'static str,
        msg_bytes: usize,
        msg_sha256: String,
        mint_executed_on: &'static str,
        mint_gas_internal: Option<u64>,
        mint_gas_external: Option<u64>,
        mint_gas_total: Option<u64>,
        mint_gas_note: String,
    }

    #[derive(Serialize)]
    struct Row {
        authenticator: &'static str,
        primitive: &'static str,
        employment: &'static str,
        binds_signed_action: bool,
        authenticate_ok: bool,
        tamper_rejected: bool,
        mint_dispatched: bool,
        auth_gas_internal: u64,
        auth_gas_external: u64,
        auth_gas_total: u64,
        wasm_sha256: String,
        notes: String,
    }

    struct Probe {
        ok: bool,
        tamper_rejected: bool,
        detail: String,
    }

    struct Meter {
        ok: bool,
        internal: u64,
        external: u64,
        detail: String,
    }

    fn bench_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bench")
    }

    fn artifact_wasm(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../artifacts")
            .join(format!("{name}.wasm"));
        fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn variables() -> Vec<VariableDef> {
        vec![VariableDef {
            name: "color".into(),
            kind: VariableKind::Options(vec!["#111111".into(), "#eeeeee".into()]),
        }]
    }

    fn mint_execute() -> ExecuteMsg {
        ExecuteMsg::UpdateExtension {
            msg: SvgExecuteMsgExt::Mint(MintMsg {
                amnt: 1,
                proof_hashes: vec![],
                alloc: 0,
                ext: None,
            }),
        }
    }

    fn svg_init(admin: &str) -> InstantiateMsg {
        let vars = variables();
        InstantiateMsg {
            name: "Bench SVG".into(),
            symbol: "BSVG".into(),
            collection_info_extension: SvgCollectionMetadata {
                seed: Binary::from(b"bench-seed-entropy-value-123456"),
                svg_template: SVG_TEMPLATE.into(),
                template_slots: compute_template_slots(SVG_TEMPLATE, &vars),
                variables: vars,
                price_tiers: vec![],
                total: 100,
                mint_start_time: None,
                mint_end_time: None,
                whitelist: None,
                require_auth: false,
            },
            minter: Some(admin.into()),
            creator: Some(admin.into()),
            withdraw_address: Some(admin.into()),
        }
    }

    fn vm_env(contract: &str) -> Env {
        let mut env = mock_env();
        env.contract.address = Addr::unchecked(contract);
        env
    }

    fn gas_delta(before: GasReport, after: GasReport) -> (u64, u64) {
        let internal = after
            .used_internally
            .checked_sub(before.used_internally)
            .unwrap_or_else(|| panic!("internal gas went backwards: {before:?} -> {after:?}"));
        let external = after
            .used_externally
            .checked_sub(before.used_externally)
            .unwrap_or_else(|| panic!("external gas went backwards: {before:?} -> {after:?}"));
        (internal, external)
    }

    fn expect_vm<T>(result: VmResult<ContractResult<T>>, what: &str) -> Result<T, String> {
        match result {
            Err(err) => Err(format!("{what} vm: {err}")),
            Ok(ContractResult::Err(err)) => Err(format!("{what}: {err}")),
            Ok(ContractResult::Ok(value)) => Ok(value),
        }
    }

    fn metered_instance(wasm: &[u8]) -> Instance<MockApi, cosmwasm_vm::testing::MockStorage, cosmwasm_vm::testing::MockQuerier> {
        let mut options = MockInstanceOptions {
            gas_limit: GAS_LIMIT,
            ..Default::default()
        };
        // Guest artifacts are built with bulk memory. The VM mock set does not advertise it.
        options.available_capabilities.insert(CAP_BULK_MEMORY.to_string());
        mock_instance_with_options(wasm, options)
    }

    fn vm_sudo(wasm: &[u8], contract: &str, admin: &str, init: &[u8], setup: Option<&[u8]>, sudo: &[u8]) -> Meter {
        let mut instance = metered_instance(wasm);
        let env = vm_env(contract);
        let info = message_info(&Addr::unchecked(admin), &[]);
        if let Err(err) = expect_vm(
            call_instantiate::<_, _, _, Empty>(&mut instance, &env, &info, init),
            "instantiate",
        ) {
            return Meter {
                ok: false,
                internal: 0,
                external: 0,
                detail: err,
            };
        }
        if let Some(exec) = setup {
            if let Err(err) = expect_vm(
                call_execute::<_, _, _, Empty>(&mut instance, &env, &info, exec),
                "setup",
            ) {
                return Meter {
                    ok: false,
                    internal: 0,
                    external: 0,
                    detail: err,
                };
            }
        }
        let before = instance.create_gas_report();
        let sudo_res = call_sudo::<_, _, _, Empty>(&mut instance, &env, sudo);
        let after = instance.create_gas_report();
        let (internal, external) = gas_delta(before, after);
        match sudo_res {
            Err(err) => Meter {
                ok: false,
                internal,
                external,
                detail: format!("sudo vm: {err} (before {before:?} after {after:?})"),
            },
            Ok(ContractResult::Ok(_)) => Meter {
                ok: true,
                internal,
                external,
                detail: String::new(),
            },
            Ok(ContractResult::Err(err)) => Meter {
                ok: false,
                internal,
                external,
                detail: err,
            },
        }
    }

    fn auth_request(account: Addr, fee_payer: Addr, signature: Binary, mint: &[u8]) -> AuthenticationRequest {
        let msg = Any {
            type_url: "/cosmwasm.wasm.v1.MsgExecuteContract".into(),
            value: Binary::from(mint.to_vec()),
        };
        AuthenticationRequest {
            authenticator_id: AUTH_ID.into(),
            account: account.clone(),
            fee_payer,
            fee_granter: None,
            fee: vec![Coin::new(1000u128, "uterp")],
            msg: msg.clone(),
            msg_index: 0,
            signature,
            sign_mode_tx_data: SignModeTxData {
                sign_mode_direct: Binary::from(mint.to_vec()),
                sign_mode_textual: None,
            },
            tx_data: TxData {
                chain_id: CHAIN_ID.into(),
                account_number: 1,
                sequence: 0,
                timeout_height: 0,
                msgs: vec![msg],
                memo: String::new(),
            },
            signature_data: SignatureData {
                signers: vec![account],
                signatures: vec![Binary::from(b"unused")],
            },
            simulate: false,
            authenticator_params: None,
        }
    }

    fn tamper_direct(mut req: AuthenticationRequest) -> AuthenticationRequest {
        let mut bytes = req.sign_mode_tx_data.sign_mode_direct.to_vec();
        if bytes.is_empty() {
            bytes.push(0x01);
        } else {
            let last = bytes.len() - 1;
            bytes[last] ^= 0x01;
        }
        req.sign_mode_tx_data.sign_mode_direct = Binary::from(bytes);
        req
    }

    fn sudo_auth(req: AuthenticationRequest) -> Vec<u8> {
        cosmwasm_std::to_json_vec(&AuthSudoMsg::Authenticate(Box::new(req))).unwrap()
    }

    fn ed25519_material(mint: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let sk = SigningKey::from([0x42u8; 32]);
        let sig = sk.sign(mint).to_bytes();
        let vk: [u8; 32] = sk.verification_key().into();
        (vk.to_vec(), sig.as_slice().to_vec())
    }

    fn eth_personal_hash(message: &[u8]) -> [u8; 32] {
        let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
        let mut hasher = Keccak256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(message);
        hasher.finalize().into()
    }

    fn eth_signature(mint: &[u8]) -> Vec<u8> {
        let sk = EthSigningKey::from_slice(&hex::decode(ANVIL0_PRIV).unwrap()).unwrap();
        let (sig, recid) = sk.sign_prehash_recoverable(&eth_personal_hash(mint)).unwrap();
        let mut out = sig.to_bytes().as_slice().to_vec();
        out.push(recid.to_byte());
        out
    }

    fn recovery_signature(guardian: &str, account: &str, mint: &[u8]) -> Binary {
        let challenge = break_glass_challenge(
            RecoveryHashAlg::Sha256,
            1,
            DEFAULT_DOMAIN,
            CHAIN_ID,
            account,
            AUTH_ID,
            mint,
        )
        .unwrap();
        let mut preimage = guardian.as_bytes().to_vec();
        preimage.push(0);
        preimage.extend_from_slice(&challenge);
        let digest = rehash(RecoveryHashAlg::Sha256, &preimage, 1).unwrap();
        cosmwasm_std::to_json_binary(&RecoveryAuthPayload {
            hash_alg: RecoveryHashAlg::Sha256,
            rehash_rounds: 1,
            preimage_proof: None,
            approvals: vec![GuardianApproval {
                guardian: guardian.into(),
                signature: Binary::from(digest),
            }],
        })
        .unwrap()
    }

    fn passkey_signature(mint: &[u8]) -> Binary {
        cosmwasm_std::to_json_binary(&passkey_auth_payload(mint)).unwrap()
    }

    fn jwt_signature(account: &str, mint: &[u8], nullifier: [u8; 32], with_bind: bool, proof: &[u8]) -> Binary {
        let claim = [0x11u8; 32];
        let root = [0xABu8; 32];
        let bind = with_bind.then(|| compute_msg_bind(CHAIN_ID, account, AUTH_ID, 0, mint));
        cosmwasm_std::to_json_binary(&ZkJwtAuthPayload {
            issuer: "https://accounts.example.com".into(),
            claim_commitment: Binary::from(claim.to_vec()),
            public_inputs: build_public_inputs(&nullifier, &claim, Some(&root), bind.as_ref()),
            proof: Binary::from(proof.to_vec()),
            circuit_id: circuit_ids::JWT_MEMBERSHIP.into(),
        })
        .unwrap()
    }

    fn note_root() -> Binary {
        Binary::from(b"note-tree-root-32bytes___________")
    }

    fn vsck_signature() -> Binary {
        let mut public_inputs = note_root().to_vec();
        public_inputs.extend_from_slice(b"|ballot");
        cosmwasm_std::to_json_binary(&VsckAuthPayload {
            session_id: SESSION_ID.into(),
            role: VsckRole::Voter,
            circuit: VsckCircuit::VoteProof,
            public_inputs: Binary::from(public_inputs),
            proof: Binary::from(b"stub-proof"),
            nullifier: Binary::from(b"nullifier"),
        })
        .unwrap()
    }

    fn mock_probe<C>(contract: &C, good: AuthenticationRequest) -> Probe
    where
        C: ContractInstance<Mock> + terp_authenticator_suite::AuthSudoContract,
    {
        let bad = tamper_direct(good.clone());
        let ok_res = contract.sudo_authenticate(good);
        let bad_res = contract.sudo_authenticate(bad);
        let ok = ok_res.is_ok();
        let tamper_rejected = bad_res.is_err();
        let detail = match (ok_res, bad_res) {
            (Err(err), _) => format!("authenticate: {err}"),
            (Ok(_), Ok(_)) => "tampered sign_mode_direct still authenticated".into(),
            (Ok(_), Err(_)) => String::new(),
        };
        Probe {
            ok,
            tamper_rejected,
            detail,
        }
    }

    fn finish_row(
        authenticator: &'static str,
        primitive: &'static str,
        employment: &'static str,
        wasm_name: &str,
        probe: Probe,
        meter: Meter,
        mint_dispatched: bool,
        notes: String,
    ) -> Row {
        if probe.ok != meter.ok {
            panic!(
                "{authenticator} mock authenticate_ok={} but vm authenticate_ok={} ({})",
                probe.ok, meter.ok, meter.detail
            );
        }
        let wasm = artifact_wasm(wasm_name);
        let mut notes = notes;
        if !probe.detail.is_empty() {
            if !notes.is_empty() {
                notes.push(' ');
            }
            notes.push_str(&probe.detail);
        }
        if !meter.detail.is_empty() && employment != "fail_closed_unlinked" && !probe.ok {
            if !notes.is_empty() {
                notes.push(' ');
            }
            notes.push_str(&meter.detail);
        }
        if employment == "fail_closed_unlinked" {
            assert!(
                meter.detail.contains("not linked"),
                "{authenticator} expected unlinked circuit, got {}",
                meter.detail
            );
        }
        Row {
            authenticator,
            primitive,
            employment,
            binds_signed_action: probe.ok && probe.tamper_rejected,
            authenticate_ok: probe.ok,
            tamper_rejected: probe.tamper_rejected,
            mint_dispatched,
            auth_gas_internal: meter.internal,
            auth_gas_external: meter.external,
            auth_gas_total: meter.internal.saturating_add(meter.external),
            wasm_sha256: sha256_hex(&wasm),
            notes,
        }
    }

    fn mint_gas(admin: &str, mint: &[u8]) -> (Option<u64>, Option<u64>, String) {
        let candidates = [
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../artifacts/cw721_svg.wasm"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/wasm32-unknown-unknown/release/cw721_svg.wasm"),
        ];
        let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
            return (
                None,
                None,
                "cw721_svg.wasm absent; mint ran on cw-orch Mock and mint_gas is null".into(),
            );
        };
        let wasm = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let init = cosmwasm_std::to_json_vec(&svg_init(admin)).unwrap();
        let api = MockApi::default();
        let contract = api.addr_make("cw721");
        let mut instance = metered_instance(&wasm);
        let env = vm_env(&contract);
        let info = message_info(&Addr::unchecked(admin), &[]);
        if let Err(err) = expect_vm(
            call_instantiate::<_, _, _, Empty>(&mut instance, &env, &info, &init),
            "cw721 instantiate",
        ) {
            return (None, None, format!("cw721 wasm instantiate failed: {err}"));
        }
        let before = instance.create_gas_report();
        let executed = call_execute::<_, _, _, Empty>(&mut instance, &env, &info, mint);
        let after = instance.create_gas_report();
        match executed {
            Ok(ContractResult::Ok(_)) => {
                let (internal, external) = gas_delta(before, after);
                (
                    Some(internal),
                    Some(external),
                    format!("metered {}", path.display()),
                )
            }
            Ok(ContractResult::Err(err)) => (None, None, format!("cw721 wasm mint: {err}")),
            Err(err) => (None, None, format!("cw721 wasm mint vm: {err}")),
        }
    }

    fn dispatch_mint(svg: &cw721_svg::Cw721SvgContractSuite<Mock>) -> bool {
        svg.execute(&mint_execute(), &[])
            .unwrap_or_else(|err| panic!("svg mint: {err}"));
        true
    }

    #[test]
    fn auth_gas_bench_svg_mint() {
        let chain = Mock::new("admin");
        let admin = chain.sender_addr();
        let suite = TerpAuthenticatorSuite::deploy_on(
            chain.clone(),
            TerpAuthenticatorDeployData {
                admin: admin.clone(),
                dao: Some(admin.clone()),
                recovery_hash_alg: None,
                require_registered_claim: false,
                eth_signer: ETH_GOLDEN_SIGNER.into(),
            },
        )
        .expect("deploy authenticators");

        let mint = cosmwasm_std::to_json_vec(&mint_execute()).unwrap();
        let (ed_pk, ed_sig) = ed25519_material(&mint);
        suite
            .ed25519
            .sudo_on_auth_removed(dummy_on_auth_removed(admin.clone(), AUTH_ID, None))
            .expect("clear ed25519 key");
        suite
            .ed25519
            .sudo_on_auth_added(dummy_on_auth_added(
                admin.clone(),
                AUTH_ID,
                Some(Binary::from(ed_pk.clone())),
            ))
            .expect("install ed25519 key");

        let svg = cw721_svg::Cw721SvgContractSuite::new(chain.clone());
        svg.upload().expect("upload cw721-svg");
        svg.instantiate(&svg_init(admin.as_str()), Some(&admin), &[])
            .expect("instantiate cw721-svg");

        let fee = admin.clone();
        let account = admin.clone();

        let api = MockApi::default();
        let vm_admin = api.addr_make("admin");
        let vm_account = api.addr_make("account");
        let vm_fee = api.addr_make("fee");
        let vm_contract = api.addr_make("authenticator");

        let mut rows = Vec::new();

        // ed25519
        {
            let good = auth_request(account.clone(), fee.clone(), Binary::from(ed_sig.clone()), &mint);
            let probe = mock_probe(&suite.ed25519, good);
            let vm_init = cosmwasm_std::to_json_vec(&terp_ed25519::InstantiateMsg {
                owner: Some(vm_admin.clone()),
                pubkey: Binary::from(ed_pk),
            })
            .unwrap();
            let vm_req = auth_request(
                Addr::unchecked(&vm_account),
                Addr::unchecked(&vm_fee),
                Binary::from(ed_sig),
                &mint,
            );
            let meter = vm_sudo(
                &artifact_wasm("terp_ed25519"),
                &vm_contract,
                &vm_admin,
                &vm_init,
                None,
                &sudo_auth(vm_req),
            );
            let dispatched = probe.ok && dispatch_mint(&svg);
            rows.push(finish_row(
                "terp-ed25519",
                "ed25519_verify",
                "signature",
                "terp_ed25519",
                probe,
                meter,
                dispatched,
                "signs sign_mode_direct, which is the mint execute JSON".into(),
            ));
        }

        // eth
        {
            let sig = eth_signature(&mint);
            let good = auth_request(account.clone(), fee.clone(), Binary::from(sig.clone()), &mint);
            let probe = mock_probe(&suite.eth, good);
            let vm_init = cosmwasm_std::to_json_vec(&terp_eth::InstantiateMsg {
                owner: Some(vm_admin.clone()),
                signer: ETH_GOLDEN_SIGNER.into(),
            })
            .unwrap();
            let vm_req = auth_request(
                Addr::unchecked(&vm_account),
                Addr::unchecked(&vm_fee),
                Binary::from(sig),
                &mint,
            );
            let meter = vm_sudo(
                &artifact_wasm("terp_eth"),
                &vm_contract,
                &vm_admin,
                &vm_init,
                None,
                &sudo_auth(vm_req),
            );
            let dispatched = probe.ok && dispatch_mint(&svg);
            rows.push(finish_row(
                "terp-eth",
                "secp256k1_recover_pubkey",
                "signature",
                "terp_eth",
                probe,
                meter,
                dispatched,
                "EIP-191 personal_sign over sign_mode_direct (the mint JSON), Anvil account 0".into(),
            ));
        }

        // recovery — challenge includes the account, so Mock and VM attestations differ
        {
            let sig = recovery_signature(admin.as_str(), account.as_str(), &mint);
            let good = auth_request(account.clone(), fee.clone(), sig, &mint);
            let probe = mock_probe(&suite.recovery, good);
            let vm_sig = recovery_signature(&vm_admin, &vm_account, &mint);
            let vm_init = cosmwasm_std::to_json_vec(&terp_recovery::InstantiateMsg {
                config: RecoveryConfig {
                    guardians: vec![GuardianConfig {
                        address: vm_admin.clone(),
                        pubkey: None,
                        commitment: None,
                    }],
                    threshold: 1,
                    hash_alg: RecoveryHashAlg::Sha256,
                    rehash_rounds: 1,
                    domain: DEFAULT_DOMAIN.into(),
                    allow_address_only_approvals: true,
                },
            })
            .unwrap();
            let vm_req = auth_request(Addr::unchecked(&vm_account), Addr::unchecked(&vm_fee), vm_sig, &mint);
            let meter = vm_sudo(
                &artifact_wasm("terp_recovery"),
                &vm_contract,
                &vm_admin,
                &vm_init,
                None,
                &sudo_auth(vm_req),
            );
            let dispatched = probe.ok && dispatch_mint(&svg);
            rows.push(finish_row(
                "terp-recovery",
                "sha256_rehash",
                "hash_attestation",
                "terp_recovery",
                probe,
                meter,
                dispatched,
                "address-only SHA-256 attestation of a challenge that includes sign_mode_direct. No guardian signature.".into(),
            ));
        }

        // passkey
        {
            let sig = passkey_signature(&mint);
            let good = auth_request(account.clone(), fee.clone(), sig, &mint);
            let probe = mock_probe(&suite.passkey, good);
            let empty = cosmwasm_std::to_json_binary(&terp_passkey::PasskeyAuthPayload {
                origin: Some("https://app.terp.network".into()),
                credential_id: Binary::from(b"cred-id-1"),
                assertion: Binary::default(),
                authenticator_data: Binary::from(b"ad"),
                client_data_json: Binary::from(b"{}"),
            })
            .unwrap();
            let empty_rejected = suite
                .passkey
                .sudo_authenticate(auth_request(account.clone(), fee.clone(), empty, &mint))
                .is_err();
            let vm_init = cosmwasm_std::to_json_vec(&terp_passkey::InstantiateMsg {
                owner: Some(vm_admin.clone()),
                registration: passkey_registration(),
            })
            .unwrap();
            let vm_req = auth_request(
                Addr::unchecked(&vm_account),
                Addr::unchecked(&vm_fee),
                passkey_signature(&mint),
                &mint,
            );
            let meter = vm_sudo(
                &artifact_wasm("terp_passkey"),
                &vm_contract,
                &vm_admin,
                &vm_init,
                None,
                &sudo_auth(vm_req),
            );
            let dispatched = probe.ok && dispatch_mint(&svg);
            rows.push(finish_row(
                "terp-passkey",
                "secp256r1",
                "signature",
                "terp_passkey",
                probe,
                meter,
                dispatched,
                format!(
                    "ES256 over authenticatorData || SHA-256(clientDataJSON). Challenge is sign_mode_direct. Empty assertion rejected: {empty_rejected}."
                ),
            ));
        }

        // zk-jwt structural envelope. Omitted msg_bind still authenticates.
        {
            let omitted = jwt_signature(account.as_str(), &mint, [0x01u8; 32], false, b"envelope-proof");
            let omitted_ok = suite
                .zk_jwt
                .sudo_authenticate(auth_request(account.clone(), fee.clone(), omitted, &mint))
                .is_ok();
            let bound = jwt_signature(account.as_str(), &mint, [0x02u8; 32], true, b"envelope-proof");
            let good = auth_request(account.clone(), fee.clone(), bound, &mint);
            let probe = mock_probe(&suite.zk_jwt, good);
            let empty_rejected = suite
                .zk_jwt
                .sudo_authenticate(auth_request(
                    account.clone(),
                    fee.clone(),
                    jwt_signature(account.as_str(), &mint, [0x03u8; 32], true, b""),
                    &mint,
                ))
                .is_err();
            let vm_init = cosmwasm_std::to_json_vec(&terp_zkjwt::InstantiateMsg {
                admin: Some(vm_admin.clone()),
                issuers: vec![terp_zkjwt::IssuerConfig {
                    issuer: "https://accounts.example.com".into(),
                    verifying_key: Binary::from(b"test-issuer-vk"),
                    zkid: Some(1),
                    audience: Some("terp-app".into()),
                    inclusion_set_root: Some(Binary::from([0xABu8; 32])),
                }],
                require_registered_claim: false,
            })
            .unwrap();
            // Gas the bound call. A fresh VM instance has not spent the nullifier.
            let vm_req = auth_request(
                Addr::unchecked(&vm_account),
                Addr::unchecked(&vm_fee),
                jwt_signature(&vm_account, &mint, [0x02u8; 32], true, b"envelope-proof"),
                &mint,
            );
            let meter = vm_sudo(
                &artifact_wasm("terp_zkjwt"),
                &vm_contract,
                &vm_admin,
                &vm_init,
                None,
                &sudo_auth(vm_req),
            );
            let mut probe = probe;
            // Binding is optional: a payload that omits msg_bind still passes.
            if omitted_ok {
                probe.tamper_rejected = false;
            }
            let dispatched = probe.ok && dispatch_mint(&svg);
            rows.push(finish_row(
                "terp-zkjwt",
                "structural_envelope",
                "envelope_only",
                "terp_zkjwt",
                probe,
                meter,
                dispatched,
                format!(
                    "default verifier checks the envelope only. Empty proof rejected: {empty_rejected}. Omitted msg_bind still authenticates: {omitted_ok}. Supplied msg_bind is checked against sign_mode_direct."
                ),
            ));
        }

        // vsck fail-closed. Open the session, then meter the rejecting sudo. Do not mint.
        {
            suite
                .vsck
                .execute(
                    &VsckExecuteMsg::OpenSession {
                        session_id: SESSION_ID.into(),
                        note_tree_root: note_root(),
                        proposal_ref: Some("cw721-svg-mint".into()),
                    },
                    &[],
                )
                .expect("open vsck session");
            let good = auth_request(account.clone(), fee.clone(), vsck_signature(), &mint);
            let ok_res = suite.vsck.sudo_authenticate(good);
            let probe = Probe {
                ok: ok_res.is_ok(),
                tamper_rejected: ok_res.is_err(),
                detail: ok_res.err().map(|err| err.to_string()).unwrap_or_default(),
            };
            let vm_init = cosmwasm_std::to_json_vec(&terp_vsck::InstantiateMsg {
                admin: Some(vm_admin.clone()),
                dao: vm_admin.clone(),
                circuit_vks: terp_vsck::CircuitVerifyingKeys {
                    delegation_vk: Binary::from(b"delegation-vk"),
                    vote_proof_vk: Binary::from(b"vote-proof-vk"),
                    share_reveal_vk: Binary::from(b"share-reveal-vk"),
                },
            })
            .unwrap();
            let open = cosmwasm_std::to_json_vec(&VsckExecuteMsg::OpenSession {
                session_id: SESSION_ID.into(),
                note_tree_root: note_root(),
                proposal_ref: Some("cw721-svg-mint".into()),
            })
            .unwrap();
            let vm_req = auth_request(
                Addr::unchecked(&vm_account),
                Addr::unchecked(&vm_fee),
                vsck_signature(),
                &mint,
            );
            let meter = vm_sudo(
                &artifact_wasm("terp_vsck"),
                &vm_contract,
                &vm_admin,
                &vm_init,
                Some(&open),
                &sudo_auth(vm_req),
            );
            rows.push(finish_row(
                "terp-vsck",
                "circuit_not_linked",
                "fail_closed_unlinked",
                "terp_vsck",
                probe,
                meter,
                false,
                "OpenSession is setup and is not included in auth_gas. Stub proofs are rejected. Mint is not dispatched.".into(),
            ));
        }

        rows.sort_by(|a, b| a.authenticator.cmp(b.authenticator));

        for row in &rows {
            assert!(row.auth_gas_total > 0, "{} gas was zero", row.authenticator);
            match row.employment {
                "signature" | "hash_attestation" => {
                    assert!(row.authenticate_ok, "{}", row.authenticator);
                    assert!(row.tamper_rejected, "{}", row.authenticator);
                    assert!(row.binds_signed_action, "{}", row.authenticator);
                    assert!(row.mint_dispatched, "{}", row.authenticator);
                }
                "shape_only" | "scaffold" | "envelope_only" => {
                    assert!(row.authenticate_ok, "{}", row.authenticator);
                    assert!(!row.binds_signed_action, "{}", row.authenticator);
                    assert!(row.mint_dispatched, "{}", row.authenticator);
                    assert!(!row.tamper_rejected, "{}", row.authenticator);
                }
                "fail_closed_unlinked" => {
                    assert!(!row.authenticate_ok, "{}", row.authenticator);
                    assert!(!row.mint_dispatched, "{}", row.authenticator);
                }
                other => panic!("unknown employment {other}"),
            }
        }

        let (mint_internal, mint_external, mint_note) = mint_gas(&vm_admin, &mint);
        let file = BenchFile {
            schema: SCHEMA,
            action: Action {
                kind: "cw721-svg-mint",
                contract: "cw721-svg",
                msg_bytes: mint.len(),
                msg_sha256: sha256_hex(&mint),
                mint_executed_on: "cw-orch-mock",
                mint_gas_internal: mint_internal,
                mint_gas_external: mint_external,
                mint_gas_total: match (mint_internal, mint_external) {
                    (Some(i), Some(e)) => Some(i.saturating_add(e)),
                    _ => None,
                },
                mint_gas_note: mint_note,
            },
            employed: rows
                .iter()
                .filter(|row| matches!(row.employment, "signature" | "hash_attestation"))
                .map(|row| row.authenticator)
                .collect(),
            not_employed: rows
                .iter()
                .filter(|row| matches!(row.employment, "shape_only" | "scaffold" | "envelope_only"))
                .map(|row| row.authenticator)
                .collect(),
            fail_closed: rows
                .iter()
                .filter(|row| row.employment == "fail_closed_unlinked")
                .map(|row| row.authenticator)
                .collect(),
            rows,
        };

        let dir = bench_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth_gas_bench.json");
        let body = serde_json::to_string_pretty(&file).unwrap();
        fs::write(&path, format!("{body}\n")).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
