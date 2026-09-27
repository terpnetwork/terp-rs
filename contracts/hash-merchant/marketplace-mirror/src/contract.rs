use cosmwasm_std::{
    entry_point, to_json_binary, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo,
    Order as IterOrder, Response, StdResult, Uint128,
};

use crate::error::ContractError;
use crate::msg::*;
use crate::state::*;

// ---------------------------------------------------------------------------
// Instantiate
// ---------------------------------------------------------------------------

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if msg.min_buffer_blocks < MIN_BUFFER_BLOCKS_LO
        || msg.min_buffer_blocks > MIN_BUFFER_BLOCKS_HI
    {
        return Err(ContractError::InvalidBuffer {
            got: msg.min_buffer_blocks,
            min: MIN_BUFFER_BLOCKS_LO,
            max: MIN_BUFFER_BLOCKS_HI,
        });
    }
    if msg.accepted_denom.trim().is_empty() {
        return Err(ContractError::EmptyDenom {});
    }
    if let Some(bps) = msg.protocol_fee_bps {
        if bps > 10_000 {
            return Err(ContractError::InvalidFeeBps { bps });
        }
    }

    let admin = deps.api.addr_validate(&msg.admin)?;
    CONFIG.save(
        deps.storage,
        &Config {
            admin,
            min_buffer_blocks: msg.min_buffer_blocks,
            accepted_denom: msg.accepted_denom.clone(),
            protocol_fee_bps: msg.protocol_fee_bps,
            auto_execute_ingress: msg.auto_execute_ingress,
        },
    )?;
    NEXT_ORDER_ID.save(deps.storage, &1u64)?;

    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("path", "marketplace-mirror")
        .add_attribute("min_buffer_blocks", msg.min_buffer_blocks.to_string())
        .add_attribute("accepted_denom", msg.accepted_denom)
        .add_attribute(
            "auto_execute_ingress",
            msg.auto_execute_ingress.to_string(),
        ))
}

// ---------------------------------------------------------------------------
// Sudo — hashmerchant module callback
// ---------------------------------------------------------------------------

