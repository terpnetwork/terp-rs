//! `SvgCollectionSuite`: a small, generic cw-orch suite for one-collection-per-config
//! cw721-svg drops (Latent Benders days, Marain batches, Yi Jing, soon/ launchpad).
//!
//! Unlike `CwSvgSuite` (minter factory + infuser + shitstrap + whitelist bag) this only
//! stores cw721-svg and instantiates collections directly, so it runs on `MockBech32`,
//! clone-testing and a loopback `Daemon` alike. Compose it next to `CwSvgSuite` when a
//! flow needs the factory.
use anyhow::{Context, Result, ensure};
use cosmwasm_std::{Binary, Coin, Timestamp};
use cw_orch::prelude::*;
use cw721_svg::Cw721SvgContractSuite;
use sha2::{Digest, Sha256};

/// Everything needed to instantiate one cw721-svg collection.
#[derive(Clone, Debug)]
pub struct SvgCollectionConfig {
    pub name: String,
    pub symbol: String,
    /// On-chain `seed` (Benders: the 32 raw bytes of record_sha256).
    pub seed: Binary,
    /// SVP1-packed template + variables.
    pub packed: Binary,
    pub total: u64,
    /// Empty = free mint.
    pub price_tiers: Vec<cw721_svg::PriceTier>,
    pub mint_start_time: Option<Timestamp>,
    pub mint_end_time: Option<Timestamp>,
    /// Used when mint_end_time is None: mint closes this many seconds after instantiate.
    pub mint_end_after_secs: Option<u64>,
    /// Creator owner (pause/scribing admin) and wasm admin.
    pub creator: Addr,
    /// None = open mint (no minter gate).
    pub minter: Option<Addr>,
    pub withdraw_address: Option<Addr>,
    pub require_auth: bool,
    pub label: String,
}

impl SvgCollectionConfig {
    pub fn packed_sha256(&self) -> String {
        hex::encode(Sha256::digest(self.packed.as_slice()))
    }

    /// cw721-svg takes mint_start_time / mint_end_time as OFFSETS (seconds after the
    /// instantiate block) and stores them absolute. Configs hold absolute times; this
    /// converts against `now` (the expected instantiate block time).
    pub fn to_init_msg(&self, now: Timestamp) -> Result<cw721_svg::InstantiateMsg> {
        let off = |t: Option<Timestamp>, what: &str| -> Result<Option<Timestamp>> {
            t.map(|t| {
                ensure!(t > now, "{what} {t} is not after the instantiate block time {now}");
                Ok(Timestamp::from_seconds(t.seconds() - now.seconds()))
            })
            .transpose()
        };
        let mint_start_time = off(self.mint_start_time, "mint_start_time")?;
        let mint_end_time = match self.mint_end_time {
            Some(_) => off(self.mint_end_time, "mint_end_time")?,
            None => self.mint_end_after_secs.map(Timestamp::from_seconds),
        };
        Ok(cw721_svg::InstantiateMsg {
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            collection_info_extension: cw721_svg::SvgCollectionMetadata {
                seed: self.seed.clone(),
                packed: self.packed.clone(),
                price_tiers: self.price_tiers.clone(),
                total: self.total,
                mint_start_time,
                mint_end_time,
                whitelist: None,
                require_auth: self.require_auth,
            },
            minter: self.minter.as_ref().map(|a| a.to_string()),
            creator: Some(self.creator.to_string()),
            withdraw_address: self.withdraw_address.as_ref().map(|a| a.to_string()),
        })
    }
}

/// What a deployed collection reports back (for assertions and run reports).
#[derive(Clone, Debug, serde::Serialize)]
pub struct SvgCollectionState {
    pub address: String,
    pub name: String,
    pub seed_hex: String,
    pub packed_sha256: String,
    pub packed_bytes: usize,
    pub mint_start_time: Option<Timestamp>,
    pub mint_end_time: Option<Timestamp>,
    pub total: u64,
    pub minted: u64,
}

pub struct SvgCollectionSuite<Chain> {
    pub chain: Chain,
    /// Shared cw721-svg handle; its address is re-pointed per collection.
    pub cwsvg: Cw721SvgContractSuite<Chain>,
    pub collections: Vec<Addr>,
}

impl<Chain: CwEnv> SvgCollectionSuite<Chain> {
    pub fn new(chain: Chain) -> Self {
        Self {
            cwsvg: Cw721SvgContractSuite::new(chain.clone()),
            chain,
            collections: vec![],
        }
    }

