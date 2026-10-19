use cosmwasm_std::{Coin, Deps, Uint256};
use prost::Message;
use std::str::FromStr;

use crate::error::{QueryError, QueryResult};
use crate::pagination::{PageRequest, PageResponse, paginate_grpc_query};

// ====================== Conversions ======================

fn coin_from_proto(c: CoinProto) -> QueryResult<Coin> {
    Ok(Coin {
        denom: c.denom,
        amount: Uint256::from_str(&c.amount)
            .map_err(|e| QueryError::Std(cosmwasm_std::StdError::msg(e.to_string())))?,
    })
}

// ====================== Public API ======================

/// Query all balances for an address, paginating through all results.
///
/// Drop-in replacement for the removed `BankQuery::AllBalances` in cosmwasm-std v3.
///
/// ```rust,ignore
/// let balances = query_all_balances(deps.as_ref(), &env.contract.address)?;
/// ```
pub fn query_all_balances(deps: Deps, address: impl Into<String>) -> QueryResult<Vec<Coin>> {
    let address = address.into();

    paginate_grpc_query(
        deps,
        "/cosmos.bank.v1beta1.Query/AllBalances",
        |page_req| QueryAllBalancesRequest {
            address: address.clone(),
            pagination: Some(page_req),
        },
        |resp: QueryAllBalancesResponse| {
            let items: QueryResult<Vec<Coin>> =
                resp.balances.into_iter().map(coin_from_proto).collect();
            let next_key = resp.pagination.and_then(|p| {
                if p.next_key.is_empty() {
                    None
                } else {
                    Some(p.next_key)
                }
            });
            Ok((items?, next_key))
        },
        None,
    )
}

/// Query all balances with a maximum item limit.
pub fn query_all_balances_limited(
    deps: Deps,
    address: impl Into<String>,
    limit: u32,
) -> QueryResult<Vec<Coin>> {
    let address = address.into();

    paginate_grpc_query(
        deps,
        "/cosmos.bank.v1beta1.Query/AllBalances",
        |page_req| QueryAllBalancesRequest {
            address: address.clone(),
            pagination: Some(page_req),
        },
        |resp: QueryAllBalancesResponse| {
            let items: QueryResult<Vec<Coin>> =
                resp.balances.into_iter().map(coin_from_proto).collect();
            let next_key = resp.pagination.and_then(|p| {
                if p.next_key.is_empty() {
                    None
                } else {
                    Some(p.next_key)
                }
            });
            Ok((items?, next_key))
        },
        Some(limit),
    )
}

/// Query a single denom balance for an address.
///
/// Drop-in replacement for `BankQuery::Balance` (still exists in v3 but included
/// here for completeness when using the gRPC path directly).
pub fn query_balance(
    deps: Deps,
    address: impl Into<String>,
    denom: impl Into<String>,
) -> QueryResult<Coin> {
    let req = QueryBalanceRequest {
        address: address.into(),
        denom: denom.into(),
    };

    let mut buf = Vec::new();
    req.encode(&mut buf)?;

    let query = cosmwasm_std::QueryRequest::Grpc(cosmwasm_std::GrpcQuery {
        path: "/cosmos.bank.v1beta1.Query/Balance".to_string(),
        data: cosmwasm_std::Binary::from(buf),
    });

    let raw_resp: cosmwasm_std::Binary = deps.querier.query(&query)?;
    let resp = QueryBalanceResponse::decode(raw_resp.as_ref())?;

    match resp.balance {
        Some(c) => coin_from_proto(c),
        None => Ok(Coin {
            denom: String::new(),
            amount: Uint256::zero(),
        }),
    }
}
