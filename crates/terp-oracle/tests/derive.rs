//! `#[oracle_query]` expansion.

use cosmwasm_schema::{QueryResponses, cw_serde};
use cosmwasm_std::{Binary, Coin, to_json_string};
use terp_oracle::{OracleQueryMsg, PriceAsset, oracle_query};

#[oracle_query]
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(String)]
    Config {},
    #[returns(u32)]
    Count { id: u64 },
}

#[test]
fn appends_variants_with_wire_names() {
    let cases = [
        (QueryMsg::OraclePricing {}, r#"{"oracle_pricing":{}}"#),
        (
            QueryMsg::OraclePrice {
                asset: PriceAsset::native("uatom"),
                base_denom: "uterp".into(),
            },
            r#"{"oracle_price":{"asset":{"native":"uatom"},"base_denom":"uterp"}}"#,
        ),
        (
            QueryMsg::OracleAggregate { pairs: vec![] },
            r#"{"oracle_aggregate":{"pairs":[]}}"#,
        ),
        (
            QueryMsg::OracleQuote {
                base: Coin::new(1u128, "uterp"),
                asset: PriceAsset::cw20("c"),
            },
            r#"{"oracle_quote":{"base":{"denom":"uterp","amount":"1"},"asset":{"cw20":"c"}}}"#,
        ),
        (
            QueryMsg::OracleRaw {
                oracle: "o".into(),
                msg: Binary::from(b"{}"),
            },
            r#"{"oracle_raw":{"oracle":"o","msg":"e30="}}"#,
        ),
    ];
    for (m, want) in cases {
        assert_eq!(to_json_string(&m).unwrap(), want);
    }
}

#[test]
fn same_json_as_standalone_enum() {
    let a = to_json_string(&QueryMsg::OracleAggregate { pairs: vec![] }).unwrap();
    let b = to_json_string(&OracleQueryMsg::OracleAggregate { pairs: vec![] }).unwrap();
    assert_eq!(a, b);
}

#[test]
fn into_oracle_query_splits() {
    assert_eq!(
        QueryMsg::OraclePricing {}.into_oracle_query().unwrap(),
        OracleQueryMsg::OraclePricing {}
    );
    assert_eq!(
        QueryMsg::Count { id: 3 }.into_oracle_query().unwrap_err(),
        QueryMsg::Count { id: 3 }
    );
    assert_eq!(
        QueryMsg::Config {}.into_oracle_query().unwrap_err(),
        QueryMsg::Config {}
    );
}

#[test]
fn schema_lists_oracle_responses() {
    let r = QueryMsg::response_schemas();
    for k in [
        "config",
        "count",
        "oracle_pricing",
        "oracle_price",
        "oracle_aggregate",
        "oracle_quote",
        "oracle_raw",
    ] {
        assert!(r.contains_key(k), "{k}");
    }
    assert!(serde_json_check(&r["oracle_price"]).contains("PriceResponse"));
}

fn serde_json_check(s: &impl serde::Serialize) -> String {
    cosmwasm_std::to_json_string(s).unwrap()
}
