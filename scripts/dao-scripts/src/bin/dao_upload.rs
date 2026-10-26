//! Upload the DAO contract set through one authz and feegrant session.
//!
//! The daemon wallet is the grantee. `--granter` is the cold Terp address.
//! The default session grants store of the three named Wasm files, pinned by
//! checksum, and nothing else. The cold key signs that authorization once on
//! http://127.0.0.1:8741. The same signature lets the ephemeral key revoke
//! the grant after the uploads.
//! `--retain` keeps the grant and skips the page when it already covers the run.
//!
//! Terp stores `cw_tokenfactory_issuer-osmosis.wasm` (the chain module is the
//! Osmosis tokenfactory), `dao_voting_token_staked-default.wasm`, and
//! `cw_vesting-staking.wasm`. The Thorchain builds are not uploaded.
//!
//! `--spawn` boots the local Terp image (`240u-1`) instead of a mock chain,
//! funds the signer (`LOCAL_MNEMONIC`, or a fresh key when that is unset) and the granter from the genesis validator, and
//! runs `DaoDaoSuite::on_spawn` before upload. The container stops when the
//! process exits. `TERP_IMAGE_VERSION` selects the tag; the default is `v6.3.0`.
//!
//! Leave `--granter` empty. The key that signs `cw-orch-granter:v1` is the granter.
//!
//! ```sh
//! cargo run -p dao-scripts --bin dao_upload -- --spawn
//! cargo run -p dao-scripts --bin dao_upload -- --chain terp-mainnet
//! ```

use anyhow::{anyhow, Result};
use clap::Parser;
use cw_orch::prelude::networks::{TERP_MAINNET, TERP_TESTNET};
use cw_orch::prelude::*;
use cw_orch_kms::{
    enable_session, planned_spend_limit, ApprovalSession, SpawnedTerp, SigningRequest,
    CW_ORCH_GRANTER_ENV,
};
use dao_cw_orch::terp_wasm_file;
use dao_testing::DaoDaoSuite;

#[derive(Parser, Debug)]
#[clap(name = "dao-upload")]
struct Cli {
    /// Optional. When empty, the airgapped signature chooses the granter.
    #[clap(long, env = CW_ORCH_GRANTER_ENV, default_value = "")]
    granter: String,

    /// `terp-mainnet` / `morocco-1` or `terp-testnet` / `120u-1`. Ignored with `--spawn`.
    #[clap(long, default_value = "terp-mainnet")]
    chain: String,

    /// Boot the local Terp image, fund the signer and the granter, then run suite preflight.
    #[clap(long)]
    spawn: bool,

    /// Refused. A session names the messages it will send. The wallet catalog
    /// is not a grant.
    #[clap(long)]
    wallet: bool,

    /// How long the wallet grant and feegrant stay valid, in seconds.
    #[clap(long, default_value_t = 2 * 60 * 60)]
    ttl_secs: u64,

    /// Zigner account index for the Terp key. The QR uses this index.
    #[clap(long, default_value_t = 0u32)]
    account_index: u32,

    /// Fee allowance in the chain gas denom. The session wallet spends this.
    /// Fee allowance in uterp. The default covers three stores and the revoke.
    #[clap(long, default_value_t = planned_spend_limit(4))]
    allowance: u128,

    /// Leave the authz and feegrant in place. The default lets the ephemeral
    /// key revoke them after the uploads, from authority in the same cold signature.
    /// A covered grant is reused and the cold key is not asked to sign it again.
    #[clap(long)]
    retain: bool,
}

fn artifact_bytes(logical: &str) -> Result<Vec<u8>> {
    let file = terp_wasm_file(logical);
    let dir = std::env::var("DAO_ARTIFACTS").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../dao-contracts/artifacts")
    });
    let path = dir.join(&file);
    std::fs::read(&path).map_err(|err| anyhow!("read {}: {err}", path.display()))
}