#[entry_point]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::HashMerchant {
            chain_uid,
            algo,
            root,
            height,
            attestation_count,
            block_time,
        } => {
            // Module sends root as base64 (Go []byte → JSON). Store as hex.
            // Do NOT mint inventory from oracle fields — local SKU stock is authoritative.
            let root_hex = root_to_hex(&root);
            let entry = RootEntry {
                chain_uid: chain_uid.clone(),
                algo: algo.clone(),
                root: root_hex.clone(),
                height,
                attestation_count,
                block_time,
            };
            ROOTS.save(deps.storage, (&chain_uid, &algo), &entry)?;

            Ok(Response::new()
                .add_attribute("action", "store_root")
                .add_attribute("chain_uid", chain_uid)
                .add_attribute("algo", algo)
                .add_attribute("root", root_hex)
                .add_attribute("height", height.to_string())
                .add_attribute("attestation_count", attestation_count.to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::ListProduct {
            sku,
            stock,
            unit_price,
            content_cid,
            d_tag,
        } => execute_list_product(deps, info, sku, stock, unit_price, content_cid, d_tag),
        ExecuteMsg::UpdateStock { sku, stock } => execute_update_stock(deps, info, sku, stock),
        ExecuteMsg::Purchase { sku, qty } => execute_purchase(deps, env, info, sku, qty),
        ExecuteMsg::Settle { order_id } => execute_settle(deps, env, order_id),
        ExecuteMsg::Cancel { order_id } => execute_cancel(deps, info, order_id),
        ExecuteMsg::SetAutoExecuteIngress { enabled } => {
            execute_set_auto_execute_ingress(deps, info, enabled)
        }
    }
}

fn execute_list_product(
    deps: DepsMut,
    info: MessageInfo,
    sku: String,
    stock: u64,
    unit_price: Uint128,
    content_cid: Option<String>,
    d_tag: Option<String>,
) -> Result<Response, ContractError> {
    if sku.trim().is_empty() {
        return Err(ContractError::EmptySku {});
    }
    if PRODUCTS.has(deps.storage, &sku) {
        return Err(ContractError::ProductExists { sku });
    }

    let product = Product {
        sku: sku.clone(),
        seller: info.sender.clone(),
        stock,
        unit_price,
        content_cid: content_cid.clone(),
        d_tag: d_tag.clone(),
    };
    PRODUCTS.save(deps.storage, &sku, &product)?;
    RESERVED.save(deps.storage, &sku, &0u64)?;

    Ok(Response::new()
        .add_attribute("action", "list_product")
        .add_attribute("sku", sku)
        .add_attribute("seller", info.sender)
        .add_attribute("stock", stock.to_string())
        .add_attribute("unit_price", unit_price)
        .add_attribute(
            "content_cid",
            content_cid.unwrap_or_default(),
        )
        .add_attribute("d_tag", d_tag.unwrap_or_default()))
}

fn execute_update_stock(
    deps: DepsMut,
    info: MessageInfo,
    sku: String,
    stock: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut product = PRODUCTS
        .may_load(deps.storage, &sku)?
        .ok_or_else(|| ContractError::ProductNotFound { sku: sku.clone() })?;

    if info.sender != product.seller && info.sender != cfg.admin {
        return Err(ContractError::Unauthorized {});
    }

    let reserved = RESERVED.may_load(deps.storage, &sku)?.unwrap_or(0);
    if stock < reserved {
        return Err(ContractError::StockBelowReserved {
            sku,
            stock,
            reserved,
        });
    }

    product.stock = stock;
    PRODUCTS.save(deps.storage, &sku, &product)?;

    Ok(Response::new()
        .add_attribute("action", "update_stock")
        .add_attribute("sku", sku)
        .add_attribute("stock", stock.to_string())
        .add_attribute("reserved", reserved.to_string())
        .add_attribute("available", (stock - reserved).to_string()))
}

fn execute_purchase(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sku: String,
    qty: u64,
) -> Result<Response, ContractError> {
    if qty == 0 {
        return Err(ContractError::ZeroQty {});
    }

    let cfg = CONFIG.load(deps.storage)?;
    let product = PRODUCTS
        .may_load(deps.storage, &sku)?
        .ok_or_else(|| ContractError::ProductNotFound { sku: sku.clone() })?;

    let reserved = RESERVED.may_load(deps.storage, &sku)?.unwrap_or(0);
    let available = product.stock.saturating_sub(reserved);
    if qty > available {
        return Err(ContractError::InsufficientStock {
            sku,
            need: qty,
            available,
        });
    }

    let need = product
        .unit_price
        .checked_mul(Uint128::from(qty))?;
    let paid = must_pay(&info, &cfg.accepted_denom)?;
    if paid < need {
        return Err(ContractError::InsufficientFunds {
            need: need.to_string(),
            got: paid.to_string(),
            denom: cfg.accepted_denom,
        });
    }

    let order_id = NEXT_ORDER_ID.load(deps.storage)?;
    NEXT_ORDER_ID.save(deps.storage, &(order_id + 1))?;

    let reserve_height = env.block.height;
    let settle_after = reserve_height + cfg.min_buffer_blocks;

    let order = PurchaseOrder {
        id: order_id,
        buyer: info.sender.clone(),
        seller: product.seller.clone(),
        sku: sku.clone(),
        qty,
        amount: paid,
        reserve_height,
        settle_after,
        status: OrderStatus::Reserved,
    };
    ORDERS.save(deps.storage, order_id, &order)?;
    RESERVED.save(deps.storage, &sku, &(reserved + qty))?;

    Ok(Response::new()
        .add_attribute("action", "purchase")
        .add_attribute("order_id", order_id.to_string())
        .add_attribute("buyer", info.sender)
        .add_attribute("sku", sku)
        .add_attribute("qty", qty.to_string())
        .add_attribute("amount", paid)
        .add_attribute("reserve_height", reserve_height.to_string())
        .add_attribute("settle_after", settle_after.to_string())
        .add_attribute("status", OrderStatus::Reserved.as_str()))
}

fn execute_settle(
    deps: DepsMut,
    env: Env,
    order_id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut order = ORDERS
        .may_load(deps.storage, order_id)?
        .ok_or(ContractError::OrderNotFound { id: order_id })?;

    if order.status != OrderStatus::Reserved {
        return Err(ContractError::BadOrderStatus {
            id: order_id,
            status: order.status.as_str().into(),
            expected: OrderStatus::Reserved.as_str().into(),
        });
    }
    if env.block.height < order.settle_after {
        return Err(ContractError::BufferNotElapsed {
            height: env.block.height,
            settle_after: order.settle_after,
        });
    }

    let mut product = PRODUCTS
        .may_load(deps.storage, &order.sku)?
        .ok_or_else(|| ContractError::ProductNotFound {
            sku: order.sku.clone(),
        })?;

    // Decrement stock (reserved units convert to sold).
    if product.stock < order.qty {
        return Err(ContractError::InsufficientStock {
            sku: order.sku.clone(),
            need: order.qty,
            available: product.stock,
        });
    }
    product.stock -= order.qty;
    PRODUCTS.save(deps.storage, &order.sku, &product)?;

    let reserved = RESERVED.may_load(deps.storage, &order.sku)?.unwrap_or(0);
    RESERVED.save(
        deps.storage,
        &order.sku,
        &reserved.saturating_sub(order.qty),
    )?;

    order.status = OrderStatus::Settled;
    ORDERS.save(deps.storage, order_id, &order)?;

    // Split protocol fee (if any) then pay seller.
    let (seller_amt, fee_amt) = split_fee(order.amount, cfg.protocol_fee_bps);
    let mut msgs: Vec<CosmosMsg> = Vec::new();

    if !seller_amt.is_zero() {
        msgs.push(CosmosMsg::Bank(BankMsg::Send {
            to_address: order.seller.to_string(),
            amount: vec![Coin {
                denom: cfg.accepted_denom.clone(),
                amount: seller_amt,
            }],
        }));
    }
    if !fee_amt.is_zero() {
        msgs.push(CosmosMsg::Bank(BankMsg::Send {
            to_address: cfg.admin.to_string(),
            amount: vec![Coin {
                denom: cfg.accepted_denom.clone(),
                amount: fee_amt,
            }],
        }));
    }

    Ok(Response::new()
        .add_messages(msgs)
        .add_attribute("action", "settle")
        .add_attribute("order_id", order_id.to_string())
        .add_attribute("sku", order.sku)
        .add_attribute("qty", order.qty.to_string())
        .add_attribute("seller_amount", seller_amt)
        .add_attribute("fee_amount", fee_amt)
        .add_attribute("status", OrderStatus::Settled.as_str()))
}

fn execute_cancel(
    deps: DepsMut,
    info: MessageInfo,
    order_id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    let mut order = ORDERS
        .may_load(deps.storage, order_id)?
        .ok_or(ContractError::OrderNotFound { id: order_id })?;

    if order.status != OrderStatus::Reserved {
        return Err(ContractError::BadOrderStatus {
            id: order_id,
            status: order.status.as_str().into(),
            expected: OrderStatus::Reserved.as_str().into(),
        });
    }

    // Mini-escrow integrity: only buyer or admin may cancel during Reserved buffer.
    // Seller cannot grief-cancel a paid reservation.
    if info.sender != order.buyer && info.sender != cfg.admin {
        return Err(ContractError::Unauthorized {});
    }

    let reserved = RESERVED.may_load(deps.storage, &order.sku)?.unwrap_or(0);
    RESERVED.save(
        deps.storage,
        &order.sku,
        &reserved.saturating_sub(order.qty),
    )?;

    order.status = OrderStatus::Cancelled;
    ORDERS.save(deps.storage, order_id, &order)?;

    let refund = CosmosMsg::Bank(BankMsg::Send {
        to_address: order.buyer.to_string(),
        amount: vec![Coin {
            denom: cfg.accepted_denom,
            amount: order.amount,
        }],
    });

    Ok(Response::new()
        .add_message(refund)
        .add_attribute("action", "cancel")
        .add_attribute("order_id", order_id.to_string())
        .add_attribute("buyer", order.buyer)
        .add_attribute("sku", order.sku)
        .add_attribute("qty", order.qty.to_string())
        .add_attribute("status", OrderStatus::Cancelled.as_str()))
}

fn execute_set_auto_execute_ingress(
    deps: DepsMut,
    info: MessageInfo,
    enabled: bool,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized {});
    }
    cfg.auto_execute_ingress = enabled;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "set_auto_execute_ingress")
        .add_attribute("auto_execute_ingress", enabled.to_string()))
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let cfg = CONFIG.load(deps.storage)?;
            let next = NEXT_ORDER_ID.load(deps.storage)?;
            to_json_binary(&ConfigResponse {
                admin: cfg.admin,
                min_buffer_blocks: cfg.min_buffer_blocks,
                accepted_denom: cfg.accepted_denom,
                protocol_fee_bps: cfg.protocol_fee_bps,
                next_order_id: next,
                auto_execute_ingress: cfg.auto_execute_ingress,
            })
        }
        QueryMsg::Product { sku } => {
            let product = PRODUCTS.load(deps.storage, &sku)?;
            let reserved = RESERVED.may_load(deps.storage, &sku)?.unwrap_or(0);
            let available = product.stock.saturating_sub(reserved);
            to_json_binary(&ProductResponse {
                product,
                reserved,
                available,
            })
        }
        QueryMsg::Order { id } => {
            let order = ORDERS.load(deps.storage, id)?;
            to_json_binary(&OrderResponse { order })
        }
        QueryMsg::ListOrders { start_after, limit } => {
            let limit = limit.unwrap_or(30).min(100) as usize;
            let start = start_after.map(cw_storage_plus::Bound::exclusive);
            let orders: Vec<PurchaseOrder> = ORDERS
                .range(deps.storage, start, None, IterOrder::Ascending)
                .take(limit)
                .map(|r| r.map(|(_, o)| o))
                .collect::<StdResult<Vec<_>>>()?;
            to_json_binary(&OrdersResponse { orders })
        }
        QueryMsg::ReservedQty { sku } => {
            let reserved = RESERVED.may_load(deps.storage, &sku)?.unwrap_or(0);
            to_json_binary(&ReservedQtyResponse { sku, reserved })
        }
        QueryMsg::GetRoot { chain_uid, algo } => {
            let entry = ROOTS.load(deps.storage, (&chain_uid, &algo))?;
            to_json_binary(&RootResponse::from(entry))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode base64 root → hex; if not base64, passthrough (already hex / opaque).
fn root_to_hex(input: &str) -> String {
    match Binary::from_base64(input) {
        Ok(bytes) => bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>(),
        Err(_) => input.to_string(),
    }
}

fn must_pay(info: &MessageInfo, denom: &str) -> Result<Uint128, ContractError> {
    let mut total = Uint128::zero();
    for c in &info.funds {
        if c.denom != denom {
            return Err(ContractError::WrongDenom {
                expected: denom.into(),
            });
        }
        total = total.checked_add(c.amount)?;
    }
    Ok(total)
}

fn split_fee(amount: Uint128, fee_bps: Option<u64>) -> (Uint128, Uint128) {
    let Some(bps) = fee_bps else {
        return (amount, Uint128::zero());
    };
    if bps == 0 {
        return (amount, Uint128::zero());
    }
    let fee = amount.multiply_ratio(bps, 10_000u128);
    let seller = amount.saturating_sub(fee);
    (seller, fee)
}
