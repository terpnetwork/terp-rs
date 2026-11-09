//! Shared sudo helper surface for every authenticator cw-orch interface.
//!
//! Sudo message types come from existing [`terp_auth`] (billboards package).
//!
//! Note: current cw-orch does not expose a generic `contract.sudo()` helper.
//! Auth sudo goes through Mock `App::wasm_sudo` (cw-multi-test path).

use cw_orch::prelude::*;
use serde::Serialize;
use terp_auth::{
    AuthSudoMsg, AuthenticationRequest, ConfirmExecutionRequest, OnAuthenticatorAddedRequest,
    OnAuthenticatorRemovedRequest, TrackRequest,
};

/// Response type for Mock sudo calls (matches `Mock` / `TxHandler::Response`).
pub type MockSudoResponse = <Mock as TxHandler>::Response;

/// Five x/smart-account sudo entry helpers every authenticator interface implements.
pub trait AuthenticatorSudoExt {
    fn sudo_on_auth_added(
        &self,
        req: OnAuthenticatorAddedRequest,
    ) -> Result<MockSudoResponse, CwOrchError>;

    fn sudo_on_auth_removed(
        &self,
        req: OnAuthenticatorRemovedRequest,
    ) -> Result<MockSudoResponse, CwOrchError>;

    fn sudo_authenticate(
        &self,
        req: AuthenticationRequest,
    ) -> Result<MockSudoResponse, CwOrchError>;

    fn sudo_track(&self, req: TrackRequest) -> Result<MockSudoResponse, CwOrchError>;

    fn sudo_confirm_execution(
        &self,
        req: ConfirmExecutionRequest,
    ) -> Result<MockSudoResponse, CwOrchError>;
}

/// Marker: interface uses [`AuthSudoMsg`] as its sudo message type.
pub trait AuthSudoContract {}

/// Call AuthSudoMsg via Mock `wasm_sudo` (x/smart-account path in multi-test).
pub fn mock_wasm_sudo<C>(
    contract: &C,
    msg: &impl Serialize,
) -> Result<MockSudoResponse, CwOrchError>
where
    C: ContractInstance<Mock>,
{
    let addr = contract.address()?;
    contract
        .as_instance()
        .environment()
        .app
        .borrow_mut()
        .wasm_sudo(addr, msg)
        .map_err(Into::into)
}

/// Blanket helpers for any AuthSudoContract on the default Mock env.
impl<C> AuthenticatorSudoExt for C
where
    C: ContractInstance<Mock> + AuthSudoContract,
{
    fn sudo_on_auth_added(
        &self,
        req: OnAuthenticatorAddedRequest,
    ) -> Result<MockSudoResponse, CwOrchError> {
        mock_wasm_sudo(self, &AuthSudoMsg::OnAuthAdded(req))
    }

    fn sudo_on_auth_removed(
        &self,
        req: OnAuthenticatorRemovedRequest,
    ) -> Result<MockSudoResponse, CwOrchError> {
        mock_wasm_sudo(self, &AuthSudoMsg::OnAuthRemoved(req))
    }

    fn sudo_authenticate(
        &self,
        req: AuthenticationRequest,
    ) -> Result<MockSudoResponse, CwOrchError> {
        mock_wasm_sudo(self, &AuthSudoMsg::Authenticate(Box::new(req)))
    }

    fn sudo_track(&self, req: TrackRequest) -> Result<MockSudoResponse, CwOrchError> {
        mock_wasm_sudo(self, &AuthSudoMsg::Track(req))
    }

    fn sudo_confirm_execution(
        &self,
        req: ConfirmExecutionRequest,
    ) -> Result<MockSudoResponse, CwOrchError> {
        mock_wasm_sudo(self, &AuthSudoMsg::ConfirmExecution(req))
    }
}
