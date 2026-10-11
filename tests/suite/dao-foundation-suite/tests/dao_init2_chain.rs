//! Nested subDAOs on a real terpd, using wasm built by the cw-optimizer image.
//!
//! cw-orch Mock still delivers instantiate events to replies. wasmd does not:
//! `filterWasmReplyEvents` drops them. This test uploads the optimizer artifacts
//! and creates one subDAO per voting module that now predicts its child with
//! instantiate2. The child must exist, and its creator must be the voting module.
//!
//! ```text
//! TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 ICT_DAO_INIT2=1 \
//!   cargo test -p dao-foundation-suite --test dao_init2_chain -- --ignored --nocapture
//! ```
//!
//! Artifacts come from `dao-contracts/artifacts/init2/` (override with
//! `DAO_INIT2_ARTIFACTS`). `ICT_MOCK=1` is rejected: the mock runtime does not
//! run wasmd.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cosmwasm_std::Binary;
use ict_rs::chain::{Chain, GenesisStyle};
use ict_rs::testing::{TestChain, TestChainConfig, TestEnv};
use ict_rs::tx::WalletAmount;
use serde_json::{json, Value};

const KINDS: &[&str] = &[
    "cw4",
    "cw20-staked",
    "cw721-staked",
    "cw721-roles",
    "cw20-balance",
    "token-staked",
];

const WASM: &[&str] = &[
    "dao_dao_core.wasm",
    "dao_proposal_single.wasm",
    "dao_voting_cw4.wasm",
    "cw4_group.wasm",
    "dao_voting_cw20_staked.wasm",
    "cw20_base.wasm",
    "cw20_stake.wasm",
    "dao_voting_cw721_staked.wasm",
    "cw721_base.wasm",
    "dao_voting_cw721_roles.wasm",
    "cw721_roles.wasm",
    "dao_voting_cw20_balance.wasm",
    "dao_voting_token_staked.wasm",
    "cw_tokenfactory_issuer.wasm",
];

struct Codes {
    core: u64,
    proposal: u64,
    voting_cw4: u64,
    cw4_group: u64,
    voting_cw20_staked: u64,
    cw20_base: u64,
    cw20_stake: u64,
    voting_cw721_staked: u64,
    cw721_base: u64,
    voting_cw721_roles: u64,
    cw721_roles: u64,
    voting_cw20_balance: u64,
    voting_token_staked: u64,
    tokenfactory_issuer: u64,
}

struct Child {
    code_id: u64,
    address: String,
}

#[tokio::test]
#[ignore = "docker: TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 ICT_DAO_INIT2=1"]
async fn nested_subdaos_instantiate2_on_terp() {
    if std::env::var("ICT_DAO_INIT2").ok().as_deref() != Some("1") {
        panic!(
            "set ICT_DAO_INIT2=1 TERP_IMAGE_REPO=terp-core TERP_IMAGE_VERSION=v630-arm64 \
             before running this ignored test"
        );
    }
    if std::env::var("ICT_MOCK").ok().as_deref() == Some("1") {
        panic!("ICT_MOCK does not exercise wasmd reply filtering");
    }

    let artifacts = artifact_dir();
    for name in WASM {
        let path = artifacts.join(name);
        let len = std::fs::metadata(&path)
            .unwrap_or_else(|err| panic!("missing {}: {err}", path.display()))
            .len();
        assert!(
            len > 0 && len <= 800 * 1024,
            "{name} is {len} bytes; wasmd MaxWasmSize is 800KiB uncompressed"
        );
        println!("artifact {name} {len} bytes");
    }

    let mut cfg = TestEnv::terp_config();
    cfg.chain_id = "dao-init2-1".into();
    cfg.gas_prices = "0uterp".into();
    cfg.gas_adjustment = 2.5;
    cfg.block_time = "500ms".into();
    // v6.3 terpd: `init`, `add-genesis-account`, `collect-gentxs`.
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
        // A fresh issuer cannot pay the default 10_000_000uterp creation fee.
        if let Some(params) = genesis.pointer_mut("/app_state/tokenfactory/params") {
            params["denom_creation_fee"] = json!([]);
        }
        serde_json::to_vec(&genesis)
            .map_err(|err| ict_rs::error::IctError::Config(format!("genesis encode: {err}")))
    }));

    let mut chain = TestChain::setup(
        "dao-init2",
        TestChainConfig {
            chain_config: cfg,
            num_validators: 1,
            num_full_nodes: 0,
            genesis_wallets: vec![],
        },
    )
    .await
    .expect("local terp chain");

    let result = exercise(&chain, &artifacts).await;
    if let Err(ref err) = result {
        eprintln!("instantiate2 chain failed: {err}");
        chain.dump_logs().await;
    }
    let cleanup = chain.cleanup().await;
    result.expect("instantiate2 chain exercise");
    cleanup.expect("chain cleanup");
}

