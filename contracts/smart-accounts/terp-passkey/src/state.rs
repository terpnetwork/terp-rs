use cosmwasm_std::Binary;
use cw_storage_plus::Item;

/// P-256 credential checked by [`crate::webauthn::verify_passkey`].
#[cosmwasm_schema::cw_serde]
pub struct PasskeyRegistration {
    /// `clientDataJSON.origin`, including the scheme.
    pub origin: String,
    /// Relying party id. `authenticatorData` must start with SHA-256 of this string.
    pub rp_id: String,
    /// Credential id bytes.
    pub credential_id: Binary,
    /// SEC1 P-256 public key, uncompressed (65) or compressed (33).
    pub public_key: Binary,
}

pub const REGISTRATION: Item<PasskeyRegistration> = Item::new("reg");
