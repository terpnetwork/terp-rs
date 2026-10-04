/// Per-address allocation used in whitelist tests — must match the tree leaf `addr || TEST_WL_ALLOCATION`.
/// Set to 3 so tests that mint multiple tokens stay within the limit.
pub const TEST_WL_ALLOCATION: u32 = 3;

#[cfg(test)]
mod tests {
    use crate::{
        contract::{instantiate, query_has_member},
        msg::InstantiateMsg,
        state::{GENESIS_MINT_START_TIME, NATIVE_FEE_DENOM},
        tests::{
            hasher::SortingBlake3Hasher, test_helpers::hash_and_build_tree,
            unit_tests::TEST_WL_ALLOCATION,
        },
    };
    use rs_merkle::MerkleTree;
    use std::vec;

    use cosmwasm_std::{
        coin,
        testing::{message_info, mock_dependencies, mock_env},
        Addr, BlockInfo, DepsMut, Env, Timestamp,
    };

    const ADMIN: &str = "cosmwasm1ye63jpm474yfrq02nyplrspyw75y82tpd2shys";
    const CREATION_AMOUNT: u128 = 1_000_000_000;

    const GENESIS_START_TIME: Timestamp = Timestamp::from_nanos(GENESIS_MINT_START_TIME);
    // const END_TIME: Timestamp = Timestamp::from_nanos(GENESIS_MINT_START_TIME + 1000);

    // Inline test whitelist — four cosmwasm-prefix addresses.
    // ADDR_0 matches ADMIN so the admin can prove membership at leaf 0.
    const ADDR_0: &str = ADMIN;
    const ADDR_1: &str = "cosmwasm130dxx3nr2ste4fwsum57k3en60wqd76m9spvxzz";
    const ADDR_2: &str = "cosmwasm1x97rgyyudwr29xkauhxuvkgyhsrdxx3tz6q3cs";
    const ADDR_3: &str = "cosmwasm16epdu6c7h8apxrnuu06yzfxflrede0mt6008rw";

    const TEST_WHITELIST: &[&str] = &[ADDR_0, ADDR_1, ADDR_2, ADDR_3];

    // Invalid root format constants — used to test instantiate validation.
    const NON_HEX_MERKLE_ROOT: &str =
        "5zb281bca33c9819e0daa0708d20ff8a25e65de7d1f6659dbdeb1d2050652b80";
    const NON_32BYTES_MERKLE_ROOT: &str =
        "5ab281bca33c9819e0daa0708d20ff8a25e65de7d1f6659dbdeb1d2050652b80ab";

    /// Build a fresh merkle tree from the inline test whitelist.
    /// Root hex is derived at runtime so it always matches the current address prefix.
    fn build_test_tree() -> MerkleTree<SortingBlake3Hasher> {
        let addrs: Vec<String> = TEST_WHITELIST
            .iter()
            .map(|s| format!("{}{}", s.to_string(), super::TEST_WL_ALLOCATION.to_string()))
            .collect();
        hash_and_build_tree(&addrs)
    }

    /// Mock env with block time before GENESIS_START_TIME so instantiate succeeds.
    fn early_mock_env() -> Env {
        Env {
            block: BlockInfo {
                height: 1,
                time: Timestamp::from_nanos(GENESIS_MINT_START_TIME - 1000),
                chain_id: "stargaze-1".to_string(),
            },
            ..mock_env()
        }
    }

    fn custom_mock_env() -> Env {
        Env {
            block: BlockInfo {
                height: 55_555,
                time: GENESIS_START_TIME.plus_nanos(100),
                chain_id: "stargaze-1".to_string(),
            },
            ..mock_env()
        }
    }

    /// Instantiate the contract with a freshly-computed merkle root.
    /// Returns the admin address and the tree so callers can derive proofs.
    fn setup_contract(
        deps: DepsMut,
        merkle_root: Option<String>,
    ) -> (Addr, MerkleTree<SortingBlake3Hasher>) {
        let tree = build_test_tree();
        let root = merkle_root.unwrap_or_else(|| tree.root_hex().unwrap());
        let admin = Addr::unchecked(ADMIN);
        let msg = InstantiateMsg {
            merkle_root: root,
            merkle_tree_uri: None,

            admins: vec![ADMIN.to_string()],
            admins_mutable: true,
        };
        let info = message_info(&admin, &[coin(CREATION_AMOUNT, NATIVE_FEE_DENOM)]);
        let res = instantiate(deps, early_mock_env(), info, msg).unwrap();
        println!("{:#?}", res);
        assert_eq!(0, res.messages.len());
        assert_eq!(5, res.attributes.len());
        (admin, tree)
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies();
        setup_contract(deps.as_mut(), None);
    }

