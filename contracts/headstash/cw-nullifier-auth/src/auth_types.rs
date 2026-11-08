//! Sudo request types aligned with Go `x/smart-account/authenticator` JSON tags.
//!
//! Kept local (no osmosis-authenticators dep) so this product scaffold stays lightweight.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, Coin};

#[cw_serde]
pub struct LocalAny {
    pub type_url: String,
    pub value: Binary,
}

#[cw_serde]
pub struct SignModeData {
    pub sign_mode_direct: Binary,
    #[serde(default)]
    pub sign_mode_textual: String,
}

#[cw_serde]
pub struct ExplicitTxData {
    pub chain_id: String,
    pub account_number: u64,
    pub sequence: u64,
    pub timeout_height: u64,
    pub msgs: Vec<LocalAny>,
    #[serde(default)]
    pub memo: String,
}

#[cw_serde]
pub struct SimplifiedSignatureData {
    pub signers: Vec<String>,
    pub signatures: Vec<Binary>,
}

#[cw_serde]
pub struct AuthenticationRequest {
    pub authenticator_id: String,
    pub account: String,
    pub fee_payer: String,
    #[serde(default)]
    pub fee_granter: Option<String>,
    #[serde(default)]
    pub fee: Vec<Coin>,
    pub msg: LocalAny,
    pub msg_index: u64,
    /// Auth data for this authenticator (JSON `NullifierAuthPayload` for this contract).
    pub signature: Binary,
    pub sign_mode_tx_data: SignModeData,
    pub tx_data: ExplicitTxData,
    pub signature_data: SimplifiedSignatureData,
    pub simulate: bool,
    #[serde(default)]
    pub authenticator_params: Option<Binary>,
}

#[cw_serde]
pub struct TrackRequest {
    pub authenticator_id: String,
    pub account: String,
    pub fee_payer: String,
    #[serde(default)]
    pub fee_granter: Option<String>,
    #[serde(default)]
    pub fee: Vec<Coin>,
    pub msg: LocalAny,
    pub msg_index: u64,
    #[serde(default)]
    pub authenticator_params: Option<Binary>,
}

#[cw_serde]
pub struct ConfirmExecutionRequest {
    pub authenticator_id: String,
    pub account: String,
    pub fee_payer: String,
    #[serde(default)]
    pub fee_granter: Option<String>,
    #[serde(default)]
    pub fee: Vec<Coin>,
    pub msg: LocalAny,
    pub msg_index: u64,
    #[serde(default)]
    pub authenticator_params: Option<Binary>,
}

#[cw_serde]
pub struct OnAuthenticatorAddedRequest {
    pub account: String,
    #[serde(default)]
    pub authenticator_params: Option<Binary>,
    pub authenticator_id: String,
}

#[cw_serde]
pub struct OnAuthenticatorRemovedRequest {
    pub account: String,
    #[serde(default)]
    pub authenticator_params: Option<Binary>,
    pub authenticator_id: String,
}
