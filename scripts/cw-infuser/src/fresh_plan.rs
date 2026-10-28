//! Fresh SVG collections on the ALREADY-DEPLOYED codes only (no uploads, no migrations):
//! cw721_svg code 98, factory code 99 (instance below), whitelist-mtree code 73.
//! Builds every message as data; nothing here can sign or broadcast.
use crate::paymint;
use anyhow::Result;
use serde_json::{Value, json};

pub const CODE_CW721_SVG: u64 = 98;
pub const CODE_FACTORY: u64 = 99;
pub const CODE_WHITELIST_MTREE: u64 = 73;
pub const FACTORY: &str = "terp1wg5z9rcwpw5kvrl4g78ees2l9ejptrhrx4t2k56nq6t27uly87zssujm80";
pub const FOUNDATION_DAO: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";
pub const ZIGNER_GRANTER: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";
/// Off-chain payment minter (owner-confirmed 2026-10-28). Not yet an on-chain account:
/// it needs a THIOL balance for tx fees before step 5.
pub const PAYMENT_MINTER: &str = "terp1z4zc83jvff6w5d6dap23nl75ah9f5s828na3z8";
pub const PAYMINT_SALT: &str = "permissionless-fractal-v1";
/// paymint_tree(PAYMENT_MINTER, 10000, PAYMINT_SALT), verified on the code-73 wasm (gas-bench wl73).
pub const PAYMINT_ROOT: &str = "bfeda62026afd9f946dba360eb0be86785613db261ecbf6913a175dcfa1d818c";
pub const PAYMINT_PROOF: &str = "59a73b7800d05280d8838a704cc496993b7ca578e7a3b4d016c433a8b869aaf5";
/// Retired code-71 logo collections: no step here. If their mint is still open they keep
/// minting until their own creator pauses / ends them (creator-signed, not part of this plan).
pub const RETIRED_CODE71: [&str; 7] = [
    "terp1jagdzq7ncqtv7zmechkuexem2ak50kjpk3uj5vajucw0l3cqg9rqz7al0a", "terp1tsk4rgs87c6da2pgnypgftg267c2k2x0d3ec0kdxf7pcwdedgdrqf6dc2w",
    "terp1ptewup5u6jgg8x5zrw7fndt5845nynr5efvd7pe5e36y4c8k7rzsrc6l6f", "terp15wlh64paltl9c0kfmuxqfafkdnr89mvevmygxdxs6tg4wd4w5zesmvv7us",
    "terp13pz6xddpj0n8ky6nsyqptvgkm9ulp96xqg3zygwzh9s3w3a2ju5qlh2yxn", "terp1wx5vuf7aduxp69x7xuge9dapxdysdvl33dlwd7chfwtdfvsrph0s2t632a",
    "terp1vp7smemrv846hpsyqrzlnqapmu3sv7hyygf4rrnw3ls26ezk864qupr5zt"];
pub const FRACTAL_CURATOR_TBD: &str = "FRACTAL_CURATOR_TBD";
pub const LOGOS_CURATOR_TBD: &str = "LOGOS_CURATOR_TBD";
/// Curators chosen by the owner (2026-10-28): the Zigner granter for both, for now.
pub const FRACTAL_CURATOR: &str = ZIGNER_GRANTER;
pub const LOGOS_CURATOR: &str = ZIGNER_GRANTER;
pub const SERIES: &str = "interchain-logos";
pub const BATCHES: [&[&str]; 3] = [
    &["penumbra"],
    &["osmosis", "cardano", "bcna", "cosmos", "terp", "bitcoin"],
    &["akash", "bitsong", "dao", "eth", "monero", "zec"],
];