async fn exercise(chain: &TestChain, artifacts: &Path) -> Result<(), String> {
    chain
        .create_key("member")
        .await
        .map_err(|err| err.to_string())?;
    let member = chain
        .key_address("member")
        .await
        .map_err(|err| err.to_string())?;
    println!("member {member}");
    chain
        .send_funds(
            "validator",
            &WalletAmount {
                address: member.clone(),
                denom: "uterp".into(),
                amount: 50_000_000_000,
            },
        )
        .await
        .map_err(|err| err.to_string())?;
    wait_included(chain).await?;

    let node = chain.validators().first().ok_or("no validator node")?;
    for name in WASM {
        let dest = format!("/tmp/{name}");
        node.copy_file_from_host(&artifacts.join(name), &dest)
            .await
            .map_err(|err| format!("copy {name}: {err}"))?;
    }

    let codes = Codes {
        core: store(chain, "/tmp/dao_dao_core.wasm").await?,
        proposal: store(chain, "/tmp/dao_proposal_single.wasm").await?,
        voting_cw4: store(chain, "/tmp/dao_voting_cw4.wasm").await?,
        cw4_group: store(chain, "/tmp/cw4_group.wasm").await?,
        voting_cw20_staked: store(chain, "/tmp/dao_voting_cw20_staked.wasm").await?,
        cw20_base: store(chain, "/tmp/cw20_base.wasm").await?,
        cw20_stake: store(chain, "/tmp/cw20_stake.wasm").await?,
        voting_cw721_staked: store(chain, "/tmp/dao_voting_cw721_staked.wasm").await?,
        cw721_base: store(chain, "/tmp/cw721_base.wasm").await?,
        voting_cw721_roles: store(chain, "/tmp/dao_voting_cw721_roles.wasm").await?,
        cw721_roles: store(chain, "/tmp/cw721_roles.wasm").await?,
        voting_cw20_balance: store(chain, "/tmp/dao_voting_cw20_balance.wasm").await?,
        voting_token_staked: store(chain, "/tmp/dao_voting_token_staked.wasm").await?,
        tokenfactory_issuer: store(chain, "/tmp/cw_tokenfactory_issuer.wasm").await?,
    };
    println!(
        "codes core={} cw4={} cw20={} cw721={} roles={} balance={} token={}",
        codes.core,
        codes.voting_cw4,
        codes.voting_cw20_staked,
        codes.voting_cw721_staked,
        codes.voting_cw721_roles,
        codes.voting_cw20_balance,
        codes.voting_token_staked
    );

    let root_msg = dao_init("foundation", &member, &codes, "cw4");
    let before = contracts_by_code(chain, codes.core).await?;
    broadcast(
        chain,
        &[
            "tx",
            "wasm",
            "instantiate",
            &codes.core.to_string(),
            &root_msg.to_string(),
            "--label",
            "foundation",
            "--admin",
            &member,
            "--from",
            "member",
        ],
    )
    .await?;
    let root = find_dao(chain, codes.core, &before, "foundation").await?;
    println!("foundation {root}");
    assert_voting(chain, &root, "cw4", &codes).await?;

    let mut parent = root;
    for kind in &KINDS[1..] {
        let name = format!("{kind}-subdao");
        let init = dao_init(&name, &member, &codes, kind);
        let msg = json!({
            "execute_admin_msgs": {
                "msgs": [{
                    "wasm": {
                        "instantiate": {
                            "admin": parent,
                            "code_id": codes.core,
                            "msg": b64(&init),
                            "funds": [],
                            "label": name
                        }
                    }
                }]
            }
        });
        let before = contracts_by_code(chain, codes.core).await?;
        broadcast(
            chain,
            &[
                "tx",
                "wasm",
                "execute",
                &parent,
                &msg.to_string(),
                "--from",
                "member",
            ],
        )
        .await
        .map_err(|err| format!("{name}: {err}"))?;
        let child = find_dao(chain, codes.core, &before, &name).await?;
        let info = contract_info(chain, &child).await?;
        assert_eq!(info.creator, parent, "{name} creator is the parent DAO");
        assert_eq!(info.admin, parent, "{name} wasm admin is the parent DAO");
        assert_eq!(info.code_id, codes.core);
        println!("{name} {child} creator={parent}");
        assert_voting(chain, &child, kind, &codes).await?;
        parent = child;
    }
    Ok(())
}

