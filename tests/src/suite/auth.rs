//! Home for VSCK / authenticator suite imports (`--features auth-suite`).
//!
//! **Parked:** SA-WAVS / SA-DAO / SA-ZKTLS stay frozen. VSCK is in-scope:
//! vote-sdk fail-closed verify + suite tests import types from here.
//!
//! Full `TerpAuthenticatorSuite` is behind `auth-suite-full` (halo2 / other
//! authenticators). Default `auth-suite` is VSCK-only so ict-rs/scripts tests
//! compile without the parked stack.

use cosmwasm_std::{Addr, Binary};

pub use terp_vsck::{
    verify::{verify_envelope, vote_sdk_mirror, DefaultVsckVerifier, VsckVerifier},
    CircuitVerifyingKeys, ContractError as VsckContractError, ExecuteMsg as VsckExecuteMsg,
    InstantiateMsg as VsckInstantiateMsg, VsckAuthPayload, VsckCircuit, VsckRole,
};

#[cfg(feature = "auth-suite-full")]
pub use terp_authenticator_suite::{
    TerpAuthenticatorDeployData, TerpAuthenticatorSuite, ETH_GOLDEN_SIGNER,
};

/// Minimal `Authenticate` sudo body for VSCK matrix tests (no suite fixtures).
pub fn vsck_authenticate_sudo(signature: Binary) -> terp_vsck::SudoMsg {
    let acct = Addr::unchecked("acct");
    terp_vsck::SudoMsg::Authenticate(Box::new(terp_auth::AuthenticationRequest {
        authenticator_id: "1".into(),
        account: acct.clone(),
        fee_payer: acct.clone(),
        fee_granter: None,
        fee: vec![],
        msg: terp_auth::Any {
            type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
            value: Binary::default(),
        },
        msg_index: 0,
        signature,
        sign_mode_tx_data: terp_auth::SignModeTxData {
            sign_mode_direct: Binary::default(),
            sign_mode_textual: None,
        },
        tx_data: terp_auth::TxData {
            chain_id: "terp-test-1".into(),
            account_number: 0,
            sequence: 0,
            timeout_height: 0,
            msgs: vec![],
            memo: String::new(),
        },
        signature_data: terp_auth::SignatureData {
            signers: vec![acct],
            signatures: vec![],
        },
        simulate: false,
        authenticator_params: None,
    }))
}
