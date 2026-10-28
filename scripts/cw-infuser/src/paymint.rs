//! Fee-free minting for the off-chain payment minter on the DEPLOYED code 98 (no code change).
//!
//! cw721-svg `execute_mint` skips native payment when the sender proves membership in the
//! collection's merkle whitelist (`is_whitelisted || is_dregg_runtime`, contract.rs:300).
//! The leaf is `blake3("{recipient}{alloc}")`; recipient = tx sender, so only the payment
//! minter can use its leaf. A proof must be non-empty (contract.rs:288), so the tree has
//! a second, unspendable salt leaf. The payment minter mints to itself and transfers the
//! token to the buyer in the same tx (cw721 `TransferNft`).
use anyhow::{Result, ensure};

/// Placeholder; scripts refuse to broadcast while any role still holds a `*_TBD` value.
pub const PAYMENT_MINTER_TBD: &str = "PAYMENT_MINTER_TBD";

pub fn is_placeholder(s: &str) -> bool {
    s.ends_with("_TBD") || s.is_empty()
}

/// Refuse to build anything broadcastable from placeholder roles.
pub fn require_real(roles: &[(&str, &str)]) -> Result<()> {
    for (k, v) in roles {
        ensure!(!is_placeholder(v), "{k} is a placeholder ({v}); refusing to broadcast");
    }
    Ok(())
}

pub struct PayMinterTree {
    pub root: String,
    /// Proof the payment minter sends in `MintMsg.proof_hashes`.
    pub proof: Vec<String>,
    pub alloc: u32,
}

fn pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&x);
    buf[32..].copy_from_slice(&y);
    *blake3::hash(&buf).as_bytes()
}

/// Two-leaf tree: the payment minter with `alloc` (use the collection total) plus a salt
/// leaf that no address can match.
pub fn paymint_tree(payment_minter: &str, alloc: u32, salt: &str) -> PayMinterTree {
    let leaf = *blake3::hash(format!("{payment_minter}{alloc}").as_bytes()).as_bytes();
    let salt_leaf = *blake3::hash(format!("paymint-salt:{salt}").as_bytes()).as_bytes();
    PayMinterTree {
        root: hex::encode(pair(leaf, salt_leaf)),
        proof: vec![hex::encode(salt_leaf)],
        alloc,
    }
}

pub fn whitelist_init(tree: &PayMinterTree, admin: &str) -> whitelist_mtree::msg::InstantiateMsg {
    whitelist_mtree::msg::InstantiateMsg {
        merkle_root: tree.root.clone(),
        merkle_tree_uri: None,
        admins: vec![admin.to_string()],
        admins_mutable: true,
    }
}

/// `MintMsg` the payment minter sends (no funds).
pub fn paymint_msg(tree: &PayMinterTree, amnt: u64) -> cw721_svg::ExecuteMsg {
    cw721_svg::ExecuteMsg::UpdateExtension {
        msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
            alloc: tree.alloc,
            amnt,
            proof_hashes: tree.proof.clone(),
            ext: None,
        }),
    }
}
