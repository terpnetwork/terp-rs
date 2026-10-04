#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_json_binary, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use hm_loyalty_whitelist::{assert_eligible, validate_root_hex, CODEC_V1};
use hm_nft_oracle::{
    apply_sudo_price, assert_oracle_cannot_modify_supply, assert_payment, resolve_bound,
    HashMerchantSudoMsg, MintPriceBound, OracleMid, PricePolicy, PriceSyncMode,
};

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, IsEligibleResponse, NextTokenIdResponse,
    OracleMidResponse, QueryMsg, WhitelistRootResponse,
};
use crate::state::{Config, RootMeta, CONFIG, LAST_ROOT_META, ORACLE_MID};

pub const CONTRACT_NAME: &str = "crates.io:hm-nft-minter";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if msg.max_tokens == 0 {
        return Err(ContractError::InvalidMaxTokens {});
    }

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    cw_ownable::initialize_owner(deps.storage, deps.api, msg.owner.as_deref())?;

    let owner_addr = cw_ownable::get_ownership(deps.storage)?
        .owner
        .unwrap_or_else(|| info.sender.clone());

    let cw721 = msg
        .cw721_address
        .as_ref()
        .map(|a| deps.api.addr_validate(a))
        .transpose()?;

    let proceeds = match msg.proceeds {
        Some(p) => deps.api.addr_validate(&p)?,
        None => owner_addr,
    };

    let mut whitelist = msg.whitelist;
    if whitelist.codec.is_empty() {
        whitelist.codec = CODEC_V1.to_string();
    }
    if let Some(ref root) = whitelist.root {
        whitelist.root = Some(validate_root_hex(root)?);
    }

    let config = Config {
        cw721_address: cw721,
        max_tokens: msg.max_tokens,
        next_token_id: 0,
        token_uri_prefix: msg.token_uri_prefix,
        whitelist,
        price: msg.price,
        proceeds,
        paused: msg.paused.unwrap_or(false),
    };
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("max_tokens", config.max_tokens.to_string()))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Mint { proof, token_uri } => execute_mint(deps, env, info, proof, token_uri),
        ExecuteMsg::SetWhitelistRoot {
            root,
            codec,
            enabled,
        } => execute_set_whitelist_root(deps, info, root, codec, enabled),
        ExecuteMsg::SetPricePolicy { price } => execute_set_price_policy(deps, info, price),
        ExecuteMsg::AdminSyncPrice {
            mid,
            foreign_height,
            asset_id,
        } => execute_admin_sync_price(deps, env, info, mid, foreign_height, asset_id),
        ExecuteMsg::LinkCollection { cw721_address } => {
            execute_link_collection(deps, info, cw721_address)
        }
        ExecuteMsg::SetPaused { paused } => execute_set_paused(deps, info, paused),
        ExecuteMsg::SetProceeds { proceeds } => execute_set_proceeds(deps, info, proceeds),
        ExecuteMsg::Withdraw { amount } => execute_withdraw(deps, env, info, amount),
        ExecuteMsg::UpdateOwnership(action) => {
            let ownership = cw_ownable::update_ownership(deps, &env.block, &info.sender, action)?;
            Ok(Response::new().add_attributes(ownership.into_attributes()))
        }
    }
}

fn assert_owner(deps: Deps, sender: &cosmwasm_std::Addr) -> Result<(), ContractError> {
    cw_ownable::assert_owner(deps.storage, sender)?;
    Ok(())
}