fn artifact_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("DAO_INIT2_ARTIFACTS") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../dao-contracts/artifacts/init2")
}

fn b64(value: &Value) -> String {
    Binary::new(serde_json::to_vec(value).expect("json")).to_base64()
}

fn proposal_module(codes: &Codes, label: &str) -> Value {
    let msg = json!({
        "threshold": {"absolute_count": {"threshold": "1"}},
        "max_voting_period": {"time": 86400},
        "min_voting_period": null,
        "only_members_execute": false,
        "allow_revoting": false,
        "pre_propose_info": {"anyone_may_propose": {}},
        "close_proposal_on_execution_failure": true,
        "veto": null,
        "delegation_module": null
    });
    json!({
        "code_id": codes.proposal,
        "msg": b64(&msg),
        "admin": {"core_module": {}},
        "funds": null,
        "label": label,
        "salt": null
    })
}

fn voting_module(codes: &Codes, kind: &str, member: &str, label: &str) -> Value {
    let (code_id, msg) = match kind {
        "cw4" => (
            codes.voting_cw4,
            json!({
                "group_contract": {
                    "new": {
                        "cw4_group_code_id": codes.cw4_group,
                        "cw4_group_salt": null,
                        "initial_members": [{"addr": member, "weight": 1}]
                    }
                }
            }),
        ),
        "cw20-staked" => (
            codes.voting_cw20_staked,
            json!({
                "token_info": {
                    "new": {
                        "code_id": codes.cw20_base,
                        "salt": null,
                        "label": format!("{label} token"),
                        "name": label,
                        "symbol": "VOTE",
                        "decimals": 6,
                        "initial_balances": [{"address": member, "amount": "1"}],
                        "marketing": null,
                        "staking_code_id": codes.cw20_stake,
                        "staking_salt": null,
                        "unstaking_duration": null,
                        "initial_dao_balance": null
                    }
                },
                "active_threshold": null
            }),
        ),
        "cw721-staked" => {
            let init = json!({
                "name": label,
                "symbol": "VOTE",
                "minter": member,
                "collection_info_extension": null,
                "creator": null,
                "withdraw_address": null
            });
            let mint = json!({
                "mint": {
                    "token_id": "1",
                    "owner": member,
                    "token_uri": null,
                    "extension": {}
                }
            });
            (
                codes.voting_cw721_staked,
                json!({
                    "nft_contract": {
                        "new": {
                            "code_id": codes.cw721_base,
                            "salt": null,
                            "label": format!("{label} nft"),
                            "msg": b64(&init),
                            "initial_nfts": [b64(&mint)]
                        }
                    },
                    "unstaking_duration": null,
                    "active_threshold": null
                }),
            )
        }
        "cw721-roles" => (
            codes.voting_cw721_roles,
            json!({
                "nft_contract": {
                    "new": {
                        "code_id": codes.cw721_roles,
                        "salt": null,
                        "label": format!("{label} roles"),
                        "name": label,
                        "symbol": "ROLE",
                        "initial_nfts": [{
                            "token_id": "1",
                            "owner": member,
                            "token_uri": null,
                            "extension": {"role": null, "weight": 1}
                        }]
                    }
                }
            }),
        ),
        "cw20-balance" => (
            codes.voting_cw20_balance,
            json!({
                "token_info": {
                    "new": {
                        "code_id": codes.cw20_base,
                        "salt": null,
                        "label": format!("{label} balance"),
                        "name": label,
                        "symbol": "BAL",
                        "decimals": 6,
                        "initial_balances": [{"address": member, "amount": "1"}],
                        "marketing": null
                    }
                }
            }),
        ),
        "token-staked" => (
            codes.voting_token_staked,
            json!({
                "token_info": {
                    "new": {
                        "token_issuer_code_id": codes.tokenfactory_issuer,
                        "token_issuer_salt": null,
                        "subdenom": "vote",
                        "metadata": null,
                        "initial_balances": [{"amount": "1", "address": member}],
                        "initial_dao_balance": null
                    }
                },
                "unstaking_duration": null,
                "active_threshold": null
            }),
        ),
        other => panic!("unknown voting kind {other}"),
    };
    json!({
        "code_id": code_id,
        "msg": b64(&msg),
        "admin": {"core_module": {}},
        "funds": null,
        "label": label,
        "salt": null
    })
}

