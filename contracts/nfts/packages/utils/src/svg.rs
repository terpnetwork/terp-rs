//! SVG collection definitions the raffle and loan suites instantiate.
//!
//! A drop is a name, a template, and its variables. Tests and `deploy_on`
//! pass one of these into `cw721-svg` instead of the in-memory cw721 double.

use cosmwasm_std::Binary;
use cw_svg::{VariableDef, VariableKind};

/// One cw721-svg collection. Token ids are minted in order, starting at `"0"`,
/// and the minter is the transaction sender.
#[derive(Clone, Debug)]
pub struct SvgDrop {
    pub name: String,
    pub symbol: String,
    pub template: String,
    pub variables: Vec<VariableDef>,
    pub total: u64,
    pub seed: Vec<u8>,
}

impl SvgDrop {
    pub fn two_tone(name: &str, symbol: &str) -> Self {
        Self {
            name: name.into(),
            symbol: symbol.into(),
            template: "<svg><circle fill='${color_yin}' /><path fill='${color_yang}' /></svg>"
                .into(),
            variables: vec![
                options(
                    "color_yin",
                    &["rgb(64,38,14)", "rgb(34,34,37)", "rgb(177,182,180)"],
                ),
                options(
                    "color_yang",
                    &["rgb(141,93,51)", "rgb(56,56,64)", "rgb(208,229,226)"],
                ),
            ],
            total: 32,
            seed: b"nftfi-two-tone-seed-0123456789".to_vec(),
        }
    }

    pub fn solid(name: &str, symbol: &str) -> Self {
        Self {
            name: name.into(),
            symbol: symbol.into(),
            template: "<svg><rect fill='${fill}' width='8' height='8'/></svg>".into(),
            variables: vec![options("fill", &["#111111", "#eeeeee", "#336699"])],
            total: 32,
            seed: b"nftfi-solid-seed-0123456789ab".to_vec(),
        }
    }

    pub fn instantiate_msg(&self, admin: &str) -> cw721_svg::InstantiateMsg {
        cw721_svg::InstantiateMsg {
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            minter: None,
            creator: Some(admin.to_string()),
            withdraw_address: Some(admin.to_string()),
            collection_info_extension: cw721_svg::SvgCollectionMetadata {
                total: self.total,
                seed: Binary::from(self.seed.clone()),
                mint_start_time: None,
                mint_end_time: None,
                price_tiers: Vec::new(),
                whitelist: None,
                require_auth: false,
                packed: cw721_svg::pack_svg(&self.template, &self.variables),
            },
        }
    }
}

/// The two lab collections. Functional tests run once per drop.
pub fn lab() -> Vec<SvgDrop> {
    vec![
        SvgDrop::two_tone("NftFi", "NFI"),
        SvgDrop::solid("NftFi Solid", "NFS"),
    ]
}

/// Open mint of `amount` tokens. cw721-svg rejects the stock `Mint` variant.
pub fn mint_msg(amount: u64) -> cw721_svg::ExecuteMsg {
    cw721_svg::ExecuteMsg::UpdateExtension {
        msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
            alloc: 0,
            amnt: amount,
            proof_hashes: Vec::new(),
            ext: None,
        }),
    }
}

#[cfg(feature = "testing")]
pub fn contract() -> Box<dyn cw_multi_test::Contract<cosmwasm_std::Empty>> {
    Box::new(cw_multi_test::ContractWrapper::new_with_empty(
        cw721_svg::entry::execute,
        cw721_svg::entry::instantiate,
        cw721_svg::entry::query,
    ))
}

fn options(name: &str, values: &[&str]) -> VariableDef {
    VariableDef {
        name: name.to_string(),
        kind: VariableKind::Options(values.iter().map(|v| (*v).to_string()).collect()),
    }
}