    #[test]
    fn improper_initializations() {
        let mut deps = mock_dependencies();
        let env = custom_mock_env();
        let admin = Addr::unchecked(ADMIN);

        // Fresh valid root for test cases that are testing other validations.
        let _valid_root = build_test_tree().root_hex().unwrap();

        let invalid_msgs: Vec<InstantiateMsg> = vec![
            // invalid merkle root (non hex)
            InstantiateMsg {
                merkle_root: NON_HEX_MERKLE_ROOT.to_string(),
                merkle_tree_uri: None,

                admins: vec![admin.to_string()],
                admins_mutable: false,
            },
            // invalid merkle root (non 32 bytes)
            InstantiateMsg {
                merkle_root: NON_32BYTES_MERKLE_ROOT.to_string(),
                merkle_tree_uri: None,

                admins: vec![admin.to_string()],
                admins_mutable: false,
            },
            // // invalid mint price denom
            // InstantiateMsg {
            //     merkle_root: valid_root.clone(),
            //     merkle_tree_uri: None,

            //     admins: vec![admin.to_string()],
            //     admins_mutable: false,
            // },
            // // invalid admin address (MockApi only) (too short)
            // InstantiateMsg {
            //     merkle_root: valid_root.clone(),
            //     merkle_tree_uri: None,

            //     admins: vec!["A".to_string()],
            //     admins_mutable: false,
            // },
            // // invalid start time (after end time)
            // InstantiateMsg {
            //     merkle_root: valid_root.clone(),
            //     merkle_tree_uri: None,

            //     admins: vec![admin.to_string()],
            //     admins_mutable: false,
            // },
            // // invalid start time (before genesis mint start time)
            // InstantiateMsg {
            //     merkle_root: valid_root.clone(),
            //     merkle_tree_uri: None,

            //     admins: vec![admin.to_string()],
            //     admins_mutable: false,
            // },
            // // invalid start time (before current block time)
            // InstantiateMsg {
            //     merkle_root: valid_root.clone(),
            //     merkle_tree_uri: None,

            //     admins: vec![admin.to_string()],
            //     admins_mutable: false,
            // },
        ];

        let info = message_info(&admin, &[]);
        for msg in invalid_msgs {
            instantiate(deps.as_mut(), env.clone(), info.clone(), msg).unwrap_err();
        }
    }

    #[test]
    fn query_membership() {
        let mut deps = mock_dependencies();
        // Build tree and instantiate with its root in one step.
        let (_admin, tree) = setup_contract(deps.as_mut(), None);

        // leaf index 0: ADDR_0 (= ADMIN)
        let proof = tree.proof(&[0]);
        let res = query_has_member(
            deps.as_ref(),
            format!("{}{}", ADDR_0.to_string(), TEST_WL_ALLOCATION),
            proof.proof_hashes_hex(),
        )
        .unwrap();
        assert!(res.has_member);

        // leaf index 1: ADDR_1
        let proof = tree.proof(&[1]);
        let res = query_has_member(
            deps.as_ref(),
            format!("{}{}", ADDR_1.to_string(), TEST_WL_ALLOCATION),
            proof.proof_hashes_hex(),
        )
        .unwrap();
        assert!(res.has_member);

        // leaf index 3: ADDR_3
        let proof = tree.proof(&[3]);
        let res = query_has_member(
            deps.as_ref(),
            format!("{}{}", ADDR_3.to_string(), TEST_WL_ALLOCATION),
            proof.proof_hashes_hex(),
        )
        .unwrap();
        assert!(res.has_member);

        // mismatched proof: proof for index 1 presented with ADDR_0's address
        let wrong_proof = tree.proof(&[1]);
        let res = query_has_member(
            deps.as_ref(),
            format!("{}{}", ADDR_0.to_string(), TEST_WL_ALLOCATION),
            wrong_proof.proof_hashes_hex(),
        )
        .unwrap();
        assert!(!res.has_member);

        // invalid proof hashes (not valid hex) must error
        let bad_proof = vec!["x".to_string(), "x".to_string()];
        query_has_member(
            deps.as_ref(),
            format!("{}{}", ADDR_0.to_string(), TEST_WL_ALLOCATION),
            bad_proof,
        )
        .unwrap_err();
    }
}
