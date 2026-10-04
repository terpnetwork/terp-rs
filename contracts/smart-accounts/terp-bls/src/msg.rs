use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;
use terp_auth::AuthSudoMsg;

/// Which BLS12-381 subgroup holds the public key.
///
/// Both variants use the proof-of-possession ciphersuite (basic scheme).
/// The signed message is `sign_mode_direct`.
#[cw_serde]
pub enum BlsVariant {
    /// Public key on G1 (48 bytes), signature on G2 (96 bytes).
    /// Hash-to-G2 with `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_`.
    #[serde(rename = "min_pk")]
    MinPk,
    /// Public key on G2 (96 bytes), signature on G1 (48 bytes).
    /// Hash-to-G1 with `BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_`.
    #[serde(rename = "min_sig")]
    MinSig,
}

impl BlsVariant {
    pub fn pubkey_len(&self) -> usize {
        match self {
            Self::MinPk => 48,
            Self::MinSig => 96,
        }
    }

    pub fn signature_len(&self) -> usize {
        match self {
            Self::MinPk => 96,
            Self::MinSig => 48,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MinPk => "min_pk",
            Self::MinSig => "min_sig",
        }
    }
}

#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,
    /// Compressed BLS12-381 public keys. Length is fixed by [`BlsVariant`].
    pub operator_pubkeys: Vec<Binary>,
    /// Distinct valid signatures required. `1..=operator_pubkeys.len()`, at most 10 keys.
    pub threshold: u32,
    pub variant: BlsVariant,
}

#[cw_serde]
pub enum ExecuteMsg {
    UpdateOwnership(cw_ownable::Action),
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(BlsConfigResponse)]
    Config {},
}

#[cw_serde]
pub struct BlsConfigResponse {
    pub variant: BlsVariant,
    pub threshold: u32,
    pub operator_pubkeys: Vec<Binary>,
}

/// JSON body of `AuthenticationRequest.signature`.
///
/// `key_indices[i]` selects `operator_pubkeys` for `signature_data.signatures[i]`.
#[cw_serde]
pub struct BlsAuthPayload {
    pub key_indices: Vec<u32>,
}

pub type SudoMsg = AuthSudoMsg;
