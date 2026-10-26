use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Ownership(#[from] cw_ownable::OwnershipError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Instantiate2 address error: {msg}")]
    Instantiate2AddressError { msg: String },

    #[error("{0}")]
    Svg(#[from] cw_svg::SvgError),

    #[error("unknown series {series}")]
    UnknownSeries { series: String },

    #[error("series {series}: next collection allowed at {next_at} (min interval)")]
    SeriesTooSoon { series: String, next_at: u64 },

    #[error("series {series}: max {max} collections per {window}s reached; next at {next_at}")]
    SeriesWindowFull {
        series: String,
        max: u32,
        window: u64,
        next_at: u64,
    },

    #[error("series {series}: mint_end_time required and at most {max}s after creation")]
    SeriesMintWindow { series: String, max: u64 },
}