    pub fn upload(&self) -> Result<u64> {
        self.cwsvg.upload_if_needed()?;
        Ok(self.cwsvg.code_id()?)
    }

    /// Point the shared handle at `addr` and return it.
    pub fn at(&self, addr: &Addr) -> &Cw721SvgContractSuite<Chain> {
        self.cwsvg.set_address(addr);
        &self.cwsvg
    }

    /// Instantiate a collection from `cfg` (uploads first if needed).
    pub fn instantiate(&mut self, cfg: &SvgCollectionConfig) -> Result<Addr> {
        ensure!(!cfg.packed.is_empty(), "packed svg is required");
        ensure!(cfg.seed.len() == 32, "seed must be 32 bytes");
        if let (Some(s), Some(e)) = (cfg.mint_start_time, cfg.mint_end_time) {
            ensure!(s < e, "mint_start_time must be before mint_end_time");
        }
        self.upload()?;
        // On a Daemon the tx lands a few seconds after this read, so stored times may
        // drift later by that much; on Mock they are exact.
        let now = self.chain.block_info()?.time;
        let addr = self
            .cwsvg
            .instantiate(&cfg.to_init_msg(now)?, Some(&cfg.creator), &[])?
            .instantiated_contract_address()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        self.cwsvg.set_address(&addr);
        self.collections.push(addr.clone());
        Ok(addr)
    }

    /// Open mint = the block time is inside [start, end] and the collection is not paused.
    /// Pause is a toggle in cw721-svg, so it is only sent when `unpause` is true.
    pub fn open_mint(&self, addr: &Addr, unpause: bool) -> Result<bool> {
        let st = self.query(addr)?;
        if unpause {
            self.at(addr).execute(
                &cw721_svg::ExecuteMsg::UpdateExtension {
                    msg: cw721_svg::SvgExecuteMsgExt::Pause { pause: false },
                },
                &[],
            )?;
        }
        let now = self.chain.block_info()?.time;
        Ok(st.mint_start_time.is_none_or(|s| now >= s) && st.mint_end_time.is_none_or(|e| now <= e))
    }

    pub fn mint(&self, addr: &Addr, amnt: u64, funds: &[Coin]) -> Result<()> {
        self.at(addr).execute(
            &cw721_svg::ExecuteMsg::UpdateExtension {
                msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
                    amnt,
                    proof_hashes: vec![],
                    alloc: 0,
                    ext: None,
                }),
            },
            funds,
        )?;
        Ok(())
    }

    pub fn query(&self, addr: &Addr) -> Result<SvgCollectionState> {
        let c = self.at(addr);
        let info: cw721::msg::CollectionInfoAndExtensionResponse<cw721_svg::SvgCollectionMetadata> =
            c.query(&cw721_svg::QueryMsg::GetCollectionInfoAndExtension {})
                .context("collection info")?;
        let packed: Binary = c.query(&cw721_svg::QueryMsg::Extension {
            msg: cw721_svg::SvgQueryMsgExt::SvgPacked {},
        })?;
        let minted: cw721::msg::NumTokensResponse = c.query(&cw721_svg::QueryMsg::NumTokens {})?;
        Ok(SvgCollectionState {
            address: addr.to_string(),
            name: info.name,
            seed_hex: hex::encode(info.extension.seed.as_slice()),
            packed_sha256: hex::encode(Sha256::digest(packed.as_slice())),
            packed_bytes: packed.len(),
            mint_start_time: info.extension.mint_start_time,
            mint_end_time: info.extension.mint_end_time,
            total: info.extension.total,
            minted: minted.count,
        })
    }
}

// ---------------------------------------------------------------------------------------
// Factory + series (contract-enforced cadence) and feeshare.
// ---------------------------------------------------------------------------------------
pub use cw_svg_minter::state::SeriesConfig;
use cw_svg_minter::interface::Cw721SvgMinter;

/// Latent Benders: one collection per day. A 23h min interval tolerates scheduler jitter
/// while making two drops inside one day impossible; mint window capped at 24h.
pub fn benders_series(operators: Vec<String>) -> SeriesConfig {
    SeriesConfig {
        operators,
        min_interval_secs: Some(23 * 3600),
        max_per_window: None,
        window_secs: None,
        max_mint_window_secs: Some(86_400),
    }
}

/// Marain orbs: up to 3 per rolling day.
pub fn marain_series(operators: Vec<String>) -> SeriesConfig {
    SeriesConfig {
        operators,
        max_per_window: Some(3),
        window_secs: Some(86_400),
        ..Default::default()
    }
}

