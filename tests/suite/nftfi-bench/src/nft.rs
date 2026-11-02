//! cw721-base collateral. cw721-svg in this worktree does not compile
//! (`execute_mint` uses `m.amnt` after moving `m`).

use anyhow::Context;
use cosmwasm_std::{Addr, Empty};
use cw_orch::prelude::*;
use cw721::msg::OwnerOfResponse;
use cw721_base::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

#[cw_orch::interface(InstantiateMsg, ExecuteMsg, QueryMsg, Empty, id = "cw721_base")]
pub struct BenchNft;

impl<Chain: CwEnv> Uploadable for BenchNft<Chain> {
    fn wasm(_chain: &ChainInfoOwned) -> WasmPath {
        panic!("nftfi-bench uploads cw721-base through the mock wrapper");
    }

    fn wrapper() -> Box<dyn MockContract<Empty>> {
        Box::new(ContractWrapper::new(
            cw721_base::entry::execute,
            cw721_base::entry::instantiate,
            cw721_base::entry::query,
        ))
    }
}

pub fn deploy(chain: &MockBech32, admin: &Addr) -> anyhow::Result<BenchNft<MockBech32>> {
    let nft = BenchNft::new(chain.clone());
    nft.call_as(admin).upload().context("upload cw721-base")?;
    nft.call_as(admin)
        .instantiate(
            &InstantiateMsg {
                name: "nftfi-bench".into(),
                symbol: "BENCH".into(),
                collection_info_extension: None,
                minter: Some(admin.to_string()),
                creator: Some(admin.to_string()),
                withdraw_address: Some(admin.to_string()),
            },
            Some(admin),
            &[],
        )
        .context("instantiate cw721-base")?;
    for id in 0..6 {
        nft.call_as(admin)
            .execute(
                &ExecuteMsg::Mint {
                    token_id: id.to_string(),
                    owner: admin.to_string(),
                    token_uri: None,
                    extension: None,
                },
                &[],
            )
            .with_context(|| format!("mint {id}"))?;
    }
    Ok(nft)
}

pub fn transfer(
    nft: &BenchNft<MockBech32>,
    from: &Addr,
    token_id: &str,
    recipient: &Addr,
) -> anyhow::Result<()> {
    nft.call_as(from)
        .execute(
            &ExecuteMsg::TransferNft {
                recipient: recipient.to_string(),
                token_id: token_id.to_string(),
            },
            &[],
        )
        .with_context(|| format!("transfer {token_id}"))?;
    Ok(())
}

pub fn approve(
    nft: &BenchNft<MockBech32>,
    owner: &Addr,
    spender: &Addr,
    token_id: &str,
) -> anyhow::Result<()> {
    nft.call_as(owner)
        .execute(
            &ExecuteMsg::Approve {
                spender: spender.to_string(),
                token_id: token_id.to_string(),
                expires: None,
            },
            &[],
        )
        .with_context(|| format!("approve {token_id}"))?;
    Ok(())
}

pub fn owner(nft: &BenchNft<MockBech32>, token_id: &str) -> anyhow::Result<String> {
    let resp: OwnerOfResponse = nft
        .query(&QueryMsg::OwnerOf {
            token_id: token_id.to_string(),
            include_expired: None,
        })
        .with_context(|| format!("owner of {token_id}"))?;
    Ok(resp.owner)
}
