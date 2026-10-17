//! Loyalty verifier — Merkle root store + claim paths.
//!
//! ## Public path (existing `loyalty_rewards` e2e)
//! `ClaimRewards` — keyed by `info.sender` + root_index.
//!
//! ## Private path (zk-jwt elevation) — **complete**
//! `ClaimRewardsPrivate` — keyed by **nullifier** (from terp-zkjwt) + root_index;
//! optional `action_bind` (`loyalty-claim/v1`); settles via Bank send when funded
//! (`settlement=bank_send`) or records `mint_intent` toward `info.sender`
//! (expected: smart account authenticated by zk-jwt).
//!
//! See `reviews/ZKJWT-LOYALTY-REWARDS-EXTENSION.md`.

use cosmwasm_std::{
    entry_point, to_json_binary, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Response, StdError, StdResult, Uint128, Uint256,
};
use cw_storage_plus::{Item, Map};
use sha2::{Digest, Sha256};

pub mod msg;
use msg::*;

#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
mod interface;
#[cfg(all(feature = "interface", not(target_arch = "wasm32")))]
pub use interface::LoyaltyVerifier;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[cosmwasm_schema::cw_serde]
pub struct RootEntry {
    pub chain_uid: String,
    pub algo: String,
    pub root: String,
    pub height: u64,
}

#[cosmwasm_schema::cw_serde]
pub struct ClaimRecord {
    pub leaf_hash: String,
    pub claimer: String,
}

#[cosmwasm_schema::cw_serde]
pub struct PrivateClaimRecord {
    pub leaf_hash: String,
    pub claimer: String,
    pub amount: Uint128,
    pub nullifier: String,
}

#[cosmwasm_schema::cw_serde]
pub struct MintConfig {
    pub reward_denom: Option<String>,
    pub mint_enabled: bool,
    pub admin: String,
}

const ROOTS: Item<Vec<RootEntry>> = Item::new("roots");
/// Public path: (claimer_address, root_index)
const CLAIMS: Map<(&str, u32), ClaimRecord> = Map::new("claims");
/// Private path: (nullifier_hex, root_index)
const NULLIFIER_CLAIMS: Map<(&str, u32), PrivateClaimRecord> = Map::new("nf_claims");
const MINT_CFG: Item<MintConfig> = Item::new("mint_cfg");

// ---------------------------------------------------------------------------
// Instantiate
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    ROOTS.save(deps.storage, &vec![])?;
    MINT_CFG.save(
        deps.storage,
        &MintConfig {
            reward_denom: msg.reward_denom,
            mint_enabled: msg.mint_enabled,
            admin: info.sender.to_string(),
        },
    )?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("path", "loyalty-verifier"))
}

