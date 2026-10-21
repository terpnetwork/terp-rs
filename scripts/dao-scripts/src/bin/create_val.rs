use cosmos_sdk_proto::cosmos::base::v1beta1::Coin;
use cosmos_sdk_proto::cosmos::staking::v1beta1::{
    CommissionRates, Description, MsgCreateValidator,
};
use cosmrs::Any;
use cosmrs::tendermint::PublicKey as TendermintPublicKey;
use cw_orch::{daemon::TxSender, prelude::*};
use terp_rs::{Message, Name};

fn main() -> anyhow::Result<()> {
    // Fix for rustls 0.23+ CryptoProvider
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls ring crypto provider");

    // let args = Args::parse();

    let terp: ChainInfoOwned = networks::TERP_MAINNET.to_owned().into();
    // let terp: ChainInfoOwned = match args.network.as_str() {
    //     "main" => networks::TERP_MAINNET.to_owned(),
    //     _ => panic!("Invalid network"),
    // }
    // .into();

    // connect to chain with mnemonic
    let chain = DaemonBuilder::new(terp.clone()).build()?;
    let sender = chain.sender();

    // Get sender addresses
    let delegator_addr = sender.address();
    // todo: get val address for sender
    let _validator_addr = sender;

    // Load consensus public key - this should be your validator's consensus pubkey
    // You can load from file or environment variable
    let consensus_pubkey_bytes = std::fs::read("config/priv_validator_key.json")
        .or_else(|_| std::env::var("CONSENSUS_PUBKEY").map(|s| s.into_bytes()))?;

    // Parse tendermint public key
    let tm_pubkey = TendermintPublicKey::from_raw_ed25519(&consensus_pubkey_bytes).unwrap();

    // Build validator description
    let description = Description::default();
    // let description = Description {
    //     moniker: args.moniker,
    //     identity: args.identity.unwrap_or_default(),
    //     website: args.website.unwrap_or_default(),
    //     security_contact: args.security_contact.unwrap_or_default(),
    //     details: args.details.unwrap_or_default(),
    // };

    // Build commission rates
    let commission = CommissionRates::default();
    // let commission = CommissionRates {
    // let commission = CommissionRates {
    //     rate: args.commission_rate.parse()?,
    //     max_rate: args.commission_max_rate.parse()?,
    //     max_change_rate: args.commission_max_change_rate.parse()?,
    // };

    // Build initial self-delegation
    let value = Coin::default();
    // let value = Coin {
    //     denom: args.denom,
    //     amount: args.amount.to_string(),
    // };

    // Create the validator message
    let msg_create_validator = MsgCreateValidator {
        description: Some(description),
        commission: Some(commission),
        min_self_delegation: String::default(),
        delegator_address: delegator_addr.to_string(),
        validator_address: String::default(),
        pubkey: Some(Any {
            type_url: "/cosmos.crypto.ed25519.PubKey".to_string(),
            value: tm_pubkey.to_bytes(),
        }),
        value: Some(value),
    };

    let msg = Any {
        type_url: MsgCreateValidator::type_url(),
        value: msg_create_validator.encode_to_vec(),
    };

    match chain
        .rt_handle
        .block_on(sender.commit_tx_any(vec![msg], None))
    {
        Ok(res) => {
            eprintln!("[BROADCAST] Simulation OK — {:#?}", res);
        }
        Err(e) => {
            eprintln!("[BROADCAST] Simulation FAILED: {:?}", e);
            return Err(anyhow::anyhow!("[BROADCAST] Simulation error: {}", e));
        }
    }

    // // Build and send transaction
    // println!("Validator created successfully!");
    // println!("Transaction hash: {}", tx_response.txhash);
    // println!("Gas used: {}", tx_response.gas_used);
    // println!("Validator address: {}", validator_addr);
    Ok(())
}
