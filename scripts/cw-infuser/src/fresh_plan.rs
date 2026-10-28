//! Fresh SVG collections on the ALREADY-DEPLOYED codes only (no uploads, no migrations):
//! cw721_svg code 98, factory code 99 (instance below), whitelist-mtree code 73.
//! Builds every message as data; nothing here can sign or broadcast.
use crate::paymint::{self, PAYMENT_MINTER_TBD};
use anyhow::Result;
use serde_json::{Value, json};

pub const CODE_CW721_SVG: u64 = 98;
pub const CODE_FACTORY: u64 = 99;
pub const CODE_WHITELIST_MTREE: u64 = 73;
pub const FACTORY: &str = "terp1wg5z9rcwpw5kvrl4g78ees2l9ejptrhrx4t2k56nq6t27uly87zssujm80";
pub const FOUNDATION_DAO: &str = "terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w";
pub const ZIGNER_GRANTER: &str = "terp1fxccvvhhy43tvet2ah7jqwq4cwl9k3dxmww052";
pub const FRACTAL_CURATOR_TBD: &str = "FRACTAL_CURATOR_TBD";
pub const LOGOS_CURATOR_TBD: &str = "LOGOS_CURATOR_TBD";
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
        Self { fractal_curator: FRACTAL_CURATOR_TBD.into(), logos_curator: LOGOS_CURATOR_TBD.into(),
            payment_minter: PAYMENT_MINTER_TBD.into(), withdraw: FOUNDATION_DAO.into(),
            operator: ZIGNER_GRANTER.into(), salt: "permissionless-fractal-v1".into() }
    }
}
impl Roles {
    pub fn require_real(&self) -> Result<()> {
        paymint::require_real(&[("fractal_curator", &self.fractal_curator), ("logos_curator", &self.logos_curator),
            ("payment_minter", &self.payment_minter), ("withdraw", &self.withdraw), ("operator", &self.operator)])
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
