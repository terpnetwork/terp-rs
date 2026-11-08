//! Broadcast MsgCreateValidator for a zcinti consensus public key.
//!
//! `--spawn` broadcasts on chain `240u-1`. A running SpawnedTerp validator
//! is reused. This command boots that chain only when it is not already up.
//!
//! `ZCINTI_PUBKEY` is the public key the Zigner phone shows. A file path is refused.
//!
//! The funded spawn account broadcasts the transaction. Set `CW_ORCH_GRANTER`
//! when a cold account must grant that ephemeral account authz and a feegrant
//! first. The approval page URL (random loopback port) prints at start.

use clap::Parser;
use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
use cosmos_sdk_proto::cosmos::crypto::ed25519::PubKey as Ed25519PubKey;
use cosmos_sdk_proto::cosmos::staking::v1beta1::{
    CommissionRates, Description, MsgCreateValidator,
};
use cosmrs::Any;
use cosmrs::proto::traits::Message;
use cw_orch::daemon::TxSender;
use cw_orch::prelude::*;
use cw_orch_kms::CW_ORCH_GRANTER_ENV;
use cw_orch_kms::{ApprovalSession, SpawnedTerp, enable_approval};

#[derive(Parser)]
struct Args {
    /// Public key shown by Zigner. A file path is refused.
    #[arg(long, env = "ZCINTI_PUBKEY")]
    pubkey: String,
    /// Broadcast on chain 240u-1. Reuse it when it is already running.
    #[arg(long)]
    spawn: bool,
    #[arg(long, default_value = "zcinti")]
    moniker: String,
    /// Self-delegation. The denom must match the spawned chain.
    #[arg(long, default_value = "1000000uterp")]
    amount: String,
}

fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let args = Args::parse();
    if !args.spawn {
        anyhow::bail!("create_val refuses a live chain. Pass --spawn for the local Terp image.");
    }
    let public = phone_pubkey(&args.pubkey)?;
    let spawned = match SpawnedTerp::attach().map_err(|err| anyhow::anyhow!(err))? {
        Some(joined) => joined,
        None => SpawnedTerp::boot().map_err(|err| anyhow::anyhow!(err))?,
    };
    let info = spawned.chain_info();
    println!("operator {}", spawned.signer());
    let granter = std::env::var(CW_ORCH_GRANTER_ENV)
        .ok()
        .filter(|value| !value.is_empty());
    if let Some(granter) = granter.as_deref() {
        spawned.fund(granter).map_err(|err| anyhow::anyhow!(err))?;
        let mut session = ApprovalSession::open(granter)
            .allow_sdk("/cosmos.staking.v1beta1.MsgCreateValidator")
            .map_err(|err| anyhow::anyhow!(err))?;
        session.retain = true;
        let chain = enable_approval(&DaemonBuilder::new(info), session)
            .map_err(|err| anyhow::anyhow!(err))?;
        broadcast(&chain, &args, &public)?;
    } else {
        let chain = DaemonBuilder::new(info).build()?;
        println!("no {CW_ORCH_GRANTER_ENV}; the funded spawn account broadcasts directly");
        broadcast(&chain, &args, &public)?;
    }
    Ok(())
}

fn phone_pubkey(raw: &str) -> anyhow::Result<Vec<u8>> {
    let value = raw.trim();
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value.contains(".kms")
        || value.contains("priv_validator")
    {
        anyhow::bail!("ZCINTI_PUBKEY is the phone's public key, not a key file");
    }
    let public = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
        .map_err(|_| anyhow::anyhow!("phone pubkey refused"))?;
    if public.len() != 32 {
        anyhow::bail!("phone pubkey is {} bytes", public.len());
    }
    Ok(public)
}

fn broadcast(chain: &Daemon, args: &Args, public: &[u8]) -> anyhow::Result<()> {
    let sender = chain.sender();
    let delegator = sender.address().to_string();
    let (_, data) = bech32::decode(&delegator)?;
    let validator = bech32::encode::<bech32::Bech32>(
        bech32::Hrp::parse("terpvaloper").map_err(|err| anyhow::anyhow!(err))?,
        &data,
    )?;
    let (denom, amount) = split_coin(&args.amount)?;
    let key = Ed25519PubKey {
        key: public.to_vec(),
    };
    let msg = MsgCreateValidator {
        description: Some(Description {
            moniker: args.moniker.clone(),
            identity: String::new(),
            website: String::new(),
            security_contact: String::new(),
            details: "zcinti".into(),
        }),
        commission: Some(CommissionRates {
            rate: "100000000000000000".into(),
            max_rate: "200000000000000000".into(),
            max_change_rate: "10000000000000000".into(),
        }),
        min_self_delegation: "1".into(),
        validator_address: validator,
        pubkey: Some(Any {
            type_url: "/cosmos.crypto.ed25519.PubKey".into(),
            value: key.encode_to_vec(),
        }),
        value: Some(Coin { denom, amount }),
        // delegator_address is deprecated: the SDK derives the self-delegator from validator_address.
        ..Default::default()
    };
    let any = Any {
        type_url: "/cosmos.staking.v1beta1.MsgCreateValidator".into(),
        value: msg.encode_to_vec(),
    };
    let response = chain
        .rt_handle
        .block_on(sender.commit_tx_any(vec![any], Some("zcinti create validator")))?;
    println!("broadcast ok code {}", response.code);
    Ok(())
}

fn split_coin(raw: &str) -> anyhow::Result<(String, String)> {
    let raw = raw.trim();
    let split = raw
        .find(|ch: char| ch.is_ascii_alphabetic())
        .ok_or_else(|| anyhow::anyhow!("amount needs a denom, for example 1000000uterp"))?;
    Ok((raw[split..].to_string(), raw[..split].to_string()))
}