pub fn execute_mint(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    proof: Option<hm_loyalty_whitelist::LoyaltyProof>,
    token_uri: Option<String>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    if config.paused {
        return Err(ContractError::Paused {});
    }
    if config.next_token_id >= config.max_tokens {
        return Err(ContractError::SoldOut {
            max_tokens: config.max_tokens,
        });
    }
    let cw721 = config
        .cw721_address
        .clone()
        .ok_or(ContractError::CollectionNotLinked {})?;

    // 1) Whitelist (eligibility) — orthogonal to price.
    assert_eligible(
        &config.whitelist,
        info.sender.as_str(),
        proof.as_ref(),
    )?;

    // 2) Price bound from oracle / fallback.
    let mid = ORACLE_MID.may_load(deps.storage)?;
    let bound = resolve_bound(&config.price, mid.as_ref(), env.block.time)?;
    let paid = assert_payment(&bound, &info.funds)?;

    let token_id = config.next_token_id;
    let uri = token_uri.unwrap_or_else(|| {
        match &config.token_uri_prefix {
            Some(p) => format!("{p}{token_id}"),
            None => token_id.to_string(),
        }
    });

    // Advance supply BEFORE emitting mint (fail closed on double-mint races in same block is N/A for CW).
    config.next_token_id += 1;
    // max_tokens is never increased here.
    CONFIG.save(deps.storage, &config)?;

    // Minimal CW721 Mint shape (compatible with cw721 / cw721-svg ExecuteMsg::Mint).
    #[derive(serde::Serialize)]
    struct Cw721Mint<'a> {
        mint: Cw721MintBody<'a>,
    }
    #[derive(serde::Serialize)]
    struct Cw721MintBody<'a> {
        token_id: String,
        owner: &'a str,
        token_uri: Option<String>,
        extension: Option<()>,
    }
    let mint_msg = Cw721Mint {
        mint: Cw721MintBody {
            token_id: token_id.to_string(),
            owner: info.sender.as_str(),
            token_uri: Some(uri),
            extension: None,
        },
    };

    let mut msgs: Vec<CosmosMsg> = vec![WasmMsg::Execute {
        contract_addr: cw721.to_string(),
        msg: to_json_binary(&mint_msg)?,
        funds: vec![],
    }
    .into()];

    // Forward payment to proceeds (if any).
    if !paid.is_zero() {
        msgs.push(
            BankMsg::Send {
                to_address: config.proceeds.to_string(),
                amount: vec![Coin {
                    denom: bound.denom.clone(),
                    amount: paid.into(),
                }],
            }
            .into(),
        );
    }

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "mint")
        .add_attribute("token_id", token_id.to_string())
        .add_attribute("owner", info.sender)
        .add_attribute("paid", paid)
        .add_attribute("denom", bound.denom))
}

fn execute_set_whitelist_root(
    deps: DepsMut,
    info: MessageInfo,
    root: String,
    codec: Option<String>,
    enabled: Option<bool>,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let mut config = CONFIG.load(deps.storage)?;
    let root = validate_root_hex(&root)?;
    config.whitelist.root = Some(root.clone());
    if let Some(c) = codec {
        config.whitelist.codec = c;
    }
    if let Some(e) = enabled {
        config.whitelist.enabled = e;
    }
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "set_whitelist_root")
        .add_attribute("root", root)
        .add_attribute("codec", config.whitelist.codec))
}

fn execute_set_price_policy(
    deps: DepsMut,
    info: MessageInfo,
    price: PricePolicy,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let mut config = CONFIG.load(deps.storage)?;
    config.price = price;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("action", "set_price_policy"))
}

fn execute_admin_sync_price(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mid: Uint128,
    foreign_height: Option<u64>,
    asset_id: Option<String>,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let config = CONFIG.load(deps.storage)?;
    // Mode C is always allowed for owner as demo fallback; prefer Sudo when mode is Sudo
    // but admin sync remains available for ops (documented as Mode C).
    let _ = config.price.mode;
    let stored = OracleMid {
        mid,
        foreign_height: foreign_height.unwrap_or(0),
        observed_at: env.block.time.seconds(),
        source_id: Some("admin_sync".into()),
        asset_id,
    };
    ORACLE_MID.save(deps.storage, &stored)?;
    Ok(Response::new()
        .add_attribute("action", "admin_sync_price")
        .add_attribute("mid", mid)
        .add_attribute("mode", format!("{:?}", PriceSyncMode::AdminSync)))
}

fn execute_link_collection(
    deps: DepsMut,
    info: MessageInfo,
    cw721_address: String,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let mut config = CONFIG.load(deps.storage)?;
    if config.cw721_address.is_some() {
        return Err(ContractError::CollectionAlreadyLinked {});
    }
    config.cw721_address = Some(deps.api.addr_validate(&cw721_address)?);
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "link_collection")
        .add_attribute("cw721", cw721_address))
}

