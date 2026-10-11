//! Instantiate gas for the reply voting module and the instantiate2 voting
//! module, on the recut `v6.4.0` image, through the cw-orch `DaoDaoSuite`.
//!
//! ict-rs starts terpd. `daemon_builder_from_chain` is the same connection
//! `dao_actions_chain` uses. Each framework is uploaded with
//! `DaoDaoSuite` (`dao_core`, `prop_single`, `voting_cw4`, `cw4_group`) and
//! instantiated with [`dao_foundation_suite::instantiate_msg`]. `ARTIFACTS_DIR`
//! selects the wasm directory. Transaction `gas_used` is the comparison.
//! `Reply.GasUsed` stays 0.
//!
//! ```text
//! ICT_DAO_GAS=1 cargo test -p dao-foundation-suite --test dao_reply_gas \
//!   -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use cosmwasm_std::Addr;
use cw_orch::daemon::CosmTxResponse;
use cw_orch::prelude::*;
use dao_foundation_suite::{instantiate_msg, voting_module, Codes, VotingKind};
use dao_interface::msg::QueryMsg;
use dao_testing::DaoDaoSuite;
use dao_voting_cw4::msg::QueryMsg as Cw4Query;
use ict_rs::chain::GenesisStyle;
use ict_rs::prelude::generate_mnemonic;
use ict_rs::testing::{TestChain, TestChainConfig, TestEnv};
use ict_rs::tx::WalletAmount;
use ict_rs::wallet::KeyWallet;
use ict_rs_cw_orch::daemon_builder_from_chain;
use serde_json::{json, Value};

const IMAGE_REPO: &str = "registry.terp.network/terp-core";
const IMAGE_VERSION: &str = "v6.4.0";

const REPLY_VOTING: &str = "84e520f80fc115a2079357dfada5ebbaf4564df8d618f1ba5c3e0e1ad0966be8";
const INIT2_VOTING: &str = "668b2966d37f6eea7e544b6bc584443965a2a05cfed8b5a70e65c11fcf43c23b";

struct GasRow {
    design: &'static str,
    gas_wanted: u64,
    gas_used: u64,
    txhash: String,
}

