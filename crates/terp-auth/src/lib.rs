mod authenticator;
mod helpers;
mod macros;
mod types;

pub use helpers::*;
pub use types::*;

pub use types::AuthSudoMsg;

use cosmwasm_std::{DepsMut, Env};

pub trait AuthMethodStructTrait {}

/// # TerpAccountTrait (historically TerpAccountTrait)
///
/// Core trait for CosmWasm-based authenticators in the x/smart-account module.
/// Enables custom on-chain verification of transactions while integrating with
/// the Cosmos SDK authentication flow.
///
/// ## Key Concepts from x/smart-account
pub trait TerpAccountTrait {
    type InstantiateMsg;
    type ExecuteMsg;
    type QueryMsg;
    type SudoMsg;
    type ContractError;
    /// Any custom structure to extend for authentication functionality.
    type AuthMethodStructs;
    type AuthProcessResult;

    /// Authenticate is a wrapper for the specific authentication logic a structure will implement.\
    /// **Note that this function is separate from the `on_auth_request` used in the x/smart-account authentication workflow.**\
    /// Purpose: Internal wrapper for custom auth logic. Use for one-off validation outside the standard flow (e.g., init helpers). Not called by the module—separate from on_auth_request.\
    /// Notes: Can mutate state if needed, but prefer stateless for module integration.
    fn extended_authenticate(
        deps: DepsMut,
        auth: Self::AuthMethodStructs,
    ) -> Self::AuthProcessResult;
    /// ### Entry point for all sudo calls from x/smart-account.
    /// Routes `SudoMsg` into their respective authentication logic. **Use this function in the contracts Sudo entrypoint to route all other contract functions internally**.
    fn process_sudo_auth(deps: DepsMut, env: Env, req: &Self::SudoMsg) -> Self::AuthProcessResult;
    /// Perform specific logic on an authenticator being added to an account.\
    /// Validate/setup on `MsgAddAuthenticator`. Check req.config (user params), store account-specific state (e.g., pubkey).\
    /// Notes: Reject invalid config (return Err). Called before auth is active.\
    /// Relation to Module: Invoked on add; ensures integrity (see OnAuthenticatorAdded).
    fn on_auth_added(
        deps: DepsMut,
        env: Env,
        req: &OnAuthenticatorAddedRequest,
    ) -> Self::AuthProcessResult;
    /// Perform specific logic on an authenticator being removed from account.\
    /// *Ideally, here is where any internal state specific to an account gets cleaned from state*.
    fn on_auth_removed(
        deps: DepsMut,
        env: Env,
        req: &OnAuthenticatorRemovedRequest,
    ) -> Self::AuthProcessResult;
    /// ### `on_auth_request`: Stateless validation of message.
    /// Logic to authenticate or reject messages using custom authentication will live in this function.\
    /// **DO NOT use logic updates any state, it will be discarded during failure and we can make use of other operations in this workflow for state modfication.**
    fn on_auth_request(
        deps: DepsMut,
        env: Env,
        req: &Box<AuthenticationRequest>,
    ) -> Self::AuthProcessResult;
    /// Logic to process stateful events. We expect an authentication request to have been valid, so we can update stateful data based on the message and params.\
    /// Notes: Changes committed always if auth succeeds (not reverted on exec failure). For composites: Called on all subs (AnyOf) or used subs (AllOf).
    /// Relation to Module: Step 5; notifies for future rules (see Track).
    fn on_auth_track(deps: DepsMut, env: Env, req: &TrackRequest) -> Self::AuthProcessResult;
    /// ## on_auth_confirm: Post-Auth Actions
    /// Logic to process stateful events. We expect an authentication request to have been valid, so we can update stateful data based on the message and params.\
    /// Purpose: Post-exec rules (e.g., check spend limits). Ok commits exec changes; Err reverts them.\
    /// Inputs (from ConfirmExecutionRequest): Includes exec results/events.\
    /// Notes: No auth guarantee from on_auth_request (e.g., AnyOf may call on_auth_confirm on unused subs). For composites: OR/AND logic.\
    /// Relation to Module: Post-handler step 7; enforces outcomes (see ConfirmExecution).
    fn on_auth_confirm(
        deps: DepsMut,
        env: Env,
        req: &ConfirmExecutionRequest,
    ) -> Self::AuthProcessResult;
    /// Generic hook for chain-specific events (e.g., epoch triggers). Optional; return Ok(()) if unused.
    fn on_hooks(deps: DepsMut, env: Env) -> Self::AuthProcessResult;
}