fn dao_init(name: &str, member: &str, codes: &Codes, kind: &str) -> Value {
    json!({
        "admin": member,
        "name": name,
        "description": name,
        "image_url": null,
        "automatically_add_cw20s": false,
        "automatically_add_cw721s": false,
        "voting_module_instantiate_info": voting_module(codes, kind, member, name),
        "proposal_modules_instantiate_info": [proposal_module(codes, &format!("{name}-proposal"))],
        "initial_items": null,
        "initial_actions": null,
        "dao_uri": null
    })
}

async fn assert_voting(
    chain: &TestChain,
    dao: &str,
    kind: &str,
    codes: &Codes,
) -> Result<(), String> {
    let voting = smart_addr(chain, dao, &json!({"voting_module": {}})).await?;
    let voting_info = contract_info(chain, &voting).await?;
    assert_eq!(voting_info.creator, dao, "{kind} voting module creator");
    let child = match kind {
        "cw4" => {
            assert_eq!(voting_info.code_id, codes.voting_cw4);
            Child {
                code_id: codes.cw4_group,
                address: smart_addr(chain, &voting, &json!({"group_contract": {}})).await?,
            }
        }
        "cw20-staked" => {
            assert_eq!(voting_info.code_id, codes.voting_cw20_staked);
            let token = smart_addr(chain, &voting, &json!({"token_contract": {}})).await?;
            let stake = smart_addr(chain, &voting, &json!({"staking_contract": {}})).await?;
            assert_child(chain, &voting, &token, codes.cw20_base, "cw20 token").await?;
            Child {
                code_id: codes.cw20_stake,
                address: stake,
            }
        }
        "cw721-staked" => {
            assert_eq!(voting_info.code_id, codes.voting_cw721_staked);
            let config = smart(chain, &voting, &json!({"config": {}})).await?;
            Child {
                code_id: codes.cw721_base,
                address: config["nft_address"]
                    .as_str()
                    .ok_or_else(|| format!("cw721 config {config}"))?
                    .to_string(),
            }
        }
        "cw721-roles" => {
            assert_eq!(voting_info.code_id, codes.voting_cw721_roles);
            let config = smart(chain, &voting, &json!({"config": {}})).await?;
            Child {
                code_id: codes.cw721_roles,
                address: config["nft_address"]
                    .as_str()
                    .ok_or_else(|| format!("cw721-roles config {config}"))?
                    .to_string(),
            }
        }
        "cw20-balance" => {
            assert_eq!(voting_info.code_id, codes.voting_cw20_balance);
            Child {
                code_id: codes.cw20_base,
                address: smart_addr(chain, &voting, &json!({"token_contract": {}})).await?,
            }
        }
        "token-staked" => {
            assert_eq!(voting_info.code_id, codes.voting_token_staked);
            let denom = smart(chain, &voting, &json!({"denom": {}})).await?;
            let denom = denom["denom"]
                .as_str()
                .ok_or_else(|| format!("denom response {denom}"))?;
            let mut parts = denom.split('/');
            let factory = parts.next().unwrap_or("");
            let issuer = parts.next().unwrap_or("").to_string();
            let sub = parts.next().unwrap_or("");
            assert_eq!(factory, "factory", "tokenfactory denom {denom}");
            assert_eq!(sub, "vote", "tokenfactory denom {denom}");
            assert!(issuer.starts_with("terp1"), "issuer in {denom}");
            Child {
                code_id: codes.tokenfactory_issuer,
                address: issuer,
            }
        }
        other => return Err(format!("unknown kind {other}")),
    };
    assert_child(chain, &voting, &child.address, child.code_id, kind).await
}

