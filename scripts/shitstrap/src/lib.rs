use cosmwasm_std::Decimal;
use cw_orch::prelude::*;
use cw_shitstrap::contract::interface::CwShitstrap;
use cw_shitstrap_factory::interface::CwShitstrapFactory;

pub use cw_shit_denom::UncheckedDenom;
pub use cw_shitstrap::contract::msg::{
    ExecuteMsgFns as _, InstantiateMsg as ShitInitMsg, QueryMsgFns as _,
};
pub use cw_shitstrap_factory::msg::{
    ExecuteMsgFns as _, InstantiateMsg as ShitFactoryInitMsg, QueryMsgFns as _,
};

pub const DAB: u128 = 1_000_000_000_000_000u128;
pub const CUTOFF: u128 = 500_000_000_000u128;

#[derive(Clone, Debug, Default)]
pub struct CwShitstrapSuiteDeployData {
    pub shit: Vec<ShitInitMsg>,
    pub admin: Option<Addr>,
}

pub struct CwShitstrapSuite<Chain> {
    pub chain: Chain,
    pub shitstrap: CwShitstrap<Chain>,
    pub factory: CwShitstrapFactory<Chain>,
}

impl<Chain: CwEnv> CwShitstrapSuite<Chain> {
    pub fn new(chain: Chain) -> CwShitstrapSuite<Chain> {
        CwShitstrapSuite::<Chain> {
            chain: chain.clone(),
            shitstrap: CwShitstrap::new(chain.clone()),
            factory: CwShitstrapFactory::new(chain.clone()),
        }
    }
    pub fn upload_if_needed(&self) -> Result<(), CwOrchError> {
        self.shitstrap.upload_if_needed()?;
        self.factory.upload_if_needed()?;
        Ok(())
    }
    pub fn upload(&self) -> Result<(), CwOrchError> {
        self.shitstrap.upload()?;
        self.factory.upload()?;
        Ok(())
    }
}

impl<Chain: CwEnv> cw_orch::contract::Deploy<Chain> for CwShitstrapSuite<Chain> {
    type Error = CwOrchError;
    type DeployData = Option<CwShitstrapSuiteDeployData>;

    fn store_on(chain: Chain) -> Result<Self, Self::Error> {
        let suite = CwShitstrapSuite::new(chain.clone());
        suite.upload_if_needed()?;
        Ok(suite)
    }

    fn get_contracts_mut(&mut self) -> Vec<Box<&mut dyn ContractInstance<Chain>>> {
        vec![Box::new(&mut self.shitstrap), Box::new(&mut self.factory)]
    }

    fn load_from(_chain: Chain) -> Result<Self, Self::Error> {
        todo!()
    }

    fn deploy_on(chain: Chain, data: Self::DeployData) -> Result<Self, Self::Error> {
        let suite = CwShitstrapSuite::store_on(chain.clone())?;
        match data {
            Some(d) => {
                // Always create a factory on deploy
                suite.factory.instantiate(
                    &ShitFactoryInitMsg {
                        owner: d
                            .admin
                            .as_ref()
                            .map(|d| Some(d.to_string()))
                            .unwrap_or_default(),
                        shitstrap_id: suite.shitstrap.code_id()?,
                    },
                    d.admin.as_ref(),
                    &[],
                )?;

                for (_, init) in d.shit.iter().enumerate() {
                    let _res = suite.factory.create_native_shit_strap_contract(
                        init.clone(),
                        format!("{}", init.title),
                    )?;
                }

                Ok(suite)
            }
            None => Err(CwOrchError::NotImplemented {}),
        }
    }
}

// ---------------------------------------------------------------------------
// shitstrap deploy data builders
// ---------------------------------------------------------------------------

/// Single shitstrap: 20 THIOL in -> 1 TERP out.
pub fn shit_deploy_data_single(admin: Addr) -> Option<CwShitstrapSuiteDeployData> {
    let mut dd = CwShitstrapSuiteDeployData::default();
    dd.admin = Some(admin.clone());
    dd.shit = vec![ShitInitMsg {
        daos: Vec::new(),
        owner: Some(admin.to_string()),
        accepted: vec![cw_shit_denom::PossibleShit::native_denom(
            "uthiol",
            50_000_000_000_000_000u128,
        )],
        cutoff: 500_000_000_000u128.into(),
        shitmos: UncheckedDenom::Native("uterp".into()),
        title: "terp".into(),
        description: "terp".into(),
    }];
    Some(dd)
}