fn execute_set_paused(
    deps: DepsMut,
    info: MessageInfo,
    paused: bool,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let mut config = CONFIG.load(deps.storage)?;
    config.paused = paused;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "set_paused")
        .add_attribute("paused", paused.to_string()))
}

fn execute_set_proceeds(
    deps: DepsMut,
    info: MessageInfo,
    proceeds: String,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let mut config = CONFIG.load(deps.storage)?;
    config.proceeds = deps.api.addr_validate(&proceeds)?;
    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new()
        .add_attribute("action", "set_proceeds")
        .add_attribute("proceeds", proceeds))
}

fn execute_withdraw(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    amount: Option<Uint128>,
) -> Result<Response, ContractError> {
    assert_owner(deps.as_ref(), &info.sender)?;
    let config = CONFIG.load(deps.storage)?;
    let balance = deps
        .querier
        .query_balance(env.contract.address, config.price.denom.clone())?;
    let send_amount: cosmwasm_std::Uint256 = match amount {
        Some(a) => a.into(),
        None => balance.amount,
    };
    if send_amount.is_zero() {
        return Ok(Response::new().add_attribute("action", "withdraw_noop"));
    }
    Ok(Response::new()
        .add_message(BankMsg::Send {
            to_address: config.proceeds.to_string(),
            amount: vec![Coin {
                denom: config.price.denom,
                amount: send_amount,
            }],
        })
        .add_attribute("action", "withdraw")
        .add_attribute("amount", send_amount))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: HashMerchantSudoMsg) -> Result<Response, ContractError> {
    // Hard invariant: never touch max_tokens / supply from oracle path.
    assert_oracle_cannot_modify_supply()?;

    let config = CONFIG.load(deps.storage)?;
    let max_before = config.max_tokens;
    let next_before = config.next_token_id;

    let payload = msg.hash_merchant;
    LAST_ROOT_META.save(
        deps.storage,
        &RootMeta {
            chain_uid: payload.chain_uid.clone(),
            algo: payload.algo.clone(),
            height: payload.height,
            attestation_count: payload.attestation_count,
            block_time: payload.block_time,
        },
    )?;

    let mut attrs = vec![
        ("action".to_string(), "sudo_hash_merchant".to_string()),
        ("chain_uid".to_string(), payload.chain_uid.clone()),
        ("algo".to_string(), payload.algo.clone()),
        ("height".to_string(), payload.height.to_string()),
    ];

    if let Some(mid) = apply_sudo_price(&config.price, &payload, env.block.time.seconds())? {
        ORACLE_MID.save(deps.storage, &mid)?;
        attrs.push(("oracle_mid".to_string(), mid.mid.to_string()));
    }

    // Post-condition: supply config unchanged.
    let config_after = CONFIG.load(deps.storage)?;
    debug_assert_eq!(config_after.max_tokens, max_before);
    debug_assert_eq!(config_after.next_token_id, next_before);
    if config_after.max_tokens != max_before || config_after.next_token_id != next_before {
        return Err(ContractError::Oracle(
            hm_nft_oracle::OracleError::OracleCannotModifySupply,
        ));
    }

    Ok(Response::new().add_attributes(attrs))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::MintPriceBound {} => to_json_binary(&query_mint_price_bound(deps, env)?),
        QueryMsg::WhitelistRoot {} => to_json_binary(&query_whitelist_root(deps)?),
        QueryMsg::IsEligible { address, proof } => {
            to_json_binary(&query_is_eligible(deps, address, proof)?)
        }
        QueryMsg::NextTokenId {} => to_json_binary(&query_next_token_id(deps)?),
        QueryMsg::OracleMid {} => to_json_binary(&query_oracle_mid(deps)?),
        QueryMsg::Ownership {} => to_json_binary(&cw_ownable::get_ownership(deps.storage)?),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    let ownership = cw_ownable::get_ownership(deps.storage)?;
    Ok(ConfigResponse {
        owner: ownership.owner,
        cw721_address: config.cw721_address,
        max_tokens: config.max_tokens,
        next_token_id: config.next_token_id,
        token_uri_prefix: config.token_uri_prefix,
        whitelist: config.whitelist,
        price: config.price,
        proceeds: config.proceeds,
        paused: config.paused,
    })
}