async fn assert_child(
    chain: &TestChain,
    voting: &str,
    child: &str,
    code_id: u64,
    kind: &str,
) -> Result<(), String> {
    let info = contract_info(chain, child).await?;
    assert_eq!(info.code_id, code_id, "{kind} child code");
    assert_eq!(
        info.creator, voting,
        "{kind} child creator is the voting module (instantiate2)"
    );
    println!("  {kind} child {child} code={code_id} creator={voting}");
    Ok(())
}

struct Info {
    code_id: u64,
    creator: String,
    admin: String,
}

async fn contract_info(chain: &TestChain, address: &str) -> Result<Info, String> {
    let out = chain
        .chain_exec(&["query", "wasm", "contract", address, "--output", "json"])
        .await
        .map_err(|err| err.to_string())?;
    if out.exit_code != 0 {
        return Err(format!("contract {address}: {}", out.stderr_str().trim()));
    }
    let value = parse_json(&out.stdout_str())?;
    let body = value.get("contract_info").unwrap_or(&value);
    let code_id = body
        .get("code_id")
        .and_then(json_u64)
        .ok_or_else(|| format!("code_id in {value}"))?;
    let creator = body
        .get("creator")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let admin = body
        .get("admin")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(Info {
        code_id,
        creator,
        admin,
    })
}

async fn smart(chain: &TestChain, address: &str, query: &Value) -> Result<Value, String> {
    let out = chain
        .chain_exec(&[
            "query",
            "wasm",
            "contract-state",
            "smart",
            address,
            &query.to_string(),
            "--output",
            "json",
        ])
        .await
        .map_err(|err| err.to_string())?;
    if out.exit_code != 0 {
        return Err(format!(
            "smart {address} {query}: {}",
            out.stderr_str().trim()
        ));
    }
    let value = parse_json(&out.stdout_str())?;
    Ok(unwrap_data(value))
}

async fn smart_addr(chain: &TestChain, address: &str, query: &Value) -> Result<String, String> {
    let value = smart(chain, address, query).await?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("address query {query} on {address} returned {value}"))
}

async fn find_dao(
    chain: &TestChain,
    code_id: u64,
    before: &[String],
    name: &str,
) -> Result<String, String> {
    let after = contracts_by_code(chain, code_id).await?;
    let mut matched = Vec::new();
    for address in after {
        if before.iter().any(|known| known == &address) {
            continue;
        }
        let config = smart(chain, &address, &json!({"config": {}})).await?;
        if config.get("name").and_then(Value::as_str) == Some(name) {
            matched.push(address);
        }
    }
    match matched.len() {
        1 => Ok(matched.remove(0)),
        0 => Err(format!("DAO {name} was not instantiated")),
        _ => Err(format!("multiple DAOs named {name}: {matched:?}")),
    }
}