// ---------------------------------------------------------------------------
// Sudo — hashmerchant module callback
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> StdResult<Response> {
    match msg {
        SudoMsg::HashMerchant {
            chain_uid,
            algo,
            root,
            height,
            ..
        } => {
            let root_hex = hex::encode(&base64_decode(&root)?);
            let mut roots = ROOTS.load(deps.storage)?;
            let index = roots.len() as u32;

            roots.push(RootEntry {
                chain_uid: chain_uid.clone(),
                algo: algo.clone(),
                root: root_hex.clone(),
                height,
            });
            ROOTS.save(deps.storage, &roots)?;

            Ok(Response::new()
                .add_attribute("action", "store_root")
                .add_attribute("chain_uid", chain_uid)
                .add_attribute("algo", algo)
                .add_attribute("root", root_hex)
                .add_attribute("height", height.to_string())
                .add_attribute("root_index", index.to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, StdError> {
    match msg {
        ExecuteMsg::ClaimRewards {
            leaf_hash,
            proof,
            root_index,
        } => execute_claim_rewards(deps, info, leaf_hash, proof, root_index),
        ExecuteMsg::ClaimRewardsPrivate {
            leaf_hash,
            proof,
            root_index,
            nullifier,
            amount,
            action_bind,
        } => execute_claim_private(
            deps,
            env,
            info,
            leaf_hash,
            proof,
            root_index,
            nullifier,
            amount,
            action_bind,
        ),
        ExecuteMsg::UpdateMintConfig {
            reward_denom,
            mint_enabled,
        } => {
            let mut cfg = MINT_CFG.load(deps.storage)?;
            if info.sender.as_str() != cfg.admin {
                return Err(StdError::msg("only admin"));
            }
            cfg.reward_denom = reward_denom;
            cfg.mint_enabled = mint_enabled;
            MINT_CFG.save(deps.storage, &cfg)?;
            Ok(Response::new().add_attribute("action", "update_mint_config"))
        }
    }
}

fn execute_claim_rewards(
    deps: DepsMut,
    info: MessageInfo,
    leaf_hash: String,
    proof: Vec<ProofStep>,
    root_index: u32,
) -> StdResult<Response> {
    let sender = info.sender.to_string();

    if CLAIMS.has(deps.storage, (&sender, root_index)) {
        return Err(StdError::msg("already claimed for this root"));
    }

    let roots = ROOTS.load(deps.storage)?;
    let entry = roots
        .get(root_index as usize)
        .ok_or_else(|| StdError::msg("root_index out of bounds"))?;

    if !verify_proof(&leaf_hash, &proof, &entry.root) {
        return Err(StdError::msg("Merkle proof verification failed"));
    }

    CLAIMS.save(
        deps.storage,
        (&sender, root_index),
        &ClaimRecord {
            leaf_hash: leaf_hash.clone(),
            claimer: sender.clone(),
        },
    )?;

    Ok(Response::new()
        .add_attribute("action", "claim_rewards")
        .add_attribute("mode", "public")
        .add_attribute("claimer", sender)
        .add_attribute("leaf_hash", leaf_hash)
        .add_attribute("root_index", root_index.to_string())
        .add_attribute("verified", "true"))
}

/// Domain-separated action bind for private claims (client + contract agree).
pub fn compute_action_bind(
    root_index: u32,
    amount: Uint128,
    leaf_hash: &str,
    dest: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(b"loyalty-claim/v1");
    h.update(root_index.to_be_bytes());
    h.update(amount.to_string().as_bytes());
    h.update([0u8]);
    h.update(leaf_hash.as_bytes());
    h.update([0u8]);
    h.update(dest.as_bytes());
    hex::encode(h.finalize())
}

fn execute_claim_private(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    leaf_hash: String,
    proof: Vec<ProofStep>,
    root_index: u32,
    nullifier: String,
    amount: Uint128,
    action_bind: Option<String>,
) -> StdResult<Response> {
    let sender = info.sender.to_string();
    let nf = nullifier.trim().to_ascii_lowercase();
    if nf.is_empty() || nf.len() < 16 {
        return Err(StdError::msg(
            "nullifier required (zk-jwt codec hex, min 16 chars)",
        ));
    }
    if amount.is_zero() {
        return Err(StdError::msg("amount must be > 0"));
    }

    if let Some(ref bind) = action_bind {
        let expected = compute_action_bind(root_index, amount, &leaf_hash, sender.as_str());
        if bind.trim().to_ascii_lowercase() != expected {
            return Err(StdError::msg(
                "action_bind mismatch (loyalty-claim/v1 bind failed)",
            ));
        }
    }

    if NULLIFIER_CLAIMS.has(deps.storage, (&nf, root_index)) {
        return Err(StdError::msg(
            "nullifier already claimed for this root (private double-claim)",
        ));
    }

    let roots = ROOTS.load(deps.storage)?;
    let entry = roots
        .get(root_index as usize)
        .ok_or_else(|| StdError::msg("root_index out of bounds"))?;

    if !verify_proof(&leaf_hash, &proof, &entry.root) {
        return Err(StdError::msg("Merkle proof verification failed"));
    }

    NULLIFIER_CLAIMS.save(
        deps.storage,
        (&nf, root_index),
        &PrivateClaimRecord {
            leaf_hash: leaf_hash.clone(),
            claimer: sender.clone(),
            amount,
            nullifier: nf.clone(),
        },
    )?;

    let mut res = Response::new()
        .add_attribute("action", "claim_rewards_private")
        .add_attribute("mode", "zkjwt")
        .add_attribute("claimer", sender.clone())
        .add_attribute("nullifier", nf)
        .add_attribute("leaf_hash", leaf_hash.clone())
        .add_attribute("root_index", root_index.to_string())
        .add_attribute("amount", amount.to_string())
        .add_attribute("verified", "true")
        .add_attribute("schema", "v1");

    if action_bind.is_some() {
        res = res.add_attribute("action_bind", "ok");
    }

    let cfg = MINT_CFG.load(deps.storage)?;
    if cfg.mint_enabled {
        if let Some(denom) = cfg.reward_denom {
            let coin = Coin {
                denom: denom.clone(),
                amount: amount.into(),
            };
            let balance = deps
                .querier
                .query_balance(env.contract.address.clone(), denom.clone())?;
            if balance.amount >= Uint256::from(amount) {
                res = res.add_message(CosmosMsg::Bank(BankMsg::Send {
                    to_address: sender,
                    amount: vec![coin],
                }));
                res = res.add_attribute("settlement", "bank_send");
                res = res.add_attribute("mint_denom", denom);
            } else {
                res = res
                    .add_attribute("settlement", "mint_intent")
                    .add_attribute("mint_denom", denom)
                    .add_attribute(
                        "mint_note",
                        "fund contract treasury to enable bank_send settlement",
                    );
            }
        } else {
            res = res.add_attribute("settlement", "none_no_denom");
        }
    } else {
        res = res.add_attribute("settlement", "disabled");
    }

    Ok(res)
}

// ---------------------------------------------------------------------------
// Merkle proof verification
// ---------------------------------------------------------------------------

fn verify_proof(leaf_hex: &str, proof: &[ProofStep], expected_root_hex: &str) -> bool {
    let Ok(mut current) = hex::decode(leaf_hex) else {
        return false;
    };

    for step in proof {
        let Ok(sibling) = hex::decode(&step.sibling) else {
            return false;
        };
        let mut hasher = Sha256::new();
        if step.is_right {
            hasher.update(&current);
            hasher.update(&sibling);
        } else {
            hasher.update(&sibling);
            hasher.update(&current);
        }
        current = hasher.finalize().to_vec();
    }

    hex::encode(&current) == expected_root_hex
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetRoot { chain_uid, algo } => {
            let roots = ROOTS.load(deps.storage)?;
            let entry = roots
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| r.chain_uid == chain_uid && r.algo == algo);
            match entry {
                Some((idx, r)) => to_json_binary(&RootResponse {
                    chain_uid: r.chain_uid.clone(),
                    algo: r.algo.clone(),
                    root: r.root.clone(),
                    height: r.height,
                    index: idx as u32,
                }),
                None => Err(StdError::msg("root not found")),
            }
        }
        QueryMsg::GetRootCount {} => {
            let roots = ROOTS.load(deps.storage)?;
            to_json_binary(&RootCountResponse {
                count: roots.len() as u32,
            })
        }
        QueryMsg::GetRootByIndex { index } => {
            let roots = ROOTS.load(deps.storage)?;
            let entry = roots
                .get(index as usize)
                .ok_or_else(|| StdError::msg("root not found"))?;
            to_json_binary(&RootResponse {
                chain_uid: entry.chain_uid.clone(),
                algo: entry.algo.clone(),
                root: entry.root.clone(),
                height: entry.height,
                index,
            })
        }
        QueryMsg::GetClaim {
            address,
            root_index,
        } => {
            let claim = CLAIMS.may_load(deps.storage, (&address, root_index))?;
            match claim {
                Some(c) => to_json_binary(&ClaimResponse {
                    claimed: true,
                    leaf_hash: c.leaf_hash,
                    root_index,
                    claimer: c.claimer,
                }),
                None => to_json_binary(&ClaimResponse {
                    claimed: false,
                    leaf_hash: String::new(),
                    root_index,
                    claimer: address,
                }),
            }
        }
        QueryMsg::GetNullifierClaim {
            nullifier,
            root_index,
        } => {
            let nf = nullifier.trim().to_ascii_lowercase();
            let claim = NULLIFIER_CLAIMS.may_load(deps.storage, (&nf, root_index))?;
            match claim {
                Some(c) => to_json_binary(&NullifierClaimResponse {
                    claimed: true,
                    nullifier: c.nullifier,
                    root_index,
                    claimer: c.claimer,
                    amount: c.amount.to_string(),
                }),
                None => to_json_binary(&NullifierClaimResponse {
                    claimed: false,
                    nullifier: nf,
                    root_index,
                    claimer: String::new(),
                    amount: "0".into(),
                }),
            }
        }
        QueryMsg::GetMintConfig {} => {
            let cfg = MINT_CFG.load(deps.storage)?;
            to_json_binary(&MintConfigResponse {
                reward_denom: cfg.reward_denom,
                mint_enabled: cfg.mint_enabled,
                admin: cfg.admin,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base64_decode(input: &str) -> StdResult<Vec<u8>> {
    Binary::from_base64(input).map(|b| b.to_vec())
}

// ---------------------------------------------------------------------------
// Tests — dense sanity + adversarial profiles
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cosmwasm_std::Addr;

    type TestDeps = cosmwasm_std::OwnedDeps<
        cosmwasm_std::testing::MockStorage,
        cosmwasm_std::testing::MockApi,
        cosmwasm_std::testing::MockQuerier,
    >;

    fn leaf_and_root() -> (String, String, Vec<ProofStep>) {
        // Single-leaf tree: root = leaf
        let data = b"customers|id=jane|points=875";
        let leaf = Sha256::digest(data).to_vec();
        let leaf_hex = hex::encode(&leaf);
        let root_hex = leaf_hex.clone();
        (leaf_hex, root_hex, vec![])
    }

    fn multi_leaf_tree() -> (String, String, Vec<ProofStep>, String) {
        // Two leaves: alice + jane. Prove alice.
        let alice = Sha256::digest(b"customers|id=alice|points=620").to_vec();
        let jane = Sha256::digest(b"customers|id=jane|points=875").to_vec();
        let mut h = Sha256::new();
        h.update(&alice);
        h.update(&jane);
        let root = h.finalize().to_vec();
        let proof = vec![ProofStep {
            sibling: hex::encode(&jane),
            is_right: true,
        }];
        (
            hex::encode(&alice),
            hex::encode(&root),
            proof,
            hex::encode(&jane),
        )
    }

    fn setup_root(deps: &mut TestDeps, root_hex: String) {
        ROOTS
            .save(
                deps.as_mut().storage,
                &vec![RootEntry {
                    chain_uid: "loyalty-db".into(),
                    algo: "sha256".into(),
                    root: root_hex,
                    height: 1,
                }],
            )
            .unwrap();
    }

    fn setup_two_roots(deps: &mut TestDeps, root0: String, root1: String) {
        ROOTS
            .save(
                deps.as_mut().storage,
                &vec![
                    RootEntry {
                        chain_uid: "loyalty-db".into(),
                        algo: "sha256".into(),
                        root: root0,
                        height: 1,
                    },
                    RootEntry {
                        chain_uid: "loyalty-db".into(),
                        algo: "sha256".into(),
                        root: root1,
                        height: 2,
                    },
                ],
            )
            .unwrap();
    }

    fn inst(deps: DepsMut, mint: bool) {
        instantiate(
            deps,
            mock_env(),
            message_info(&Addr::unchecked("admin"), &[]),
            InstantiateMsg {
                reward_denom: if mint {
                    Some("uloyalty".into())
                } else {
                    None
                },
                mint_enabled: mint,
            },
        )
        .unwrap();
    }

    fn nf(tag: &str) -> String {
        let mut h = Sha256::new();
        h.update(b"test-nf/");
        h.update(tag.as_bytes());
        hex::encode(h.finalize())
    }

    // ── SANITY ──────────────────────────────────────────────────────────

    #[test]
    fn sanity_public_claim_and_query() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let user = Addr::unchecked("jane-eoa");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            ExecuteMsg::ClaimRewards {
                leaf_hash: leaf_hex.clone(),
                proof,
                root_index: 0,
            },
        )
        .unwrap();
        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::GetClaim {
                address: user.to_string(),
                root_index: 0,
            },
        )
        .unwrap();
        let r: ClaimResponse = cosmwasm_std::from_json(q).unwrap();
        assert!(r.claimed);
        assert_eq!(r.leaf_hash, leaf_hex);
        assert_eq!(r.claimer, user.as_str());
    }

    #[test]
    fn sanity_private_claim_nullifier_and_replay() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), true);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);

        let sa = Addr::unchecked("smart-account-zkjwt");
        let n = nf("alice-session");
        let amount = Uint128::new(875);
        let bind = compute_action_bind(0, amount, &leaf_hex, sa.as_str());
        let msg = ExecuteMsg::ClaimRewardsPrivate {
            leaf_hash: leaf_hex.clone(),
            proof: proof.clone(),
            root_index: 0,
            nullifier: n.clone(),
            amount,
            action_bind: Some(bind),
        };
        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa, &[]),
            msg.clone(),
        )
        .unwrap();
        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "mode" && a.value == "zkjwt"));
        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "action_bind" && a.value == "ok"));

        let err = execute(deps.as_mut(), mock_env(), message_info(&sa, &[]), msg).unwrap_err();
        assert!(err.to_string().contains("nullifier already claimed"));
    }

    #[test]
    fn sanity_e2e_private_claim_with_bank_settlement() {
        let mut deps = mock_dependencies();
        let sa = Addr::unchecked("smart-account-zkjwt");
        let denom = "uloyalty";
        inst(deps.as_mut(), true);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);

        let amount = Uint128::new(875);
        let env = mock_env();
        deps.querier.bank.update_balance(
            env.contract.address.clone(),
            vec![Coin::new(875u128, denom)],
        );

        let n = nf("bank-e2e");
        let bind = compute_action_bind(0, amount, &leaf_hex, sa.as_str());
        let res = execute(
            deps.as_mut(),
            env,
            message_info(&sa, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: n.clone(),
                amount,
                action_bind: Some(bind),
            },
        )
        .unwrap();

        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "settlement" && a.value == "bank_send"));
        assert_eq!(res.messages.len(), 1);

        let q = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::GetNullifierClaim {
                nullifier: n,
                root_index: 0,
            },
        )
        .unwrap();
        let parsed: NullifierClaimResponse = cosmwasm_std::from_json(q).unwrap();
        assert!(parsed.claimed);
        assert_eq!(parsed.amount, "875");
        assert_eq!(parsed.claimer, sa.as_str());
    }

    #[test]
    fn sanity_private_without_bind_still_works() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let sa = Addr::unchecked("sa");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: nf("no-bind"),
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn sanity_public_and_private_independent_maps() {
        // Same leaf: public claim by EOA does not spend nullifier map (and vice versa).
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let eoa = Addr::unchecked("eoa");
        let sa = Addr::unchecked("sa");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&eoa, &[]),
            ExecuteMsg::ClaimRewards {
                leaf_hash: leaf_hex.clone(),
                proof: proof.clone(),
                root_index: 0,
            },
        )
        .unwrap();
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: nf("parallel-map"),
                amount: Uint128::new(10),
                action_bind: None,
            },
        )
        .unwrap();
    }

    // ── INSANITY / ADVERSARIAL ──────────────────────────────────────────

    #[test]
    fn insanity_public_double_claim_same_claimer_root() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let user = Addr::unchecked("jane-eoa");
        let msg = ExecuteMsg::ClaimRewards {
            leaf_hash: leaf_hex,
            proof,
            root_index: 0,
        };
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            msg.clone(),
        )
        .unwrap();
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&user, &[]),
            msg,
        )
        .unwrap_err();
        assert!(err.to_string().contains("already claimed"));
    }

    #[test]
    fn insanity_public_bad_merkle_proof_rejected() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, _) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let bad_proof = vec![ProofStep {
            sibling: hex::encode([0u8; 32]),
            is_right: true,
        }];
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("attacker"), &[]),
            ExecuteMsg::ClaimRewards {
                leaf_hash: leaf_hex,
                proof: bad_proof,
                root_index: 0,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("Merkle proof verification failed"));
    }

    #[test]
    fn insanity_public_wrong_leaf_not_in_tree() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (_, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let fake = hex::encode(Sha256::digest(b"customers|id=not-in-db|points=999999"));
        // Single-leaf tree: only correct leaf equals root; fake leaf fails
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("attacker"), &[]),
            ExecuteMsg::ClaimRewards {
                leaf_hash: fake,
                proof,
                root_index: 0,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("Merkle proof verification failed"));
    }

    #[test]
    fn insanity_private_action_bind_wrong_amount() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let sa = Addr::unchecked("smart-account-zkjwt");
        let bind = compute_action_bind(0, Uint128::new(100), &leaf_hex, sa.as_str());
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: nf("wrong-amt"),
                amount: Uint128::new(875),
                action_bind: Some(bind),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("action_bind mismatch"));
    }

    #[test]
    fn insanity_private_action_bind_wrong_dest_hijack() {
        // Attacker presents bind for victim SA but executes as self.
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let victim = "smart-account-victim";
        let attacker = Addr::unchecked("attacker-sa");
        let amount = Uint128::new(620);
        let bind = compute_action_bind(0, amount, &leaf_hex, victim);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&attacker, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: nf("hijack-dest"),
                amount,
                action_bind: Some(bind),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("action_bind mismatch"));
    }

    #[test]
    fn insanity_private_action_bind_wrong_leaf() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (alice_leaf, root_hex, proof, jane_leaf) = multi_leaf_tree();
        setup_root(&mut deps, root_hex);
        let sa = Addr::unchecked("sa");
        let amount = Uint128::new(620);
        // Bind commits to jane leaf but proof is for alice
        let bind = compute_action_bind(0, amount, &jane_leaf, sa.as_str());
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: alice_leaf,
                proof,
                root_index: 0,
                nullifier: nf("wrong-leaf-bind"),
                amount,
                action_bind: Some(bind),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("action_bind mismatch"));
    }

    #[test]
    fn insanity_private_empty_nullifier_rejected() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("sa"), &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: "ab".into(),
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("nullifier required"));
    }

    #[test]
    fn insanity_private_zero_amount_rejected() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("sa"), &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: nf("zero"),
                amount: Uint128::zero(),
                action_bind: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("amount must be > 0"));
    }

    #[test]
    fn insanity_private_bad_merkle_rejected() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, _) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("sa"), &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof: vec![ProofStep {
                    sibling: hex::encode([1u8; 32]),
                    is_right: false,
                }],
                root_index: 0,
                nullifier: nf("bad-merkle"),
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("Merkle proof verification failed"));
    }

    #[test]
    fn insanity_private_proof_for_other_root_index() {
        // Valid proof for root0 submitted against root1 → fail
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root0, proof) = leaf_and_root();
        let other_root = hex::encode(Sha256::digest(b"different-epoch-root"));
        setup_two_roots(&mut deps, root0, other_root);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("sa"), &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 1,
                nullifier: nf("wrong-root"),
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("Merkle proof verification failed"));
    }

    #[test]
    fn insanity_private_root_index_oob() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("sa"), &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 99,
                nullifier: nf("oob"),
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("root_index out of bounds"));
    }

    #[test]
    fn insanity_nullifier_replay_across_accounts() {
        // Same nullifier cannot be used by a different smart account (double-spend).
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let n = nf("shared-oidc-session");
        let sa1 = Addr::unchecked("sa-1");
        let sa2 = Addr::unchecked("sa-2");
        execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa1, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex.clone(),
                proof: proof.clone(),
                root_index: 0,
                nullifier: n.clone(),
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap();
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa2, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: n,
                amount: Uint128::new(1),
                action_bind: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("nullifier already claimed"));
    }

    #[test]
    fn insanity_mint_intent_when_treasury_empty() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), true);
        let (leaf_hex, root_hex, proof) = leaf_and_root();
        setup_root(&mut deps, root_hex);
        let sa = Addr::unchecked("sa");
        let res = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&sa, &[]),
            ExecuteMsg::ClaimRewardsPrivate {
                leaf_hash: leaf_hex,
                proof,
                root_index: 0,
                nullifier: nf("unfunded"),
                amount: Uint128::new(100),
                action_bind: None,
            },
        )
        .unwrap();
        assert!(res
            .attributes
            .iter()
            .any(|a| a.key == "settlement" && a.value == "mint_intent"));
        assert!(res.messages.is_empty());
    }

    #[test]
    fn insanity_non_admin_cannot_update_mint_config() {
        let mut deps = mock_dependencies();
        inst(deps.as_mut(), false);
        let err = execute(
            deps.as_mut(),
            mock_env(),
            message_info(&Addr::unchecked("intruder"), &[]),
            ExecuteMsg::UpdateMintConfig {
                reward_denom: Some("uhack".into()),
                mint_enabled: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("only admin"));
    }

    // ── OFFLINE MICRO-BENCHMARKS (wall-clock; on-chain gas is Docker e2e) ──
    // Run: cargo test bench_ -- --nocapture

    #[test]
    fn bench_merkle_verify_and_action_bind() {
        use std::time::Instant;

        let (alice_leaf, root_hex, proof, _) = multi_leaf_tree();
        let iters = 5_000u32;

        let t0 = Instant::now();
        for _ in 0..iters {
            assert!(verify_proof(&alice_leaf, &proof, &root_hex));
        }
        let merkle_ns = t0.elapsed().as_nanos() / iters as u128;

        let t1 = Instant::now();
        for i in 0..iters {
            let _ = compute_action_bind(
                0,
                Uint128::new(620),
                &alice_leaf,
                &format!("terp1dest{i}"),
            );
        }
        let bind_ns = t1.elapsed().as_nanos() / iters as u128;

        println!("\n=== offline micro-bench (n={iters}) ===");
        println!("  merkle verify:     {merkle_ns:>8} ns/op");
        println!("  action_bind hash:  {bind_ns:>8} ns/op");
        println!("  (on-chain gas → cargo run --example loyalty_rewards --features \"docker hashmerchant\")");
    }

    #[test]
    fn bench_claim_paths_execute_throughput() {
        use std::time::Instant;

        // Public claims: N different claimers, same leaf (single-leaf tree)
        let iters = 200u32;
        let (leaf_hex, root_hex, proof) = leaf_and_root();

        let t_pub = Instant::now();
        for i in 0..iters {
            let mut deps = mock_dependencies();
            inst(deps.as_mut(), false);
            setup_root(&mut deps, root_hex.clone());
            execute(
                deps.as_mut(),
                mock_env(),
                message_info(&Addr::unchecked(format!("eoa-{i}")), &[]),
                ExecuteMsg::ClaimRewards {
                    leaf_hash: leaf_hex.clone(),
                    proof: proof.clone(),
                    root_index: 0,
                },
            )
            .unwrap();
        }
        let pub_us = t_pub.elapsed().as_micros() / iters as u128;

        let t_priv = Instant::now();
        for i in 0..iters {
            let mut deps = mock_dependencies();
            inst(deps.as_mut(), true);
            setup_root(&mut deps, root_hex.clone());
            let sa = Addr::unchecked(format!("sa-{i}"));
            let amount = Uint128::new(875);
            let bind = compute_action_bind(0, amount, &leaf_hex, sa.as_str());
            execute(
                deps.as_mut(),
                mock_env(),
                message_info(&sa, &[]),
                ExecuteMsg::ClaimRewardsPrivate {
                    leaf_hash: leaf_hex.clone(),
                    proof: proof.clone(),
                    root_index: 0,
                    nullifier: nf(&format!("bench-{i}")),
                    amount,
                    action_bind: Some(bind),
                },
            )
            .unwrap();
        }
        let priv_us = t_priv.elapsed().as_micros() / iters as u128;

        println!("\n=== claim execute throughput (n={iters}, mock_deps) ===");
        println!("  ClaimRewards (public):         {pub_us:>6} µs/op");
        println!("  ClaimRewardsPrivate (zk-jwt):  {priv_us:>6} µs/op");
        if pub_us > 0 {
            let ratio = priv_us as f64 / pub_us as f64;
            println!("  private/public wall ratio:     {ratio:.2}x");
        }
        println!("  note: wall-clock ≠ wasmd gas; use Docker gas table for chain cost");
    }
}