#[tokio::test]
#[ignore = "docker: registry.terp.network/terp-core:v6.4.0 ICT_DAO_GAS=1"]
async fn reply_vs_instantiate2_gas() {
    if std::env::var("ICT_DAO_GAS").ok().as_deref() != Some("1") {
        panic!("set ICT_DAO_GAS=1 before this ignored test");
    }
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("ICT_MOCK does not run wasmd");
    }
    // Latest recut. Ambient TERP_IMAGE_VERSION must not select an older image.
    unsafe {
        std::env::set_var("TERP_IMAGE_REPO", IMAGE_REPO);
        std::env::set_var("TERP_IMAGE_VERSION", IMAGE_VERSION);
    }
    assert!(
        !image_has_allowlist(IMAGE_REPO, IMAGE_VERSION),
        "{IMAGE_REPO}:{IMAGE_VERSION} still contains filterWasmReplyEvents"
    );

    let reply_dir = stage_reply_dir();
    let init2_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../dao-contracts/artifacts/init2");
    assert_sha(&reply_dir.join("dao_voting_cw4.wasm"), REPLY_VOTING);
    assert_sha(&init2_dir.join("dao_voting_cw4.wasm"), INIT2_VOTING);
    println!("image {IMAGE_REPO}:{IMAGE_VERSION}");
    println!("reply voting sha256 {REPLY_VOTING} morocco-1 code 56");
    println!("init2 voting sha256 {INIT2_VOTING} artifacts/init2 (not a morocco code id)");

    let mnemonic = generate_mnemonic();
    let member = KeyWallet::from_mnemonic("member", &mnemonic, "terp", 118).expect("member wallet");

    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = "dao-gas-v640".into();
    cfg.gas_prices = "0uterp".into();
    cfg.gas_adjustment = 2.5;
    cfg.block_time = "500ms".into();
    cfg.genesis_style = GenesisStyle::Legacy;
    cfg.config_file_overrides.insert(
        "config/config.toml".into(),
        json!({
            "rpc": { "max_body_bytes": 104_857_600 },
            "mempool": { "max_tx_bytes": 104_857_600 }
        }),
    );
    cfg.modify_genesis = Some(Box::new(|_cfg, raw| {
        let mut genesis: Value = serde_json::from_slice(&raw)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis json: {err}")))?;
        if let Some(block) = genesis.pointer_mut("/consensus_params/block") {
            block["max_gas"] = json!("-1");
            block["max_bytes"] = json!("104857600");
        }
        if let Some(params) = genesis.pointer_mut("/app_state/wasm/params") {
            params["code_upload_access"] = json!({"permission": "Everybody", "addresses": []});
            params["instantiate_default_permission"] = json!("Everybody");
        }
        serde_json::to_vec(&genesis)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis encode: {err}")))
    }));

    let mut chain = TestChain::setup(
        "daogasv640",
        TestChainConfig {
            chain_config: cfg,
            num_validators: 1,
            num_full_nodes: 0,
            genesis_wallets: vec![WalletAmount {
                address: member.bech32_address.clone(),
                denom: "uterp".into(),
                amount: 50_000_000_000,
            }],
        },
    )
    .await
    .expect("local terp chain");

    let reply_dir = reply_dir.clone();
    let init2_dir = init2_dir.clone();
    let member_addr = member.bech32_address.clone();
    let exercise = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut builder = daemon_builder_from_chain(&chain.chain, Some(&mnemonic))
                    .expect("daemon builder");
                builder.load_network(false);
                let daemon = builder.build().expect("daemon");
                assert_eq!(daemon.sender_addr().as_str(), member_addr);
                let mut rows = Vec::new();
                rows.push(framework(&daemon, "reply", &reply_dir));
                rows.push(framework(&daemon, "init2", &init2_dir));
                rows
            }))
        });
        handle.join().expect("daemon thread")
    });
    if exercise.is_err() {
        if let Some(node) = chain.chain.validators().first() {
            let log = node.read_chain_log(120).await;
            println!("chain.log tail:\n{log}");
        }
        chain.dump_logs().await;
    }
    let cleanup = chain.cleanup().await;
    let rows = match exercise {
        Ok(rows) => rows,
        Err(payload) => {
            let _ = cleanup;
            std::panic::resume_unwind(payload);
        }
    };
    cleanup.expect("chain cleanup");

    let reply = rows.iter().find(|row| row.design == "reply").unwrap();
    let init2 = rows.iter().find(|row| row.design == "init2").unwrap();
    println!(
        "GAS design=reply gas_used={} gas_wanted={} txhash={}",
        reply.gas_used, reply.gas_wanted, reply.txhash
    );
    println!(
        "GAS design=init2 gas_used={} gas_wanted={} txhash={}",
        init2.gas_used, init2.gas_wanted, init2.txhash
    );
    let lower = if reply.gas_used < init2.gas_used {
        "reply"
    } else if init2.gas_used < reply.gas_used {
        "init2"
    } else {
        "tie"
    };
    println!(
        "GAS lower_gas_used={lower} reply={} init2={} delta_init2_minus_reply={}",
        reply.gas_used,
        init2.gas_used,
        init2.gas_used as i64 - reply.gas_used as i64
    );
}

