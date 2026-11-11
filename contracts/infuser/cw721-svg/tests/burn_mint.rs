//! Burn-then-mint e2e on cw-multi-test, mirroring the permissionless fractal on
//! morocco-1: public mints 0..=17, holder burns 12, the admin migrates to the
//! fixed code with `next_token_id: 18`, minting resumes at 18.
use cosmwasm_std::{Addr, Binary};
use cw_multi_test::{App, ContractWrapper, Executor};
use cw_svg::{VariableDef, VariableKind};
use cw721_svg::{
    ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, SvgCollectionMetadata, SvgExecuteMsgExt,
};

struct W {
    app: App,
    admin: Addr,
    buyer: Addr,
    nft: Addr,
    code: u64,
}

fn world() -> W {
    let mut app = App::default();
    let admin = app.api().addr_make("admin");
    let factory = app.api().addr_make("svg-minter");
    let buyer = app.api().addr_make("buyer");
    let w = ContractWrapper::new_with_empty(
        cw721_svg::entry::execute,
        cw721_svg::entry::instantiate,
        cw721_svg::entry::query,
    )
    .with_migrate(
        |d: cosmwasm_std::DepsMut,
         e: cosmwasm_std::Env,
         m: MigrateMsg,
         _: cosmwasm_std::MigrateInfo| cw721_svg::entry::migrate(d, e, m),
    );
    let code = app.store_code(Box::new(w));
    let v = vec![VariableDef {
        name: "a".into(),
        kind: VariableKind::Options(vec!["x".into(), "y".into()]),
    }];
    let t = "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>${a}</text></svg>";
    let nft = app
        .instantiate_contract(
            code,
            factory,
            &InstantiateMsg {
                name: "Permissionless Fractals".into(),
                symbol: "FRACTAL".into(),
                minter: None,
                creator: Some(admin.to_string()),
                withdraw_address: None,
                collection_info_extension: SvgCollectionMetadata {
                    seed: Binary::from(b"seed".as_slice()),
                    packed: cw721_svg::pack_svg(t, &v),
                    price_tiers: vec![],
                    total: 10_000,
                    mint_start_time: None,
                    mint_end_time: None,
                    whitelist: None,
                    require_auth: false,
                },
            },
            &[],
            "permissionless-fractal",
            Some(admin.to_string()),
        )
        .unwrap();
    W {
        app,
        admin,
        buyer,
        nft,
        code,
    }
}

impl W {
    fn mint(&mut self, amnt: u64) -> Result<Vec<String>, String> {
        let r = self
            .app
            .execute_contract(
                self.buyer.clone(),
                self.nft.clone(),
                &ExecuteMsg::UpdateExtension {
                    msg: SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
                        alloc: 0,
                        amnt,
                        proof_hashes: vec![],
                        ext: None,
                    }),
                },
                &[],
            )
            .map_err(|e| format!("{e:?}"))?;
        Ok(r.events
            .iter()
            .filter(|e| e.ty == "wasm-mint")
            .flat_map(|e| e.attributes.iter().filter(|a| a.key == "token_id"))
            .map(|a| a.value.clone())
            .collect())
    }
    fn burn(&mut self, id: &str) {
        self.app
            .execute_contract(
                self.buyer.clone(),
                self.nft.clone(),
                &ExecuteMsg::Burn {
                    token_id: id.into(),
                },
                &[],
            )
            .unwrap();
    }
    fn count(&self) -> u64 {
        let r: cw721::msg::NumTokensResponse = self
            .app
            .wrap()
            .query_wasm_smart(&self.nft, &QueryMsg::NumTokens {})
            .unwrap();
        r.count
    }
    fn tokens(&self) -> Vec<String> {
        let r: cw721::msg::TokensResponse = self
            .app
            .wrap()
            .query_wasm_smart(
                &self.nft,
                &QueryMsg::AllTokens {
                    start_after: None,
                    limit: Some(100),
                },
            )
            .unwrap();
        r.tokens
    }
    fn migrate(&mut self, next_token_id: Option<u64>) -> Result<(), String> {
        self.app
            .migrate_contract(
                self.admin.clone(),
                self.nft.clone(),
                &MigrateMsg {
                    dregg_ibc_only: None,
                    runtime_minters: None,
                    next_token_id,
                },
                self.code,
            )
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }
    /// Raw counter, as an operator would read it over LCD `raw/svg_next_token_id`.
    fn counter(&self) -> Option<u64> {
        self.app
            .wrap()
            .query_wasm_raw(&self.nft, b"svg_next_token_id".to_vec())
            .unwrap()
            .map(|b| cosmwasm_std::from_json(&b).unwrap())
    }
    /// Code-98 storage had no counter.
    fn forget_counter(&mut self) {
        let nft = self.nft.clone();
        self.app
            .contract_storage_mut(&nft)
            .remove(b"svg_next_token_id");
    }
}

fn fractal() -> W {
    let mut w = world();
    for i in 0..18u64 {
        assert_eq!(w.mint(1).unwrap(), vec![i.to_string()]);
    }
    w.burn("12");
    assert_eq!(w.count(), 17);
    w
}

#[test]
fn fractal_burn_then_mint_after_migrate() {
    let mut w = fractal();
    w.forget_counter();
    assert_eq!(w.counter(), None);
    // only the wasm admin may migrate
    let rando = w.app.api().addr_make("rando");
    assert!(
        w.app
            .migrate_contract(
                rando,
                w.nft.clone(),
                &MigrateMsg {
                    dregg_ibc_only: None,
                    runtime_minters: None,
                    next_token_id: Some(18),
                },
                w.code,
            )
            .is_err()
    );
    w.migrate(Some(18)).unwrap();
    assert_eq!(w.counter(), Some(18));
    assert_eq!(w.mint(1).unwrap(), vec!["18"]);
    assert_eq!(w.mint(1).unwrap(), vec!["19"]);
    assert_eq!(w.mint(3).unwrap(), vec!["20", "21", "22"]);
    assert_eq!(w.count(), 22);
    let toks = w.tokens();
    assert!(!toks.contains(&"12".to_string()));
    assert_eq!(toks.len(), 22);
    // a second burn does not rewind the ids either
    w.burn("22");
    assert_eq!(w.mint(1).unwrap(), vec!["23"]);
    assert_eq!(w.counter(), Some(24));
}

#[test]
fn fractal_migrate_without_a_counter_defaults_to_max_plus_one() {
    let mut w = fractal();
    w.forget_counter();
    w.migrate(None).unwrap();
    assert_eq!(w.counter(), Some(18));
    assert_eq!(w.mint(1).unwrap(), vec!["18"]);
}

#[test]
fn migrate_refuses_to_lower_the_counter() {
    let mut w = fractal();
    assert_eq!(w.counter(), Some(18));
    assert!(w.migrate(Some(17)).is_err());
    w.migrate(None).unwrap();
    assert_eq!(w.counter(), Some(18));
}
