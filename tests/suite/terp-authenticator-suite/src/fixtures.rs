//! Shared Mock request builders for AuthSudoMsg variants.
//!
//! Types come from existing [`terp_auth`] (billboards package). This module only
//! builds dummy values — it does not redefine request structs.

use cosmwasm_std::{Addr, Binary, Coin};
use terp_auth::{
    Any, AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, SignModeTxData, SignatureData, TrackRequest, TxData,
};

/// Default mock account used across suite tests.
pub fn mock_account() -> Addr {
    Addr::unchecked("terp1account000000000000000000000000000")
}

/// Secondary account (fee payer / granter scenarios).
pub fn mock_fee_payer() -> Addr {
    Addr::unchecked("terp1feepayer00000000000000000000000000")
}

pub fn dummy_any() -> Any {
    Any {
        type_url: "/cosmos.bank.v1beta1.MsgSend".to_string(),
        value: Binary::from(b"\x00"),
    }
}

pub fn dummy_sign_mode_tx_data() -> SignModeTxData {
    SignModeTxData {
        sign_mode_direct: Binary::from(b"sign-mode-direct"),
        sign_mode_textual: None,
    }
}

pub fn dummy_tx_data() -> TxData {
    TxData {
        chain_id: "terp-test-1".to_string(),
        account_number: 1,
        sequence: 0,
        timeout_height: 0,
        msgs: vec![dummy_any()],
        memo: String::new(),
    }
}

pub fn dummy_signature_data(account: &Addr) -> SignatureData {
    SignatureData {
        signers: vec![account.clone()],
        signatures: vec![Binary::from(b"sig")],
    }
}

pub fn dummy_on_auth_added(
    account: Addr,
    authenticator_id: impl Into<String>,
    authenticator_params: Option<Binary>,
) -> OnAuthenticatorAddedRequest {
    OnAuthenticatorAddedRequest {
        account,
        authenticator_id: authenticator_id.into(),
        authenticator_params,
    }
}

pub fn dummy_on_auth_removed(
    account: Addr,
    authenticator_id: impl Into<String>,
    authenticator_params: Option<Binary>,
) -> OnAuthenticatorRemovedRequest {
    OnAuthenticatorRemovedRequest {
        account,
        authenticator_id: authenticator_id.into(),
        authenticator_params,
    }
}

pub fn dummy_authentication_request(
    account: Addr,
    authenticator_id: impl Into<String>,
    signature: Binary,
    authenticator_params: Option<Binary>,
) -> AuthenticationRequest {
    let fee_payer = mock_fee_payer();
    AuthenticationRequest {
        authenticator_id: authenticator_id.into(),
        account: account.clone(),
        fee_payer,
        fee_granter: None,
        fee: vec![Coin::new(1000u128, "uterp")],
        msg: dummy_any(),
        msg_index: 0,
        signature,
        sign_mode_tx_data: dummy_sign_mode_tx_data(),
        tx_data: dummy_tx_data(),
        signature_data: dummy_signature_data(&account),
        simulate: false,
        authenticator_params,
    }
}

pub fn dummy_track_request(
    account: Addr,
    authenticator_id: impl Into<String>,
    authenticator_params: Option<Binary>,
) -> TrackRequest {
    TrackRequest {
        authenticator_id: authenticator_id.into(),
        account,
        fee_payer: mock_fee_payer(),
        fee_granter: None,
        fee: vec![Coin::new(1000u128, "uterp")],
        msg: dummy_any(),
        msg_index: 0,
        authenticator_params,
    }
}

pub fn dummy_confirm_execution_request(
    account: Addr,
    authenticator_id: impl Into<String>,
    authenticator_params: Option<Binary>,
) -> ConfirmExecutionRequest {
    ConfirmExecutionRequest {
        authenticator_id: authenticator_id.into(),
        account,
        fee_payer: mock_fee_payer(),
        fee_granter: None,
        fee: vec![Coin::new(1000u128, "uterp")],
        msg: dummy_any(),
        msg_index: 0,
        authenticator_params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_construct_without_panic() {
        let acct = mock_account();
        let _ = dummy_on_auth_added(acct.clone(), "1", Some(Binary::from(b"p")));
        let _ = dummy_on_auth_removed(acct.clone(), "1", None);
        let _ = dummy_authentication_request(acct.clone(), "1", Binary::from(b"sig"), None);
        let _ = dummy_track_request(acct.clone(), "1", None);
        let _ = dummy_confirm_execution_request(acct, "1", None);
    }
}