/// Multiple shistraps with spot-price-based exchange rates.
pub fn shit_deploy_data_full(admin: Addr) -> Option<CwShitstrapSuiteDeployData> {
    let mut dd = CwShitstrapSuiteDeployData::default();
    let cut = 710_000_000_000u128;

    let mut shit = Vec::new();

    // // atom @ $1.81
    // shit.push(build_shit_init(
    //     &admin,
    //     &tf_denom(&admin, "atom"),
    //     calc_rates(Decimal::from_ratio(181u128, 100u128)),
    //     cut,
    //     "atom",
    // ));

    // btc @ $66,350
    shit.push(build_shit_init(
        &admin,
        &tf_denom(&admin, "btc"),
        calc_rates(Decimal::from_ratio(66350u128, 1u128)),
        cut,
        "btc",
    ));

    // akt @ $0.29
    shit.push(build_shit_init(
        &admin,
        &tf_denom(&admin, "akt"),
        calc_rates(Decimal::from_ratio(29u128, 100u128)),
        cut,
        "akt",
    ));

    // // um @ $0.007
    // shit.push(build_shit_init(
    //     &admin,
    //     &tf_denom(&admin, "um"),
    //     calc_rates(Decimal::from_ratio(7u128, 1000u128)),
    //     cut,
    //     "um",
    // ));

    // eth @ $1,956.12
    shit.push(build_shit_init(
        &admin,
        &tf_denom(&admin, "eth"),
        calc_rates(Decimal::from_ratio(195612u128, 100u128)),
        cut,
        "eth",
    ));

    dd.admin = Some(admin);
    dd.shit = shit;
    Some(dd)
}


pub fn tf_denom(creator: &Addr, subdenom: &str) -> String {
    format!("factory/{}/{}", creator, subdenom)
}

pub fn build_shit_init(
    admin: &Addr,
    native_denom: &str,
    shit_rate: u128,
    cutoff: u128,
    title: &str,
) -> ShitInitMsg {
    ShitInitMsg {
        daos: Vec::new(),
        owner: Some(admin.to_string()),
        accepted: vec![cw_shit_denom::PossibleShit::native_denom(
            native_denom,
            shit_rate,
        )],
        cutoff: cutoff.into(),
        shitmos: UncheckedDenom::Native("uthiol".into()),
        title: title.into(),
        description: title.into(),
    }
}

/// Compute IBC denom hash: SHA256("transfer/{channel}/{denom}") hex-encoded
pub fn ibc_denom_hash(channel_id: &str, port_id: &str, native_denom: &str) -> String {
    use sha2::{Digest, Sha256};
    let input = format!("{}/{}/{}", port_id, channel_id, native_denom);
    let result = Sha256::digest(input.as_bytes());
    let hex_str: String = result.iter().map(|b| format!("{:02X}", b)).collect();
    format!("ibc/{}", hex_str)
}

/// One shitstrap, several denoms of the same asset. cw-shitstrap rejects an
/// empty set and a set of exactly four.
fn build_shit_init_paths(
    admin: &Addr,
    denoms: &[&str],
    shit_rate: u128,
    cutoff: u128,
    title: &str,
) -> ShitInitMsg {
    assert!(
        !denoms.is_empty() && denoms.len() != 4,
        "cw-shitstrap rejects empty and exactly-four accepted sets"
    );
    ShitInitMsg {
        daos: Vec::new(),
        owner: Some(admin.to_string()),
        accepted: denoms
            .iter()
            .map(|denom| cw_shit_denom::PossibleShit::native_denom(denom, shit_rate))
            .collect(),
        cutoff: cutoff.into(),
        shitmos: UncheckedDenom::Native("uthiol".into()),
        title: title.into(),
        description: title.into(),
    }
}