fn query_mint_price_bound(deps: Deps, env: Env) -> StdResult<MintPriceBound> {
    let config = CONFIG.load(deps.storage)?;
    let mid = ORACLE_MID.may_load(deps.storage)?;
    resolve_bound(&config.price, mid.as_ref(), env.block.time)
        .map_err(|e| cosmwasm_std::StdError::msg(e.to_string()))
}

fn query_whitelist_root(deps: Deps) -> StdResult<WhitelistRootResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(WhitelistRootResponse {
        enabled: config.whitelist.enabled,
        codec: config.whitelist.codec,
        root: config.whitelist.root,
    })
}

fn query_is_eligible(
    deps: Deps,
    address: String,
    proof: hm_loyalty_whitelist::LoyaltyProof,
) -> StdResult<IsEligibleResponse> {
    let config = CONFIG.load(deps.storage)?;
    match assert_eligible(&config.whitelist, &address, Some(&proof)) {
        Ok(()) => Ok(IsEligibleResponse {
            eligible: true,
            error: None,
        }),
        Err(e) => Ok(IsEligibleResponse {
            eligible: false,
            error: Some(e.to_string()),
        }),
    }
}

fn query_next_token_id(deps: Deps) -> StdResult<NextTokenIdResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(NextTokenIdResponse {
        next_token_id: config.next_token_id,
        max_tokens: config.max_tokens,
    })
}

