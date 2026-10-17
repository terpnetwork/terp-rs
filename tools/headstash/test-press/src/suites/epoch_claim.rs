//! cw-orch Mock exercise of the headstash epoch.
//!
//! The chain stores the Hashmerchant aggregate of the accepted batch hash
//! and pays a batch only when that aggregate matches. A public-path note is paid once from the fixed
//! denomination table, and the same nullifier is refused on the Halo2 batch.
//! This runner does not start Docker and does not verify a Halo2 proof.

use cosmwasm_std::testing::MockApi;
use cosmwasm_std::{Binary, Coin, Uint256};
use cw_headstash::epoch::{
    accepted_root, attested_root, pedersen_commit, root_b64, EpochPayout, HashMerchantSudoMsg,
    HashMerchantSudoPayload, PublicOpen, ALGO_ACCEPTED, ALGO_PENDING, CHAIN_UID,
};
use cw_headstash::interface::HeadstashContract;
use cw_headstash::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use cw_headstash::tokenfactory::HeadstashTokenObject;
use cw_headstash::wavs::generate_test_wavs_proof;
use cw_orch::prelude::*;

const DENOM: &str = "cht";

fn nf(byte: u8) -> Binary {
    Binary::from(vec![byte; 32])
}

fn payout(byte: u8, dest: &str, amount: u64) -> EpochPayout {
    EpochPayout {
        nullifier: nf(byte),
        denom: HeadstashTokenObject::new(DENOM.into()).proof,
        destination: dest.into(),
        amount,
    }
}

fn sudo_msg(algo: &str, root: &[u8; 32], height: u64) -> HashMerchantSudoMsg {
    HashMerchantSudoMsg {
        hash_merchant: HashMerchantSudoPayload {
            chain_uid: CHAIN_UID.into(),
            algo: algo.into(),
            height,
            root: root_b64(root),
            attestation_count: 1,
            block_time: 1,
            oracle_sources: vec![],
        },
    }
}

fn deploy(mock: &Mock) -> HeadstashContract<Mock> {
    let hs = HeadstashContract::new(mock.clone());
    hs.upload().expect("upload");
    mock.add_balance(&mock.sender_addr(), vec![Coin::new(1_000u64, DENOM)])
        .expect("fund sender");
    hs.instantiate(
        &InstantiateMsg {
            genesis_root: Binary::from(vec![0u8; 32]),
            distro_hash_domain: Default::default(),
            genesis_label: Some("epoch".into()),
            token_strategy: cw_headstash::tokenfactory::TokenStrategy::ExistingFungible(
                HeadstashTokenObject::new(DENOM.into()),
            ),
            wavs: generate_test_wavs_proof(1),
            circuit_id: None,
        },
        Some(&mock.sender_addr()),
        &[Coin::new(1_000u64, DENOM)],
    )
    .expect("instantiate");
    hs
}