pub struct SvgFactory<Chain> {
    pub minter: Cw721SvgMinter<Chain>,
}

impl<Chain: CwEnv> SvgCollectionSuite<Chain> {
    /// Upload cw721-svg + the factory and instantiate it (owner = `owner`, wasm admin =
    /// `admin`, so `admin` can register its feeshare).
    pub fn deploy_factory(&self, owner: Option<&Addr>, admin: &Addr) -> Result<SvgFactory<Chain>> {
        let code_id = self.upload()?;
        let minter = Cw721SvgMinter::new(self.chain.clone());
        minter.upload_if_needed()?;
        minter.instantiate(
            &cw_svg_minter::msg::InstantiateMsg {
                owner: owner.map(|a| a.to_string()),
                svg_code_id: code_id,
                dregg_ibc_only: false,
                runtime_minters: vec![],
                oracle_pricing: None,
            },
            Some(admin),
            &[],
        )?;
        Ok(SvgFactory { minter })
    }

    pub fn set_series(&self, f: &SvgFactory<Chain>, series: &str, cfg: Option<SeriesConfig>) -> Result<()> {
        f.minter.execute(
            &cw_svg_minter::msg::ExecuteMsg::SetSeries { series: series.into(), config: cfg },
            &[],
        )?;
        Ok(())
    }

    pub fn series_info(&self, f: &SvgFactory<Chain>, series: &str) -> Result<cw_svg_minter::msg::SeriesInfo> {
        Ok(f.minter.query(&cw_svg_minter::msg::QueryMsg::Series { series: series.into() })?)
    }

    /// Canonical path: create through the factory under a rate-limited series. The factory
    /// enforces the cadence; the collection wasm admin = cfg.creator.
    pub fn instantiate_in_series(&mut self, f: &SvgFactory<Chain>, series: &str, cfg: &SvgCollectionConfig) -> Result<Addr> {
        ensure!(!cfg.packed.is_empty(), "packed svg is required");
        ensure!(cfg.seed.len() == 32, "seed must be 32 bytes");
        let now = self.chain.block_info()?.time;
        let res = f.minter.execute(
            &cw_svg_minter::msg::ExecuteMsg::CreateSeriesCollection {
                series: series.into(),
                instantiate_msg: cfg.to_init_msg(now)?,
                label: cfg.label.clone(),
            },
            &[],
        )?;
        let addr = res
            .event_attr_value("wasm", "contract")
            .map_err(|e| anyhow::anyhow!("no contract attr: {e}"))?;
        let addr = Addr::unchecked(addr);
        self.cwsvg.set_address(&addr);
        self.collections.push(addr.clone());
        Ok(addr)
    }
}

/// `/terp.feeshare.v1.MsgRegisterFeeShare` as a protobuf Any. The keeper requires
/// deployer == the contract wasm admin (or its creator when there is no admin).
pub mod feeshare {
    pub const TYPE_URL: &str = "/terp.feeshare.v1.MsgRegisterFeeShare";

    fn field(out: &mut Vec<u8>, tag: u8, s: &str) {
        if s.is_empty() {
            return; // proto3 default is omitted
        }
        out.push((tag << 3) | 2);
        let mut n = s.len();
        while n >= 0x80 {
            out.push((n as u8) | 0x80);
            n >>= 7;
        }
        out.push(n as u8);
        out.extend_from_slice(s.as_bytes());
    }

    pub fn encode(contract: &str, deployer: &str, withdrawer: &str) -> Vec<u8> {
        let mut v = vec![];
        field(&mut v, 1, contract);
        field(&mut v, 2, deployer);
        field(&mut v, 3, withdrawer);
        v
    }

    pub fn msg(contract: &str, deployer: &str, withdrawer: &str) -> cosmrs::Any {
        cosmrs::Any { type_url: TYPE_URL.into(), value: encode(contract, deployer, withdrawer) }
    }

    /// Broadcast on a Daemon signed by `admin_chain` (must be the contract admin).
    /// Not available on Mock: cw-multi-test cannot execute custom Any msgs.
    pub fn register(
        admin_chain: &cw_orch::daemon::Daemon,
        contract: &str,
        withdrawer: &str,
    ) -> anyhow::Result<String> {
        use cw_orch::daemon::TxSender;
        use cw_orch::prelude::TxHandler;
        let deployer = admin_chain.sender_addr().to_string();
        let any = msg(contract, &deployer, withdrawer);
        let res = admin_chain
            .rt_handle
            .block_on(admin_chain.sender().commit_tx_any(vec![any], None))?;
        Ok(res.txhash)
    }
}
