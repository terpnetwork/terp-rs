use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Binary;

/// Contract admin configures trusted OIDC/JWT issuers and circuit binding.
#[cw_serde]
pub struct InstantiateMsg {
    pub admin: Option<String>,
    pub issuers: Vec<IssuerConfig>,
    /// When true, claim_commitment must be registered to `req.account`.
    pub require_registered_claim: bool,
}

/// Trusted issuer + circuit binding + **inclusion set root**.
///
/// ## Inclusion set (Headscale / OIDC membership)
///
/// The issuer (or mesh operator) periodically publishes a merkle root over the
/// set of allowed identities (e.g. OIDC `sub` values admitted to a Headscale
/// tailnet, or a JWKS epoch). That root is stored here as `inclusion_set_root`.
///
/// The ZK circuit proves the JWT's subject is a leaf under that root; the root
/// itself is a **public instance** so the contract can check:
///
/// ```text
/// public_inputs[64..96) == IssuerConfig.inclusion_set_root
/// ```
///
/// Rotating the root (new epoch of mesh members) is `RotateInclusionRoot`.
///
/// ## VM verification
///
/// When `zkid` is set (feature `zk-host`):
/// ```ignore
/// deps.api.proof_instance_verify(zkid, proof, public_inputs)?
/// ```
#[cw_serde]
pub struct IssuerConfig {
    /// OIDC issuer URL or stable id (e.g. Headscale IdP issuer).
    pub issuer: String,
    /// Optional client JWKS thumbprint / hint (not used for host PLONK verify).
    pub verifying_key: Binary,
    /// CosmWasm circuit id (required for zk-host path).
    pub zkid: Option<u64>,
    /// Optional audience (`aud`) restriction.
    pub audience: Option<String>,
    /// Merkle / membership root of the inclusion set (32 bytes).  
    /// When `Some`, Authenticate requires matching root in public instances.
    pub inclusion_set_root: Option<Binary>,
}

#[cw_serde]
pub enum ExecuteMsg {
    UpsertIssuer(IssuerConfig),
    RemoveIssuer { issuer: String },
    /// Rotate the inclusion-set root for an issuer (new membership epoch).
    RotateInclusionRoot {
        issuer: String,
        inclusion_set_root: Binary,
    },
    /// Link a claim commitment to the caller for later sudo Authenticate binding.
    RegisterClaim {
        claim_commitment: Binary,
        issuer: String,
    },
    UpdateAdmin { admin: String },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(IssuerConfig)]
    Issuer { issuer: String },
    #[returns(Vec<String>)]
    ListIssuers {},
    #[returns(bool)]
    IsClaimRegistered { claim_commitment: Binary },
    #[returns(Option<String>)]
    ClaimOwner { claim_commitment: Binary },
    #[returns(ConfigResponse)]
    Config {},
    /// Current inclusion root for an issuer (if set).
    #[returns(Option<Binary>)]
    InclusionSetRoot { issuer: String },
}

#[cw_serde]
pub struct ConfigResponse {
    pub admin: String,
    pub require_registered_claim: bool,
}

/// Payload in `AuthenticationRequest.signature` (JSON).
///
/// ## Public instance layout (`public_inputs`)
///
/// | Offset | Len | Field |
/// |--------|-----|--------|
/// | 0 | 32 | `nullifier` |
/// | 32 | 32 | `claim_commitment` |
/// | 64 | 32 | `inclusion_set_root` (required if issuer registered a root) |
/// | 96 | 32 | `msg_bind` (optional) |
/// | 128… | … | circuit-specific rest |
#[cw_serde]
pub struct ZkJwtAuthPayload {
    pub issuer: String,
    /// Must equal bytes [32..64) of `public_inputs`.
    pub claim_commitment: Binary,
    /// Circuit public instances (see table).
    pub public_inputs: Binary,
    /// Opaque proof bytes for the host verifier.
    pub proof: Binary,
    /// Product/circuit catalog id; host uses issuer.zkid.
    pub circuit_id: String,
}
