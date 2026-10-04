#[cosmwasm_schema::cw_serde]
pub struct SvgMintCallbackAction {
    /// contract addr of cw721-svg (self-contained minter).
    ///  MUST in same token a given shitstrap instance shits out.
    pub c: String,
    pub msg: MintMsg,
}

/// ADR-36 / CosmosArbitrary mint auth message formats (UTF-8 bytes in `ext.message`):
///
/// **v1 (nonce only):** `{CW721SVG_DST}{nonce_decimal}`
/// no separator between DST and nonce.
///
/// **v2 (content-bound):** `{CW721SVG_DST}{nonce_decimal}|{digest_hex}`
/// where `digest_hex` is 64 lowercase hex chars (32-byte ADR-032 / pledge digest,
/// without `0x`). Binds the signed auth to a specific project / manifesto /
/// multi-dst commitment rollup computed off-chain (see dao-dao-ui
/// `trustlessPledge.ts`).
///
/// Max length is {@link AUTH_MSG_MAX_EXTRA} bytes after the DST prefix.
pub const AUTH_MSG_MAX_EXTRA: usize = 128;

#[cosmwasm_schema::cw_serde]
pub struct MintMsg {
    /// amount of tokens to mint. Must send atleast `t` funds, where t >= amnt * mint-fee
    pub amnt: u64,
    /// Merkle proof hashes for whitelist verification (bypasses mint fees)
    pub proof_hashes: Vec<String>,
    /// Per-address mint allocation encoded in the merkle leaf.
    /// When set, the leaf is hash(sender || allocation) and this value
    /// caps how many tokens the address can mint via whitelist.
    pub alloc: u32,
    /// Required ADR-36 authentication (`CosmosArbitrary`).
    ///
    /// Message bytes must start with {@link crate::CW721SVG_DST} and follow
    /// the v1/v2 formats documented on {@link AUTH_MSG_MAX_EXTRA}.
    /// Signature is one-time: `(message_hex, pubkey_hex)` is consumed in
    /// `MINTER_NONCE` storage so the same auth cannot be replayed.
    pub ext: Option<terp_account::verify_generic::CosmosArbitrary>,
}