fn framework(daemon: &Daemon, design: &'static str, artifacts: &Path) -> GasRow {
    unsafe {
        std::env::set_var("ARTIFACTS_DIR", artifacts);
    }
    let suite = DaoDaoSuite::new(daemon.clone());
    suite.dao_core.upload().expect("upload dao core");
    suite.proposal.prop_single.upload().expect("upload proposal");
    suite.voting.voting_cw4.upload().expect("upload voting cw4");
    suite.voting.cw4_group.upload().expect("upload cw4 group");
    let codes = Codes {
        core: suite.dao_core.code_id().expect("core code"),
        proposal_single: suite.proposal.prop_single.code_id().expect("proposal code"),
        cw4_group: suite.voting.cw4_group.code_id().expect("cw4 code"),
        voting_cw4: suite.voting.voting_cw4.code_id().expect("voting code"),
        cw20_base: 0,
        cw20_stake: 0,
        voting_cw20_staked: 0,
        cw721_base: 0,
        voting_cw721_staked: 0,
        cw721_roles: 0,
        voting_cw721_roles: 0,
        voting_cw20_balance: 0,
    };
    let member = daemon.sender_addr();
    let name = format!("{design}-dao");
    let msg = instantiate_msg(
        &name,
        &codes,
        voting_module(&codes, VotingKind::Cw4, &member, &name),
    );
    let resp = suite
        .dao_core
        .instantiate(&msg, Some(&member), &[])
        .unwrap_or_else(|err| panic!("{design} instantiate: {err}"));
    let dao = dao_named(daemon, &resp, &name);
    let voting: Addr = daemon
        .query(&QueryMsg::VotingModule {}, &dao)
        .unwrap_or_else(|err| panic!("{design} voting module: {err}"));
    let voting_info = daemon
        .wasm_querier()
        .contract_info(&voting)
        .unwrap_or_else(|err| panic!("{design} voting info: {err}"));
    assert_eq!(voting_info.code_id, codes.voting_cw4, "{design} voting code");
    assert_eq!(voting_info.creator, dao, "{design} voting creator");
    let group: Addr = daemon
        .query(&Cw4Query::GroupContract {}, &voting)
        .unwrap_or_else(|err| panic!("{design} group: {err}"));
    let group_info = daemon
        .wasm_querier()
        .contract_info(&group)
        .unwrap_or_else(|err| panic!("{design} group info: {err}"));
    assert_eq!(group_info.code_id, codes.cw4_group, "{design} group code");
    assert_eq!(group_info.creator, voting, "{design} group creator");
    println!(
        "{design} dao={dao} voting={voting} group={group} gas_used={} gas_wanted={} txhash={}",
        resp.gas_used, resp.gas_wanted, resp.txhash
    );
    GasRow {
        design,
        gas_wanted: resp.gas_wanted,
        gas_used: resp.gas_used,
        txhash: resp.txhash,
    }
}

fn dao_named(daemon: &Daemon, resp: &CosmTxResponse, name: &str) -> Addr {
    resp.events()
        .into_iter()
        .flat_map(|event| event.attributes)
        .filter(|attr| attr.key == "_contract_address" || attr.key == "contract_address")
        .map(|attr| attr.value)
        .find_map(|addr| {
            let addr = Addr::unchecked(addr);
            let config: Result<dao_interface::state::Config, _> =
                daemon.query(&QueryMsg::Config {}, &addr);
            config.ok().filter(|config| config.name == name).map(|_| addr)
        })
        .unwrap_or_else(|| panic!("DAO {name} was not instantiated"))
}

fn stage_reply_dir() -> PathBuf {
    let dir = PathBuf::from("/tmp/dao-gas-wasm/framework-reply");
    std::fs::create_dir_all(&dir).unwrap();
    let src = PathBuf::from("/tmp/dao-gas-wasm");
    for (from, to) in [
        ("dao_dao_core.wasm", "dao_dao_core.wasm"),
        ("dao_proposal_single.wasm", "dao_proposal_single.wasm"),
        ("cw4_group.wasm", "cw4_group.wasm"),
        ("dao_voting_cw4_reply.wasm", "dao_voting_cw4.wasm"),
    ] {
        std::fs::copy(src.join(from), dir.join(to)).unwrap_or_else(|err| panic!("copy {from}: {err}"));
    }
    dir
}

fn assert_sha(path: &Path, expect: &str) {
    let out = Command::new("shasum")
        .args(["-a", "256", &path.display().to_string()])
        .output()
        .expect("shasum");
    assert!(out.status.success(), "shasum {}", path.display());
    let got = String::from_utf8_lossy(&out.stdout);
    assert!(got.starts_with(expect), "{} hashed {got}", path.display());
}

fn image_has_allowlist(repo: &str, version: &str) -> bool {
    let image = format!("{repo}:{version}");
    let out = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--entrypoint",
            "/bin/sh",
            &image,
            "-c",
            "grep -a -q filterWasmReplyEvents /usr/local/bin/terpd",
        ])
        .output()
        .expect("docker grep terpd");
    out.status.success()
}
