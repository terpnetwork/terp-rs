//! IBC packet / mint metadata: commitments only.

use serde::{Deserialize, Serialize};

/// What the federation puts on the wire and what `cw721` attributes may store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintPacket {
    pub c_deposit: [u8; 32],
    pub n_deposit: [u8; 32],
    pub c_action: [u8; 32],
    pub bind: [u8; 32],
    /// Cell receipt digest (attested turn). Not a payment txid.
    pub receipt: [u8; 32],
    /// Terp recipient of the NFT (not the foreign payer).
    #[serde(default)]
    pub token_recipient: String,
    /// cw721_svg collection the runtime mints into.
    #[serde(default)]
    pub collection: String,
}

impl MintPacket {
    /// Fail closed if JSON ever grew a payment-source field.
    pub fn leaks_payment_source(&self, json: &str) -> bool {
        let lower = json.to_ascii_lowercase();
        const FORBIDDEN: &[&str] = &[
            "txid",
            "tx_hash",
            "txhash",
            "outpoint",
            "vout",
            "note_commitment",
            "vault_log",
            "rail",
            "zec",
            "ironwood",
            "bitcoin",
            "ethereum",
            "0x",
            "secret-txid",
        ];
        FORBIDDEN.iter().any(|k| lower.contains(k))
    }
}
