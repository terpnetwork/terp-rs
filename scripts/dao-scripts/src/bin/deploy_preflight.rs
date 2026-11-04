//! Preflight for the deploys we already have suites for.
//!
//! Prints one JSON plan. It does not boot a chain, sign, or broadcast.
//! Ready steps name the existing command. Gaps are the steps that still
//! have no suite, so they are not given a cw-orch-kms grant.
//!
//! ```sh
//! cargo run -p dao-scripts --bin deploy_preflight
//! ```

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use serde_json::{Value, json};

#[derive(Parser, Debug)]
#[command(name = "deploy_preflight")]
struct Cli {
    /// Directory of built wasm. Defaults to terp-rs/artifacts.
    #[arg(long)]
    artifacts: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let artifacts = cli
        .artifacts
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts"));
    let plan = json!({
        "kms": "cw-orch-kms is the default signer. One cold signature per session on the chain-bound cw-orch-granter:v2 challenge; the ephemeral key broadcasts only the scoped messages, then close revokes. Every script is dry-run unless --broadcast --chain-id morocco-1 and TERP_SCRIPT_BROADCAST=1. Mnemonic signers need TERP_SCRIPT_LEGACY_SIGNER=1.",
        "steps": [
            relay_grants_step(),
            light_clients(&artifacts),
            expired_clients(),
            new_clients(),
            dao_store(),
            steward(),
            polytone_osmosis(),
            bounty_dao(),
            zechub_dao(),
            svg_markets(),
        ]
    });
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn relay_grants_step() -> Value {
    json!({
        "id": "self-relay-grants",
        "status": "ready",
        "command": "cargo run -p dao-scripts --bin relay_grants -- --chains 240u-1,osmosis-1",
        "suite": "cw-orch-kms relay_grants and PacketInspector",
        "does": "The same authz list and a feegrant are prepared for every chain. enable_relay_sessions opens them all before MsgCreateClient, a handshake message, or a packet message.",
        "covers": [
            "MsgCreateClient",
            "MsgUpdateClient",
            "connection open",
            "channel open",
            "MsgRecvPacket",
            "MsgAcknowledgement",
            "MsgTimeout",
            "MsgTransfer"
        ],
    })
}

fn present(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

fn light_clients(dir: &Path) -> Value {
    let have = [
        ("crosslink", "cw_ics08_wasm_crosslink.wasm"),
        ("dregg", "cw_ics08_wasm_dregg.wasm"),
        ("terp", "cw_ics08_wasm_terp.wasm"),
        ("zap1", "cw_ics08_wasm_zap1.wasm"),
    ];
    let files: Vec<Value> = have
        .iter()
        .map(|(role, file)| {
            json!({
                "role": role,
                "file": file,
                "present": present(dir, file),
            })
        })
        .collect();
    json!({
        "id": "light-clients-on-disk",
        "status": "ready",
        "suite": "contracts/light-clients and tests/tests/crosslink_light_client.rs",
        "files": files,
        "kms": "08-wasm store-code is a gov proposal. The signer is the gov module account, not an x/wasm MsgStoreCode from the ephemeral key. dao_upload's store grant does not cover it.",
        "tuning": "A session for these clients has to grant the proposal submit and deposit, then the gov execution, not checksum store alone.",
    })
}

fn expired_clients() -> Value {
    json!({
        "id": "expired-light-clients",
        "status": "suite-ready",
        "command": "cargo run -p terp-orch --bin terp-gov-proposal",
        "suite": "tests/suite/terp-orch/src/gov.rs",
        "does": "Queries each draft client. Expired or frozen clients become MsgRecoverClient messages inside one MsgSubmitProposal. The deposit on the draft is the minimum the proposer pays.",
        "kms": "The deposit wallet is the cw-orch-kms ephemeral after the cold key signs the grant. The proposal signer inside the message stays the gov module account. A fresh 240u-1 chain has no expired client, so this broadcast is not a spawn exercise until a subject and an Active substitute exist.",
        "command_kms": "cargo run -p dao-scripts --bin ibc_recover_clients (or terp-orch --features gov terp_gov_proposal; cw-orch-kms by default, --legacy-signer opts into the mnemonic)",
        "gap": "Needs an expired subject and an Active substitute.",
    })
}

fn new_clients() -> Value {
    json!({
        "id": "crosslink-light-client",
        "status": "artifact-ready",
        "file": "cw_ics08_wasm_crosslink.wasm",
        "suite": "contracts/light-clients/cw-ics08-wasm-crosslink and tests/tests/crosslink_light_client.rs",
        "kms": "Store is a gov proposal whose authority is the gov module, then MsgCreateClient. Not an x/wasm store grant.",
        "not_in_this_cut": ["osmosis", "atomone", "penumbra", "juno"],
    })
}

fn dao_store() -> Value {
    json!({
        "id": "dao-wasm-store",
        "status": "ready",
        "command": "cargo run -p dao-scripts --bin dao_upload -- --spawn",
        "suite": "DaoDaoSuite in dao-testing, exercised by dao_upload",
        "kms": "One session. Store of the named checksums, then revoke. It does not instantiate a DAO.",
    })
}

fn steward() -> Value {
    json!({
        "id": "steward-daos",
        "status": "dry-run",
        "command": "cargo run -p dao-scripts --bin steward_multi_dao",
        "suite": "tests/suite/dao-foundation-suite/src/ibc_ops/steward.rs",
        "does": "Predicts oversight and custody DAO addresses and writes the Foundation proposal. --broadcast sends one propose to PRE_PROPOSE_A through cw-orch-kms.",
        "polytone": "Defaults to the predicted morocco-1 note from polytone_osmosis (not instantiated yet); --no-polytone drops it.",
    })
}

fn polytone_osmosis() -> Value {
    json!({
        "id": "polytone-osmosis",
        "status": "dry-run",
        "command": "cargo run -p dao-scripts --bin polytone_osmosis",
        "does": "Checks DAO-DAO polytone code checksums (osmosis-1 125-128, morocco-1 4-7) and builds the gov proposal that instantiate2s the Terp note and listener.",
        "gap": "Needs the gov proposal to pass, an Osmosis voice (code 128) signed by an osmo1 account, a morocco-1 <-> osmosis-1 connection, and a relayer for the polytone-1 channel.",
    })
}

fn bounty_dao() -> Value {
    json!({
        "id": "bounty-subdao",
        "status": "dry-run",
        "command": "cargo run -p dao-scripts --bin bounty_dao -- --member terp1... --bounty-code-id N --bounty-init init.json",
        "does": "BOUNTY-SUBDAO under the Foundation (morocco_bounty_instantiate): Foundation is wasm and DAO admin, 1,000,000 TERP seed. --broadcast sends one propose to PRE_PROPOSE_A through cw-orch-kms.",
        "gap": "Needs the bounties module code id and its instantiate JSON.",
    })
}

fn svg_markets() -> Value {
    json!({
        "id": "svg-markets",
        "status": "ready",
        "command": "cargo run -p dao-scripts --bin svg_markets -- --spawn",
        "does": "One cw-orch-kms session stores cw721-svg, the minter factory, svg-auction, the reserve auction, and the revenue marketplace. It instantiate2s the markets and the factory, registers marain-orbs and latent-benders, then creates Marain Orbs, Kara & Gippy, and Yi Jing Harmony through the factory.",
        "collections": ["marain-orbs", "kara-gippy", "yijing-harmony"],
    })
}

fn zechub_dao() -> Value {
    json!({
        "id": "zechub-dao",
        "status": "gap",
        "have": "The bounty design cites ZecHub only for the rule that done is not paid.",
        "gap": "No accounts file, no checksum, and no proposal builder. Same steward plan, different members, after those exist.",
    })
}