pub fn new_default_ibc_set(admin: Addr) -> CwShitstrapSuiteDeployData {
    let mut shit = Vec::new();
    let mut dd = CwShitstrapSuiteDeployData::default();
    let cut = 710_000_000_000u128;

    // Newest open paths are the temporary mint denoms. The older channel is
    // kept as a second accepted option for when that canonical path reopens.
    let akt = ibc_denom_hash("channel-9", "transfer", "uakt");
    assert_eq!(
        &akt,
        "ibc/ED3433FEFECE0B9EC584F0EBAB31E8AB6A7A187537F3E5B1D8A80998986E7ACF"
    );
    let akt_canonical = ibc_denom_hash("channel-6", "transfer", "uakt");
    assert_eq!(
        &akt_canonical,
        "ibc/2E4CF68617591638A6CBD7DA3E8CAF8C9C0B34AAB4878775A0B16ADDC56E4389"
    );
    let osmo = ibc_denom_hash("channel-11", "transfer", "uosmo");
    assert_eq!(
        &osmo,
        "ibc/4646A60D9F2EC44B281852B38FE269E2D0870DA7A3BBD805DB5F9CD7AF0D6280"
    );
    let osmo_canonical = ibc_denom_hash("channel-1", "transfer", "uosmo");
    assert_eq!(
        &osmo_canonical,
        "ibc/0471F1C4E7AFD3F07702BEF6DC365268D64570F7C1FDC98EA6098DD6DE59817B"
    );
    // Osmosis trace transfer/channel-79703/upenumbra is ibc/0FA9232B… (chain registry).
    // On Terp that packet arrives through channel-11 / client 07-tendermint-33.
    let um_on_osmosis = ibc_denom_hash("channel-79703", "transfer", "upenumbra");
    assert_eq!(
        &um_on_osmosis,
        "ibc/0FA9232B262B89E77D1335D54FB1E1F506A92A7E4B51524B400DC69C68D28372"
    );
    let um = ibc_denom_hash(
        "channel-11",
        "transfer",
        "transfer/channel-79703/upenumbra",
    );
    assert_eq!(
        &um,
        "ibc/6B32258CACFCBB5486A47CB1EAF4B6F5AD76C9E372E5F3CB08B3D1C606F9F39E"
    );
    let btc = ibc_denom_hash(
        "channel-11",
        "transfer",
        "factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC",
    );
    assert_eq!(
        &btc,
        "ibc/9FC418778218616ADEC741B8E402C856EBF2AADAD93A8D7EA322F798480F641C"
    );
    let btc_canonical = ibc_denom_hash(
        "channel-1",
        "transfer",
        "factory/osmo1z6r6qdknhgsc0zeracktgpcxf43j6sekq07nw8sxduc9lg0qjjlqfu25e3/alloyed/allBTC",
    );
    assert_eq!(
        &btc_canonical,
        "ibc/2E13104D0924DDB574E98CF01E1FD2D8913779BCB5B514B6468C6EC6037A15DC"
    );

    let eth = ibc_denom_hash(
        "channel-11",
        "transfer",
        "factory/osmo1k6c8jln7ejuqwtqmay3yvzrg3kueaczl96pk067ldg8u835w0yhsw27twm/alloyed/allETH",
    );
    assert_eq!(
        &eth,
        "ibc/0F92AF0DAD1FA5CBC4D8FDF9F5BF9F7F5BE36DF268B1E115DB82E1A71BE9CB31"
    );
    let eth_canonical = ibc_denom_hash(
        "channel-1",
        "transfer",
        "factory/osmo1k6c8jln7ejuqwtqmay3yvzrg3kueaczl96pk067ldg8u835w0yhsw27twm/alloyed/allETH",
    );
    assert_eq!(
        &eth_canonical,
        "ibc/D7B7D0835B5CAACA52109939CEB064D81EBC098B4E2C3FD71106FEFFC06CC308"
    );
    let zec = ibc_denom_hash(
        "channel-11",
        "transfer",
        "factory/osmo1twk0c4kcwnhkyn6pzxe0qsk6a3nrye2w2sy309d60sljfysugagsd7e3mn/alloyed/allZEC",
    );
    assert_eq!(
        &zec,
        "ibc/BC072FE6C12FD02688EA5DE89857BB269584E6DD378170080C3E1463825B9F81"
    );
    let zec_canonical = ibc_denom_hash(
        "channel-1",
        "transfer",
        "factory/osmo1twk0c4kcwnhkyn6pzxe0qsk6a3nrye2w2sy309d60sljfysugagsd7e3mn/alloyed/allZEC",
    );
    assert_eq!(
        &zec_canonical,
        "ibc/910DBA7D901A0A4985457CFF8C0C54BF7DE14D59B9A91D3FF1E926906C5B1DF1"
    );

    let atone = ibc_denom_hash("channel-10", "transfer", "uatone");
    let photon = ibc_denom_hash("channel-10", "transfer", "uphoton");

    // btc @ $66,350. Newest channel-11 stays; channel-1 is the extra option.
    shit.push(build_shit_init_paths(
        &admin,
        &[&btc, &btc_canonical],
        calc_rates(Decimal::from_ratio(66350u128, 1u128)),
        cut,
        "BTC",
    ));

    // akt @ $0.29. Newest client 07-tendermint-32 / channel-9 stays.
    shit.push(build_shit_init_paths(
        &admin,
        &[&akt, &akt_canonical],
        calc_rates(Decimal::from_ratio(29u128, 100u128)),
        cut,
        "AKT",
    ));

    // eth @ $1,956.12. Alloyed allETH on Osmosis, both Terp channels.
    shit.push(build_shit_init_paths(
        &admin,
        &[&eth, &eth_canonical],
        calc_rates(Decimal::from_ratio(195612u128, 100u128)),
        cut,
        "ETH",
    ));

    // osmo, um, zec rates are a Coingecko USD snapshot from 2026-10-08 so the
    // new rows are non-zero. They are not a replacement for the rates above.
    // osmo $0.03380079, um $0.00127324, zec $1240.81.
    shit.push(build_shit_init_paths(
        &admin,
        &[&osmo, &osmo_canonical],
        calc_rates(Decimal::from_ratio(3_380_079u128, 100_000_000u128)),
        cut,
        "OSMO",
    ));
    shit.push(build_shit_init(
        &admin,
        &um,
        calc_rates(Decimal::from_ratio(127_324u128, 100_000_000u128)),
        cut,
        "UM",
    ));
    shit.push(build_shit_init_paths(
        &admin,
        &[&zec, &zec_canonical],
        calc_rates(Decimal::from_ratio(124_081u128, 100u128)),
        cut,
        "ZEC",
    ));

    // atomone @ $0.2267
    shit.push(build_shit_init(
        &admin,
        &atone,
        calc_rates(Decimal::from_ratio(2267u128, 10000u128)),
        cut,
        "ATONE",
    ));

    // photon @ $0.03291
    shit.push(build_shit_init(
        &admin,
        &photon,
        calc_rates(Decimal::from_ratio(3291u128, 100000u128)),
        cut,
        "PHOTON",
    ));

    // CwShitstrapSuiteDeployData {
    //     admin:
    //     shit: vec![ShitInitMsg {
    //         daos: vec![],
    //         owner: Some(admin.to_string()),
    //         accepted: vec![PossibleShit::native_denom("uthiol", DAB)],
    //         cutoff: CUTOFF.into(),
    //         shitmos: UncheckedDenom::Native("uterp".into()),
    //         title: "terp".into(),
    //         description: "terp".into(),
    //     }],
    // }
    dd.admin = Some(admin.clone());
    dd.shit = shit;
    dd
}