fn query_oracle_mid(deps: Deps) -> StdResult<OracleMidResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(OracleMidResponse {
        mid: ORACLE_MID.may_load(deps.storage)?,
        mode: config.price.mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::{coins, from_json, Addr, Timestamp};
    use hm_loyalty_whitelist::{LoyaltyProof, WhitelistConfig, CODEC_V1};
    use hm_nft_oracle::{
        HashMerchantSudoOracleAttestation, HashMerchantSudoPayload, MintPriceBound, PricePolicy,
        PriceSyncMode,
    };

    /// Golden loyalty fixture address (string identity for merkle leaves — not bech32-validated).
    const ALICE: &str = "terp1alice000000000000000000000000000001";
    const ROOT: &str = "d97a6a7ea9c877c628d7d27eefa7ce63ff9c0eb7f83755d77e0778a08782293f";

    fn alice_proof() -> LoyaltyProof {
        LoyaltyProof {
            codec: CODEC_V1.to_string(),
            address: ALICE.to_string(),
            leaf: Some("1ce202d3f07152b1b05ce927ff02369f9336909e359488964ad17501d0fc8c60".into()),
            proof: vec![
                "25bedb835af688d4f03c52a582c6e7588f45286a786e63eb3d18724c284162da".into(),
                "3a87fbb37a3d837219fed2184b905d2ea29b2cc2ebc2b240cf4af5a633a129bc".into(),
            ],
            root: Some(ROOT.into()),
        }
    }

    fn base_price() -> PricePolicy {
        PricePolicy {
            enabled: true,
            mode: PriceSyncMode::Sudo,
            denom: "uterp".into(),
            bps_band: 0,
            max_staleness_seconds: 1_000,
            chain_uid: Some("price-oracle".into()),
            algo: Some("sha256".into()),
            asset_id: None,
            fallback_unit_price: Some(Uint128::new(1_000_000)),
            usd_to_unit_scale: None,
        }
    }

    struct TestAddrs {
        owner: Addr,
        cw721: Addr,
        treasury: Addr,
    }

    fn addrs(api: &cosmwasm_std::testing::MockApi) -> TestAddrs {
        TestAddrs {
            owner: api.addr_make("owner"),
            cw721: api.addr_make("cw721_collection"),
            treasury: api.addr_make("treasury"),
        }
    }

    fn setup(deps: DepsMut, a: &TestAddrs) {
        let info = message_info(&a.owner, &[]);
        instantiate(
            deps,
            mock_env(),
            info,
            InstantiateMsg {
                owner: Some(a.owner.to_string()),
                cw721_address: Some(a.cw721.to_string()),
                max_tokens: 10,
                token_uri_prefix: Some("ipfs://x/".into()),
                whitelist: WhitelistConfig {
                    enabled: true,
                    codec: CODEC_V1.into(),
                    root: Some(ROOT.into()),
                },
                price: base_price(),
                proceeds: Some(a.treasury.to_string()),
                paused: Some(false),
            },
        )
        .unwrap();
    }

    fn sync_mid(deps: DepsMut, owner: &Addr, mid: u128, env: Env) {
        execute(
            deps,
            env,
            message_info(owner, &[]),
            ExecuteMsg::AdminSyncPrice {
                mid: Uint128::new(mid),
                foreign_height: Some(1),
                asset_id: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn mint_happy_path_emits_cw721_and_payment() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        sync_mid(deps.as_mut(), &a.owner, 1_000_000, mock_env());

        // ALICE is a fixture string; mint path only needs string equality for proofs.
        let minter = message_info(&Addr::unchecked(ALICE), &coins(1_000_000, "uterp"));
        let res = execute(
            deps.as_mut(),
            mock_env(),
            minter,
            ExecuteMsg::Mint {
                proof: Some(alice_proof()),
                token_uri: None,
            },
        )
        .unwrap();
        assert_eq!(res.messages.len(), 2); // mint + bank send
        let next: NextTokenIdResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::NextTokenId {}).unwrap()).unwrap();
        assert_eq!(next.next_token_id, 1);
    }

    #[test]
    fn whitelist_reject_wrong_proof() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        sync_mid(deps.as_mut(), &a.owner, 1_000_000, mock_env());

        let mut bad = alice_proof();
        bad.proof[0] = "00".repeat(32);
        let minter = message_info(&Addr::unchecked(ALICE), &coins(1_000_000, "uterp"));
        let err = execute(
            deps.as_mut(),
            mock_env(),
            minter,
            ExecuteMsg::Mint {
                proof: Some(bad),
                token_uri: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Whitelist(_)));
    }

    #[test]
    fn underpayment_rejected() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        sync_mid(deps.as_mut(), &a.owner, 1_000_000, mock_env());
        let minter = message_info(&Addr::unchecked(ALICE), &coins(1, "uterp"));
        let err = execute(
            deps.as_mut(),
            mock_env(),
            minter,
            ExecuteMsg::Mint {
                proof: Some(alice_proof()),
                token_uri: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Oracle(_)));
    }

    /// Price tether: mid rises → previously-sufficient payment falls outside band.
    #[test]
    fn price_rise_rejects_old_payment_amount() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        let env = mock_env();

        // Affordable mid
        sync_mid(deps.as_mut(), &a.owner, 1_000_000, env.clone());
        let pay_lo = 1_000_000u128;
        execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(ALICE), &coins(pay_lo, "uterp")),
            ExecuteMsg::Mint {
                proof: Some(alice_proof()),
                token_uri: None,
            },
        )
        .unwrap();

        // Oracle mid spikes 10x (same asset, not a fixed token price)
        sync_mid(deps.as_mut(), &a.owner, 10_000_000, env.clone());
        let bound: MintPriceBound =
            from_json(query(deps.as_ref(), env.clone(), QueryMsg::MintPriceBound {}).unwrap())
                .unwrap();
        assert_eq!(bound.mid, Some(Uint128::new(10_000_000)));
        assert!(bound.min > Uint128::new(pay_lo));

        // Same payment that worked before is now unaffordable
        let err = execute(
            deps.as_mut(),
            env.clone(),
            message_info(&Addr::unchecked(ALICE), &coins(pay_lo, "uterp")),
            ExecuteMsg::Mint {
                proof: Some(alice_proof()),
                token_uri: Some("ipfs://after-spike".into()),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Oracle(_)));

        // Paying the new mid succeeds (price tether, not fixed amount)
        execute(
            deps.as_mut(),
            env,
            message_info(&Addr::unchecked(ALICE), &coins(10_000_000, "uterp")),
            ExecuteMsg::Mint {
                proof: Some(alice_proof()),
                token_uri: Some("ipfs://post-spike".into()),
            },
        )
        .unwrap();
    }

    #[test]
    fn stale_oracle_rejects_mint() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        let mut env = mock_env();
        env.block.time = Timestamp::from_seconds(10_000);
        sync_mid(deps.as_mut(), &a.owner, 1_000_000, env.clone());

        env.block.time = Timestamp::from_seconds(12_001);
        let minter = message_info(&Addr::unchecked(ALICE), &coins(1_000_000, "uterp"));
        let err = execute(
            deps.as_mut(),
            env,
            minter,
            ExecuteMsg::Mint {
                proof: Some(alice_proof()),
                token_uri: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Oracle(_)));
    }

    #[test]
    fn sudo_updates_mid_never_max_tokens() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        let cfg_before: ConfigResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
        assert_eq!(cfg_before.max_tokens, 10);

        let sudo_msg = HashMerchantSudoMsg {
            hash_merchant: HashMerchantSudoPayload {
                chain_uid: "price-oracle".into(),
                algo: "sha256".into(),
                height: 42,
                root: Binary::from(vec![9u8; 32]),
                attestation_count: 5,
                block_time: 1_700_000_000,
                oracle_sources: vec![HashMerchantSudoOracleAttestation {
                    source_id: "demo".into(),
                    value: Binary::from(br#"{"mid":"2500000"}"#.as_slice()),
                    height: 41,
                }],
            },
        };
        sudo(deps.as_mut(), mock_env(), sudo_msg).unwrap();

        let cfg_after: ConfigResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
        assert_eq!(cfg_after.max_tokens, 10);
        assert_eq!(cfg_after.next_token_id, 0);

        let mid: OracleMidResponse =
            from_json(query(deps.as_ref(), mock_env(), QueryMsg::OracleMid {}).unwrap()).unwrap();
        assert_eq!(mid.mid.unwrap().mid, Uint128::new(2_500_000));
    }

    #[test]
    fn unauthorized_set_root_fails() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        let hacker = deps.api.addr_make("hacker");
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&hacker, &[]),
            ExecuteMsg::SetWhitelistRoot {
                root: ROOT.into(),
                codec: None,
                enabled: Some(true),
            },
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::Ownership(_)));
    }

    #[test]
    fn whitelist_off_still_price_checked() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        let info = message_info(&a.owner, &[]);
        instantiate(
            deps.as_mut(),
            mock_env(),
            info.clone(),
            InstantiateMsg {
                owner: Some(a.owner.to_string()),
                cw721_address: Some(a.cw721.to_string()),
                max_tokens: 3,
                token_uri_prefix: None,
                whitelist: WhitelistConfig {
                    enabled: false,
                    codec: CODEC_V1.into(),
                    root: None,
                },
                price: base_price(),
                proceeds: None,
                paused: None,
            },
        )
        .unwrap();
        sync_mid(deps.as_mut(), &a.owner, 500, mock_env());

        let anyone = deps.api.addr_make("anyone");
        let minter = message_info(&anyone, &coins(1, "uterp"));
        assert!(execute(
            deps.as_mut(),
            mock_env(),
            minter,
            ExecuteMsg::Mint {
                proof: None,
                token_uri: None,
            },
        )
        .is_err());

        let minter = message_info(&anyone, &coins(500, "uterp"));
        execute(
            deps.as_mut(),
            mock_env(),
            minter,
            ExecuteMsg::Mint {
                proof: None,
                token_uri: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn codec_mismatch_on_mint() {
        let mut deps = mock_dependencies();
        let a = addrs(&deps.api);
        setup(deps.as_mut(), &a);
        sync_mid(deps.as_mut(), &a.owner, 1_000_000, mock_env());
        let mut p = alice_proof();
        p.codec = "hm-loyalty/v0".into();
        let minter = message_info(&Addr::unchecked(ALICE), &coins(1_000_000, "uterp"));
        assert!(execute(
            deps.as_mut(),
            mock_env(),
            minter,
            ExecuteMsg::Mint {
                proof: Some(p),
                token_uri: None,
            },
        )
        .is_err());
    }
}
