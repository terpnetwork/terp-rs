//! Loan, raffle, and one cw721-svg collection, deployed together.
//!
//! `terp_orch::TerpArtifactStack` carries the same contracts on
//! `products.loan`, `products.raffle`, and `infuser.svg`.
//!
//! Scripts and end-to-end tests call [`NftfiSuite::deploy_on`]. The collection
//! is a [`nftfi_utils::svg::SvgDrop`], so a caller defines the template instead
//! of uploading a stock cw721.

use cosmwasm_std::{Coin, Decimal, coin};
use cw_orch::prelude::*;
use cw721_svg::Cw721SvgContractSuite;
use nftfi_loan::NftfiLoan;
use nftfi_raffle::NftfiRaffle;
use nftfi_utils::svg::SvgDrop;

#[derive(Clone)]
pub struct NftfiDeployData {
    pub admin: Addr,
    pub treasury: String,
    pub fee_addr: String,
    pub collection: SvgDrop,
    pub raffle_fee: Decimal,
    pub loan_fee: Decimal,
    pub creation_coins: Option<Vec<Coin>>,
    pub listing_fee_coins: Option<Vec<Coin>>,
}

impl NftfiDeployData {
    /// Lab fees: 100 uterp to open a raffle, 50 uterp to list a loan, 10% cuts.
    pub fn lab(admin: Addr, collection: SvgDrop) -> Self {
        let treasury = admin.to_string();
        Self {
            fee_addr: treasury.clone(),
            treasury,
            admin,
            collection,
            raffle_fee: Decimal::percent(10),
            loan_fee: Decimal::percent(10),
            creation_coins: Some(vec![coin(100, "uterp")]),
            listing_fee_coins: Some(vec![coin(50, "uterp")]),
        }
    }
}

pub struct NftfiSuite<Chain> {
    pub chain: Chain,
    pub loan: NftfiLoan<Chain>,
    pub raffle: NftfiRaffle<Chain>,
    pub collection: Cw721SvgContractSuite<Chain>,
}

impl<Chain: CwEnv> NftfiSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            chain: chain.clone(),
            loan: NftfiLoan::new(chain.clone()),
            raffle: NftfiRaffle::new(chain.clone()),
            collection: Cw721SvgContractSuite::new(chain),
        }
    }
}

impl<Chain: CwEnv> Deploy<Chain> for NftfiSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = NftfiDeployData;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = Self::new(chain);
        suite.collection.upload()?;
        suite.loan.upload()?;
        suite.raffle.upload()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![
            Box::new(&mut self.collection),
            Box::new(&mut self.loan),
            Box::new(&mut self.raffle),
        ]
    }

    fn load_from(_chain: Chain) -> Result<Self, Self::Error> {
        Err(CwOrchError::NotImplemented {})
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = Self::store_on(chain)?;
        let admin = &data.admin;
        suite.collection.instantiate(
            &data.collection.instantiate_msg(&admin.to_string()),
            Some(admin),
            &[],
        )?;
        suite.raffle.instantiate(
            &nftfi_raffle::InstantiateMsg {
                name: format!("{} raffle", data.collection.name),
                owner: Some(admin.to_string()),
                fee_addr: data.fee_addr,
                minimum_raffle_duration: Some(1),
                max_tickets_per_raffle: Some(100),
                raffle_fee: data.raffle_fee,
                oracle_pricing: None,
                creation_coins: data.creation_coins,
                randomness: nftfi_randomness::RandomnessSourceMsg::Seed { provider: None },
            },
            Some(admin),
            &[],
        )?;
        suite.loan.instantiate(
            &nftfi_loan::InstantiateMsg {
                name: format!("{} loans", data.collection.name),
                owner: Some(admin.to_string()),
                treasury_addr: data.treasury,
                fee_rate: data.loan_fee,
                listing_fee_coins: data.listing_fee_coins,
            },
            Some(admin),
            &[],
        )?;
        Ok(suite)
    }
}