// calc_rates: USD spot price → shit_rate at 1e18 precision.
//
// THIOL target price: $0.01 → users receive (price / 0.01) = price × 100 THIOL
// per accepted token. Raw atomics are scaled by 100 to avoid Decimal × Decimal
// overflow on high-price tokens (e.g. BTC).
//
// e.g. ATOM @ $1.81   → 181_000_000_000_000_000_000   (181 THIOL per ATOM)
//      BTC  @ $66,350 → 6_635_000_000_000_000_000_000  (6,635,000 THIOL per BTC)
pub fn calc_rates(price: Decimal) -> u128 {
    price.atomics().u128() * 100
}

#[cfg(test)]
mod temp_denom_tests {
    use super::*;

    fn native(shit: &ShitInitMsg) -> Vec<String> {
        shit.accepted
            .iter()
            .map(|row| match &row.token {
                UncheckedDenom::Native(denom) => denom.clone(),
                UncheckedDenom::Cw20(denom) => denom.clone(),
            })
            .collect()
    }

    #[test]
    fn temp_ibc_set_extends_newest_and_keeps_canonical() {
        let dd = new_default_ibc_set(Addr::unchecked("terp1admin"));
        let by_title: std::collections::BTreeMap<_, _> = dd
            .shit
            .iter()
            .map(|row| (row.title.as_str(), native(row)))
            .collect();
        assert_eq!(
            by_title["AKT"][0],
            "ibc/ED3433FEFECE0B9EC584F0EBAB31E8AB6A7A187537F3E5B1D8A80998986E7ACF"
        );
        assert_eq!(
            by_title["AKT"][1],
            "ibc/2E4CF68617591638A6CBD7DA3E8CAF8C9C0B34AAB4878775A0B16ADDC56E4389"
        );
        assert_eq!(by_title["OSMO"].len(), 2);
        assert_eq!(by_title["UM"].len(), 1);
        assert_eq!(
            by_title["UM"][0],
            "ibc/6B32258CACFCBB5486A47CB1EAF4B6F5AD76C9E372E5F3CB08B3D1C606F9F39E"
        );
        assert_eq!(by_title["ETH"].len(), 2);
        assert_eq!(by_title["ZEC"].len(), 2);
        assert_eq!(by_title["BTC"].len(), 2);
        assert!(by_title.contains_key("ATONE"));
        assert!(by_title.contains_key("PHOTON"));
        for denoms in by_title.values() {
            assert_ne!(denoms.len(), 4);
        }
    }
}