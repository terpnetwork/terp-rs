use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Drand public key is not a valid point for the configured scheme")]
    InvalidPubkey,

    #[error("Drand signature did not verify")]
    InvalidSignature,

    #[error("Chained drand scheme requires previous_signature, or a stored signature for round-1")]
    MissingPreviousSignature,

    #[error(
        "Previous signature does not match the signature already stored for the previous round"
    )]
    PreviousSignatureMismatch,

    #[error("Unchained drand scheme does not take a previous signature")]
    UnexpectedPreviousSignature,

    #[error("Round 0 is not a drand round")]
    RoundZero,

    #[error("Round publish time overflowed")]
    RoundTimeOverflow,

    #[error("Round {round} is not stored")]
    RoundUnknown { round: u64 },

    #[error("Stored beacon for this round does not match the newly verified signature")]
    BeaconConflict,

    #[error("Job id is longer than the nois maximum of 64 bytes")]
    JobIdTooLong,

    #[error("Job already queued for this requester and job id")]
    JobExists,

    #[error("Payment does not cover a configured price")]
    InsufficientPayment,

    #[error("Price amount must be positive")]
    ZeroPrice,

    #[error("Duplicate price denom")]
    DuplicatePriceDenom,

    #[error("This message does not accept funds")]
    UnexpectedFunds,

    #[error("Attestor public key must be a 33-byte compressed secp256k1 key")]
    InvalidAttestorPubkey,

    #[error("Attestor is already registered")]
    AttestorExists,

    #[error("Attestor is not registered")]
    AttestorNotRegistered,

    #[error("Attestation signature did not verify")]
    InvalidAttestation,

    #[error("Attestation already stored for this round and key")]
    AttestationExists,

    #[error("Callback gas limit must be non-zero")]
    InvalidGasLimit,

    #[error("Drand period must be non-zero")]
    InvalidPeriod,

    #[error("Drand chain hash must be 32 bytes")]
    InvalidChainHash,

    #[error("Round {round} is stored but does not have the required attestations")]
    RoundNotDeliverable { round: u64 },

    #[error("Unknown reply id {id}")]
    UnknownReplyId { id: u64 },
}

/// `StdError` is not `PartialEq` in CosmWasm 4, so errors compare by message.
impl PartialEq for ContractError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}