fn wasm_sudo(mock: &Mock, addr: &cosmwasm_std::Addr, msg: &HashMerchantSudoMsg) -> Result<(), String> {
    mock.app
        .borrow_mut()
        .wasm_sudo(addr.clone(), msg)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_chain_pays_once_and_keeps_the_paths_apart() {
        let mock = Mock::new("creator");
        let dest = MockApi::default().addr_make("dest");
        let hs = deploy(&mock);
        let addr = hs.address().expect("addr");
        let root = Binary::from(vec![0u8; 32]);

        let wrong_chain = HashMerchantSudoMsg {
            hash_merchant: HashMerchantSudoPayload {
                chain_uid: "ethereum-mainnet".into(),
                algo: ALGO_ACCEPTED.into(),
                height: 1,
                root: root_b64(&[1u8; 32]),
                attestation_count: 1,
                block_time: 1,
                oracle_sources: vec![],
            },
        };
        assert!(wasm_sudo(&mock, &addr, &wrong_chain)
            .unwrap_err()
            .contains("chain_uid"));

        wasm_sudo(&mock, &addr, &sudo_msg(ALGO_PENDING, &[9u8; 32], 2)).unwrap();
        let early = hs.execute(
            &ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![payout(1, dest.as_str(), 15)],
            },
            &[],
        );
        assert!(early.unwrap_err().to_string().contains("no accepted root"));
        assert_eq!(mock.query_balance(&dest, DENOM).unwrap(), Uint256::zero());

        let row = payout(1, dest.as_str(), 15);
        let hash = accepted_root(root.as_slice(), &[row.clone()]).unwrap();
        let attested = attested_root(&hash);
        wasm_sudo(&mock, &addr, &sudo_msg(ALGO_ACCEPTED, &attested, 4)).unwrap();
        let stored: cw_headstash::epoch::EpochRootResponse = hs
            .query(&QueryMsg::EpochRoot {
                algo: ALGO_ACCEPTED.into(),
            })
            .unwrap();
        assert_eq!(stored.root.as_slice(), attested.as_slice());
        assert_eq!(stored.height, 4);

        let mismatch = hs.execute(
            &ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![payout(2, dest.as_str(), 15)],
            },
            &[],
        );
        assert!(mismatch.unwrap_err().to_string().contains("does not match"));
        assert_eq!(mock.query_balance(&dest, DENOM).unwrap(), Uint256::zero());

        hs.execute(
            &ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![row],
            },
            &[],
        )
        .expect("one credit");
        assert_eq!(
            mock.query_balance(&dest, DENOM).unwrap(),
            Uint256::from(15u64)
        );
        assert_eq!(
            mock.query_balance(&addr, DENOM).unwrap(),
            Uint256::from(985u64)
        );

        let replay = hs.execute(
            &ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root.clone(),
                root_id: 0,
                rows: vec![payout(1, dest.as_str(), 15)],
            },
            &[],
        );
        assert!(replay
            .unwrap_err()
            .to_string()
            .contains("nullifier already exists"));
        assert_eq!(
            mock.query_balance(&dest, DENOM).unwrap(),
            Uint256::from(15u64)
        );

        hs.execute(
            &ExecuteMsg::SetDenomTable {
                amounts: vec![10, 20],
            },
            &[],
        )
        .unwrap();
        hs.execute(
            &ExecuteMsg::AssignPublicNotes {
                nullifiers: vec![nf(4)],
            },
            &[],
        )
        .unwrap();
        let public_row = payout(4, dest.as_str(), 10);
        let public_hash = accepted_root(root.as_slice(), &[public_row.clone()]).unwrap();
        wasm_sudo(
            &mock,
            &addr,
            &sudo_msg(ALGO_ACCEPTED, &attested_root(&public_hash), 5),
        )
        .unwrap();
        let blocked = hs.execute(
            &ExecuteMsg::ApplyAcceptedBatch {
                allocation_root: root,
                root_id: 0,
                rows: vec![public_row],
            },
            &[],
        );
        assert!(blocked.unwrap_err().to_string().contains("public path"));
        assert_eq!(
            mock.query_balance(&dest, DENOM).unwrap(),
            Uint256::from(15u64)
        );

        let blind = [8u8; 32];
        let commitment = pedersen_commit(dest.as_str(), &blind).unwrap();
        let open = PublicOpen {
            index: 1,
            destination: dest.to_string(),
            blind: Binary::from(blind.to_vec()),
            commitment: Binary::from(commitment.to_vec()),
            tx_hash: "0xabc".into(),
            log_index: 2,
            nullifier: nf(4),
        };
        hs.execute(&ExecuteMsg::ApplyPublicBatch { rows: vec![open.clone()] }, &[])
            .expect("public credit");
        assert_eq!(
            mock.query_balance(&dest, DENOM).unwrap(),
            Uint256::from(35u64)
        );

        let again = hs.execute(
            &ExecuteMsg::ApplyPublicBatch { rows: vec![open] },
            &[],
        );
        assert!(again.unwrap_err().to_string().contains("already"));

        let other = MockApi::default().addr_make("other");
        let bad = PublicOpen {
            index: 0,
            destination: other.to_string(),
            blind: Binary::from(blind.to_vec()),
            commitment: Binary::from(pedersen_commit(other.as_str(), &blind).unwrap().to_vec()),
            tx_hash: "0xdef".into(),
            log_index: 0,
            nullifier: nf(5),
        };
        let unreserved = hs.execute(
            &ExecuteMsg::ApplyPublicBatch { rows: vec![bad] },
            &[],
        );
        assert!(unreserved
            .unwrap_err()
            .to_string()
            .contains("not on the public path"));
        assert_eq!(
            mock.query_balance(&dest, DENOM).unwrap(),
            Uint256::from(35u64)
        );
        assert_eq!(mock.query_balance(&other, DENOM).unwrap(), Uint256::zero());
    }
}
