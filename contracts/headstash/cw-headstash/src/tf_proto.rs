//! Minimal protobuf wire encoding for the osmosis tokenfactory messages
//! (`osmosis.tokenfactory.v1beta1`, as registered by terpd's x/tokenfactory).
//! wasmd unpacks `CosmosMsg::Stargate` values as protobuf `Any`, so these must be
//! proto bytes, never JSON. Hand-rolled (strings and one nested message only) to
//! avoid adding prost to this crate's dependency tree.
//!
//! MsgCreateDenom { sender = 1; subdenom = 2 }
//! MsgMint        { sender = 1; amount: Coin = 2; mint_to_address = 3 }
//! MsgBurn        { sender = 1; amount: Coin = 2; burn_from_address = 3 }
//! Coin           { denom = 1; amount = 2 }   (cosmos.base.v1beta1.Coin, amount as string)
//! MsgCreateDenomResponse { new_token_denom = 1 }

use cosmwasm_std::{StdError, StdResult};

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Length-delimited field (wire type 2). Empty values are omitted (proto3 default).
fn put_bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    if b.is_empty() {
        return;
    }
    put_varint(out, ((field as u64) << 3) | 2);
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

pub fn coin(denom: &str, amount: &str) -> Vec<u8> {
    let mut o = Vec::new();
    put_bytes(&mut o, 1, denom.as_bytes());
    put_bytes(&mut o, 2, amount.as_bytes());
    o
}

pub fn msg_create_denom(sender: &str, subdenom: &str) -> Vec<u8> {
    let mut o = Vec::new();
    put_bytes(&mut o, 1, sender.as_bytes());
    put_bytes(&mut o, 2, subdenom.as_bytes());
    o
}

/// MsgMint and MsgBurn share a layout: sender = 1, amount = 2, address = 3.
pub fn msg_coin_addr(sender: &str, denom: &str, amount: &str, addr: &str) -> Vec<u8> {
    let mut o = Vec::new();
    put_bytes(&mut o, 1, sender.as_bytes());
    // amount is never empty here (Uint128 string), so the Coin is always encoded.
    let c = coin(denom, amount);
    put_varint(&mut o, (2u64 << 3) | 2);
    put_varint(&mut o, c.len() as u64);
    o.extend_from_slice(&c);
    put_bytes(&mut o, 3, addr.as_bytes());
    o
}

fn get_varint(b: &[u8], i: &mut usize) -> StdResult<u64> {
    let mut v: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*i).ok_or_else(|| StdError::msg("proto: truncated varint"))?;
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return Ok(v);
        }
    }
    Err(StdError::msg("proto: varint overflow"))
}

/// Decode a message into (field, wire_type, payload) for length-delimited fields;
/// varint fields are skipped, other wire types are rejected.
pub fn fields(b: &[u8]) -> StdResult<Vec<(u32, &[u8])>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let key = get_varint(b, &mut i)?;
        let (field, wt) = ((key >> 3) as u32, key & 7);
        match wt {
            0 => {
                get_varint(b, &mut i)?;
            }
            2 => {
                let len = get_varint(b, &mut i)? as usize;
                let end = i.checked_add(len).filter(|e| *e <= b.len())
                    .ok_or_else(|| StdError::msg("proto: truncated field"))?;
                out.push((field, &b[i..end]));
                i = end;
            }
            _ => return Err(StdError::msg("proto: unsupported wire type")),
        }
    }
    Ok(out)
}

/// String field `field` of message `b` ("" if absent, last occurrence wins).
pub fn string_field(b: &[u8], field: u32) -> StdResult<String> {
    let mut s = String::new();
    for (f, v) in fields(b)? {
        if f == field {
            s = String::from_utf8(v.to_vec()).map_err(|_| StdError::msg("proto: invalid utf8"))?;
        }
    }
    Ok(s)
}

/// Nested message field `field` of `b` (empty if absent).
pub fn message_field(b: &[u8], field: u32) -> StdResult<Vec<u8>> {
    let mut m = Vec::new();
    for (f, v) in fields(b)? {
        if f == field {
            m = v.to_vec();
        }
    }
    Ok(m)
}

/// MsgCreateDenomResponse.new_token_denom
pub fn create_denom_response_denom(b: &[u8]) -> StdResult<String> {
    let d = string_field(b, 1)?;
    if d.is_empty() {
        return Err(StdError::msg("MsgCreateDenomResponse: empty new_token_denom"));
    }
    Ok(d)
}
