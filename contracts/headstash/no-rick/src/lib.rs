extern crate alloc;
// #[cfg(target_arch = "wasm32")]
// use lol_alloc::{AssumeSingleThreaded, FreeListAllocator};
// // SAFETY: This application is single threaded, so using AssumeSingleThreaded is allowed.
// #[cfg(target_arch = "wasm32")]
// #[global_allocator]
// static ALLOCATOR: AssumeSingleThreaded<FreeListAllocator> =
//     unsafe { AssumeSingleThreaded::new(FreeListAllocator::new()) };
#[cfg(feature = "interface")]
pub mod interface;
#[cfg(feature = "interface")]
pub use interface::NoRickContractSuite;

// version info for migration info
pub const NORICK_CONTRACT: &str = "norick";

// ────────────────────────────────────────────────────────────────────────────

use cosmwasm_schema::{QueryResponses, cw_serde};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    AnyMsg, Binary, Checksum, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, VerificationError, to_json_binary,
};
use ff::PrimeField;
use pasta_curves::vesta;
use prost::Message as _;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const RICK_SUBDENOM: &str = "rick";
const RANDY_SUBDENOM: &str = "randy";
const CREATE_DENOM_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgCreateDenom";
const MINT_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgMint";

// ---------------------------------------------------------------------------
// Contract types
// ---------------------------------------------------------------------------

#[cw_serde]
pub struct Config {
    /// words we are prooving a privte instance does not contain
    pub words: Vec<String>,
}

#[cw_serde]
pub struct InstantiateMsg {}

#[cw_serde]
pub enum ExecuteMsg {
    /// Verify proof only — no side effects
    Proove {
        cid: u64,
        forbidden: String,
        proof: Binary,
    },
    /// Verify proof and mint rick/randy token via x/tokenfactory
    ProoveAndMint {
        cid: u64,
        forbidden: String,
        proof: Binary,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(Checksum)]
    VkChecksum { cid: u64 },
}

#[derive(Error, Debug)]
pub enum Never {}

#[cw_serde]
pub enum SudoMsg {}

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),
    #[error("{0}")]
    VerificationError(#[from] VerificationError),
    #[error("invalid proof")]
    InalidProof {},
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    _deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    _msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let contract_addr = env.contract.address.to_string();

    // Create "rick" denom — minted when proof shows content contained rick
    let create_rick = CosmosMsg::Any(AnyMsg {
        type_url: CREATE_DENOM_TYPE_URL.to_string(),
        value: Binary::from(
            MsgCreateDenom {
                sender: contract_addr.clone(),
                subdenom: RICK_SUBDENOM.to_string(),
            }
            .encode_to_vec(),
        ),
    });

    // Create "randy" denom — minted when proof shows content did NOT contain rick
    let create_randy = CosmosMsg::Any(AnyMsg {
        type_url: CREATE_DENOM_TYPE_URL.to_string(),
        value: Binary::from(
            MsgCreateDenom {
                sender: contract_addr,
                subdenom: RANDY_SUBDENOM.to_string(),
            }
            .encode_to_vec(),
        ),
    });

    Ok(Response::new()
        .add_message(create_rick)
        .add_message(create_randy)
        .add_attribute("method", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Proove {
            cid,
            forbidden,
            proof,
        } => {
            if !deps.api.proof_instance_verify(
                cid.into(),
                &proof,
                &to_cosmwasm_instance(&forbidden),
            )? {
                return Err(ContractError::InalidProof {});
            }
            Ok(Response::new().add_attribute("action", "proove"))
        }
        ExecuteMsg::ProoveAndMint {
            cid,
            forbidden,
            proof,
        } => {
            let subdenom = match deps.api.proof_instance_verify(
                cid.into(),
                &proof,
                &to_cosmwasm_instance(&forbidden),
            )? {
                true => RANDY_SUBDENOM,
                false => RICK_SUBDENOM,
            };

            let contract_addr = env.contract.address.to_string();
            let recipient = info.sender.to_string();
            let denom = format!("factory/{}/{}", contract_addr, subdenom);

            let mint_msg = CosmosMsg::Any(AnyMsg {
                type_url: MINT_TYPE_URL.to_string(),
                value: Binary::from(
                    MsgMint {
                        sender: contract_addr,
                        amount: Some(ProtoCoin {
                            denom,
                            amount: "1".to_string(),
                        }),
                        mint_to_address: recipient,
                    }
                    .encode_to_vec(),
                ),
            });

            Ok(Response::new()
                .add_message(mint_msg)
                .add_attribute("action", "proove_and_mint")
                .add_attribute("result", subdenom))
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::VkChecksum { cid } => {
            to_json_binary(&deps.querier.query_circuit_info(cid)?.circuit_key)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(_deps: DepsMut, _env: Env, _msg: SudoMsg) -> Result<Response, ContractError> {
    Ok(Response::default())
}

// ---------------------------------------------------------------------------
// Instance serialization helpers
// ---------------------------------------------------------------------------

/// serialize instances into set of circuit field bytes with lenth `I`,specifically for cosmwasm-std api
pub fn to_cosmwasm_instance(forbidden: &str) -> Vec<u8> {
    let instances = to_halo2_instance(forbidden);
    let mut bytes = Vec::with_capacity(1 * 32);
    for instance_row in instances.iter() {
        for scalar in instance_row.iter() {
            // to_repr() returns a 32-byte little-endian representation
            bytes.extend_from_slice(scalar.to_repr().as_ref());
        }
    }
    bytes
}

/// serialize instances into set of circuit field [vesta::Scalar] with lenth `I`
/// Must match NoRickInstance::to_halo2_instance() which uses 2 elements:
/// [0] = Fp::one() (constraint result)
/// [1] = str_to_field(forbidden) (the forbidden word)
pub fn to_halo2_instance(f: &str) -> [[vesta::Scalar; 1]; 1] {
    let mut instance = [vesta::Scalar::zero(); 1];
    instance[0] = str_to_field(&f);
    [instance]
}
/// string to field
pub fn str_to_field<F: PrimeField>(s: &str) -> F {
    let mut repr = F::default().to_repr();
    let src = s.as_bytes();
    let len = core::cmp::min(src.len(), repr.as_ref().len());
    repr.as_mut()[..len].copy_from_slice(&src[..len]);
    F::from_repr(repr).expect("str_to_field")
}

// ---------------------------------------------------------------------------
// Tokenfactory protobuf types (mirrors terp_rs::osmosis::tokenfactory::v1beta1)
// ---------------------------------------------------------------------------

/// cosmos.base.v1beta1.Coin
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ProtoCoin {
    #[prost(string, tag = "1")]
    pub denom: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub amount: ::prost::alloc::string::String,
}

/// osmosis.tokenfactory.v1beta1.MsgCreateDenom
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MsgCreateDenom {
    #[prost(string, tag = "1")]
    pub sender: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub subdenom: ::prost::alloc::string::String,
}

/// osmosis.tokenfactory.v1beta1.MsgMint
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MsgMint {
    #[prost(string, tag = "1")]
    pub sender: ::prost::alloc::string::String,
    #[prost(message, optional, tag = "2")]
    pub amount: ::core::option::Option<ProtoCoin>,
    #[prost(string, tag = "3")]
    pub mint_to_address: ::prost::alloc::string::String,
}