#[derive(Debug, Clone)]
pub struct Roles {
    pub fractal_curator: String,
    pub logos_curator: String,
    pub payment_minter: String,
    pub withdraw: String,
    /// factory owner / series operator (signs SetSeries + CreateSeriesCollection)
    pub operator: String,
    pub salt: String,
}
impl Default for Roles {
    fn default() -> Self {
        Self { fractal_curator: FRACTAL_CURATOR.into(), logos_curator: LOGOS_CURATOR.into(),
            payment_minter: PAYMENT_MINTER.into(), withdraw: FOUNDATION_DAO.into(),
            operator: ZIGNER_GRANTER.into(), salt: PAYMINT_SALT.into() }
    }
}
impl Roles {
    pub fn require_real(&self) -> Result<()> {
        let bad: Vec<_> = [("fractal_curator", &self.fractal_curator), ("logos_curator", &self.logos_curator),
            ("payment_minter", &self.payment_minter), ("withdraw", &self.withdraw), ("operator", &self.operator)]
            .into_iter().filter(|(_, v)| paymint::is_placeholder(v)).map(|(k, v)| format!("{k}={v}")).collect();
        anyhow::ensure!(bad.is_empty(), "BLOCKED: placeholders, refusing to broadcast: {}", bad.join(", "));
        Ok(())
    }
}

/// One approvable step. `contract` = None means MsgInstantiateContract of `code_id`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Step { pub id: String, pub signer: String, pub contract: Option<String>, pub code_id: Option<u64>, pub label: Option<String>, pub admin: Option<String>, pub msg: Value, pub note: String }

pub fn logo_init(dir: &std::path::Path, logo: &str, r: &Roles) -> Result<Value> {
    let mut v: Value = serde_json::from_slice(&std::fs::read(dir.join(logo).join("init.json"))?)?;
    v["creator"] = json!(r.logos_curator);
    v["withdraw_address"] = json!(r.withdraw);
    Ok(v)
}

/// `wl_addr`: the whitelist instance created by step 1 (unknown until it lands).
pub fn plan(r: &Roles, logos_dir: &std::path::Path, wl_addr: &str, seed: u64) -> Result<Vec<Step>> {
    let tree = paymint::paymint_tree(&r.payment_minter, crate::fractal::FRACTAL_SUPPLY as u32, &r.salt);
    let fr = crate::fractal::permissionless_fractal_init_v2(&seed, &crate::fractal::FractalRoles {
        curator: r.fractal_curator.clone(), withdraw: r.withdraw.clone(), paymint_whitelist: Some(wl_addr.into()) })?;
    let mut s = vec![
        Step { id: "1-paymint-whitelist".into(), signer: r.operator.clone(), contract: None, code_id: Some(CODE_WHITELIST_MTREE),
            label: Some("permissionless-fractal-paymint-wl".into()), admin: Some(FOUNDATION_DAO.into()),
            msg: serde_json::to_value(paymint::whitelist_init(&tree, &r.fractal_curator))?,
            note: format!("instantiate existing code 73; root commits to `{}{}`", r.payment_minter, tree.alloc) },
        Step { id: "2-fractal-collection".into(), signer: r.operator.clone(), contract: Some(FACTORY.into()), code_id: None, label: None, admin: None,
            msg: json!({"create_svg_collection": {"instantiate_msg": fr, "label": "permissionless-fractal"}}),
            note: "factory owner only (owned factory); -> code 98, collection wasm admin = creator = fractal curator; whitelist = step-1 address".into() },
        Step { id: "3-set-series".into(), signer: r.operator.clone(), contract: Some(FACTORY.into()), code_id: None, label: None, admin: None,
            msg: json!({"set_series": {"series": SERIES, "config": {"operators": [r.operator]}}}), note: "factory owner".into() },
    ];
    for (bi, b) in BATCHES.iter().enumerate() {
        for l in b.iter() {
            s.push(Step { id: format!("4{}-{l}", ['a', 'b', 'c'][bi]), signer: r.operator.clone(), contract: Some(FACTORY.into()), code_id: None, label: None, admin: None,
                msg: json!({"create_series_collection": {"series": SERIES, "instantiate_msg": logo_init(logos_dir, l, r)?, "label": format!("interchain-logos-{l}")}}),
                note: format!("batch {}", ['A', 'B', 'C'][bi]) });
        }
    }
    s.push(Step { id: "5-paymint-template".into(), signer: r.payment_minter.clone(), contract: Some("<fractal collection>".into()), code_id: None, label: None, admin: None,
        msg: serde_json::to_value(paymint::paymint_msg(&tree, 1))?, note: "per sale, no funds; same tx: transfer_nft to buyer".into() });
    Ok(s)
}