async fn contracts_by_code(chain: &TestChain, code_id: u64) -> Result<Vec<String>, String> {
    let out = chain
        .chain_exec(&[
            "query",
            "wasm",
            "list-contract-by-code",
            &code_id.to_string(),
            "--output",
            "json",
        ])
        .await
        .map_err(|err| err.to_string())?;
    if out.exit_code != 0 {
        return Err(out.stderr_str());
    }
    let value = parse_json(&out.stdout_str())?;
    Ok(value
        .get("contracts")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

async fn store(chain: &TestChain, path: &str) -> Result<u64, String> {
    let before = code_ids(chain).await?;
    broadcast(chain, &["tx", "wasm", "store", path, "--from", "member"])
        .await
        .map_err(|err| format!("store {path}: {err}"))?;
    let after = code_ids(chain).await?;
    let id = after
        .into_iter()
        .find(|id| !before.contains(id))
        .ok_or_else(|| format!("no new code id after store {path}"))?;
    println!("stored {path} code_id={id}");
    id.parse().map_err(|err| format!("code id {id}: {err}"))
}

async fn code_ids(chain: &TestChain) -> Result<Vec<String>, String> {
    let out = chain
        .chain_exec(&["query", "wasm", "list-code", "--output", "json"])
        .await
        .map_err(|err| err.to_string())?;
    if out.exit_code != 0 {
        return Err(out.stderr_str());
    }
    let value = parse_json(&out.stdout_str()).unwrap_or(Value::Null);
    Ok(value
        .get("code_infos")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("code_id").and_then(json_u64))
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default())
}

async fn broadcast(chain: &TestChain, args: &[&str]) -> Result<Value, String> {
    let opts = chain.default_tx_opts().gas_adjustment(2.5);
    let out = chain
        .chain_exec_tx_with(args, opts)
        .await
        .map_err(|err| err.to_string())?;
    if out.exit_code != 0 {
        return Err(format!(
            "exit {}: {}",
            out.exit_code,
            out.stderr_str().trim()
        ));
    }
    let submitted = parse_json(&out.stdout_str())?;
    let check = submitted.get("code").and_then(json_u64).unwrap_or(0);
    if check != 0 {
        return Err(format!("checktx {submitted}"));
    }
    let hash = submitted
        .get("txhash")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("no txhash in {submitted}"))?
        .to_string();
    poll_tx(chain, &hash).await
}

async fn poll_tx(chain: &TestChain, hash: &str) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last = String::new();
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let out = match chain
            .chain_exec(&["query", "tx", hash, "--output", "json"])
            .await
        {
            Ok(out) => out,
            Err(err) => {
                last = err.to_string();
                continue;
            }
        };
        if out.exit_code != 0 {
            last = out.stderr_str();
            continue;
        }
        let value = match parse_json(&out.stdout_str()) {
            Ok(value) => value,
            Err(err) => {
                last = err;
                continue;
            }
        };
        if value.get("code").is_none() && value.get("txhash").is_none() {
            last = value.to_string();
            continue;
        }
        let code = value.get("code").and_then(json_u64).unwrap_or(0);
        if code != 0 {
            let log = value.get("raw_log").and_then(Value::as_str).unwrap_or("");
            return Err(format!("tx {hash} code {code}: {log}"));
        }
        return Ok(value);
    }
    Err(format!("tx {hash} not included: {last}"))
}

async fn wait_included(chain: &TestChain) -> Result<(), String> {
    let start = chain.height().await.map_err(|err| err.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let height = chain.height().await.map_err(|err| err.to_string())?;
        if height > start {
            return Ok(());
        }
    }
    Err("chain height did not advance after funding".into())
}

fn parse_json(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }
    if let Some(start) = trimmed.find(['{', '[']) {
        if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start..]) {
            return Ok(value);
        }
    }
    Err(format!("not json: {trimmed}"))
}

fn unwrap_data(value: Value) -> Value {
    let Some(data) = value.get("data").cloned() else {
        return value;
    };
    if let Some(text) = data.as_str() {
        if let Ok(inner) = serde_json::from_str::<Value>(text) {
            return inner;
        }
        if let Ok(bytes) = Binary::from_base64(text) {
            if let Ok(inner) = serde_json::from_slice::<Value>(bytes.as_slice()) {
                return inner;
            }
        }
        return data;
    }
    data
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}