fn upload_with_retry(dao: &dao_testing::DaoDaoSuite<Daemon>) -> Result<()> {
    let mut last = String::new();
    for attempt in 1..=3 {
        match dao.upload_if_needed() {
            Ok(()) => return Ok(()),
            Err(err) => {
                last = err.to_string();
                let transient = last.contains("h2 protocol")
                    || last.contains("stream error")
                    || last.contains("connection reset")
                    || last.contains("error reading a body");
                if !transient || attempt == 3 {
                    return Err(anyhow!(last));
                }
                println!("upload attempt {attempt} lost the chain connection. retrying");
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }
    Err(anyhow!(last))
}

fn resolve_chain(name: &str) -> Result<ChainInfo> {
    match name {
        "terp-mainnet" | "morocco-1" | "mainnet" => Ok(TERP_MAINNET),
        "terp-testnet" | "120u-1" | "testnet" => Ok(TERP_TESTNET),
        other => Err(anyhow!(
            "unknown chain {other:?}; use terp-mainnet / morocco-1 or terp-testnet / 120u-1"
        )),
    }
}

fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
    dao_scripts::kms_gate::load_dotenv_public();
    env_logger::init();

    let cli = Cli::parse();
    println!(
        "terp wasm: issuer={}, voting-token-staked={}, vesting={}",
        terp_wasm_file("cw_tokenfactory_issuer"),
        terp_wasm_file("dao_voting_token_staked"),
        terp_wasm_file("cw_vesting"),
    );
    println!(
        "skipped: dao_voting_token_staked-thorchain.wasm, cw_tokenfactory_issuer-thorchain.wasm"
    );
    let granter = if cli.granter.is_empty() {
        println!("sign once at http://127.0.0.1:8741. that key is the granter");
        cw_orch_kms::capture_granter_sync(
            "127.0.0.1:8741".parse()?,
            cli.account_index,
            "dao-upload",
        )
        .map_err(|err| anyhow!(err))?
    } else {
        cli.granter.clone()
    };
    println!("granter {granter}");
    let spawned = if cli.spawn {
        let chain = SpawnedTerp::boot().map_err(|err| anyhow!(err))?;
        chain.fund(&granter).map_err(|err| anyhow!(err))?;
        println!("funded granter {granter}");
        Some(chain)
    } else {
        None
    };
    let chain = if let Some(spawned) = &spawned {
        let info = spawned.chain_info();
        if info.chain_id != cw_orch_kms::SPAWN_CHAIN_ID {
            return Err(anyhow!(
                "spawn destination is {}, expected {}",
                info.chain_id,
                cw_orch_kms::SPAWN_CHAIN_ID
            ));
        }
        println!(
            "spawn destination {} grpc {}",
            info.chain_id,
            info.grpc_urls.first().map(String::as_str).unwrap_or("")
        );
        info
    } else {
        resolve_chain(&cli.chain)?.into()
    };

    if cli.wallet {
        return Err(anyhow!(
            "--wallet is not a grant. This upload stores three wasm files by checksum and does not grant execute, instantiate, or migrate"
        ));
    }
    let mut session = ApprovalSession::open(&granter);
    for logical in [
        "cw_tokenfactory_issuer",
        "dao_voting_token_staked",
        "cw_vesting",
    ] {
        let bytes = artifact_bytes(logical)?;
        session = session
            .allow_store(&bytes)
            .map_err(|err| anyhow!(err))?;
    }
    session.ttl_secs = cli.ttl_secs;
    session.account_index = cli.account_index;
    session.allowance_limit = cli.allowance;
    session.retain = cli.retain;

    let mut builder = Daemon::builder(chain);
    if spawned.is_some() {
        builder.is_test(true).load_network(false);
    }
    let actions = [
        "cw_tokenfactory_issuer-osmosis.wasm",
        "dao_voting_token_staked-default.wasm",
        "cw_vesting-staking.wasm",
    ]
    .into_iter()
    .map(|name| SigningRequest::new("session-key").body(format!("store {name}")))
    .collect();
    let run = enable_session(&builder, session, actions).map_err(|err| anyhow!(err))?;
    let dao = DaoDaoSuite::new(run.daemon.clone());
    if spawned.is_some() {
        dao.on_spawn()?;
        println!("spawn preflight: signer has a spendable balance");
    }
    println!("uploading contracts through the ephemeral key");
    upload_with_retry(&dao)?;
    let ids = run.daemon.state().get_all_code_ids()?;
    println!("{ids:#?}");
    println!("uploads finished");
    run.close().map_err(|err| anyhow!(err))?;
    Ok(())
}
