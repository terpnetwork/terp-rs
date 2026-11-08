//! Import the two signing keys and broadcast terpz in order.
//!
//! `TERPZ_OPERATOR_MNEMONIC` signs create-validator for the node that is
//! already running. `TERPZ_FOUNDATION_MNEMONIC` signs the next block: the
//! DAO send, then the redelegations. The consensus key is the node's
//! priv_validator key and is not imported here.
//!
//! The create-validator memo is `<node-id>@127.0.0.1:26656`, matching the
//! address this process advertises in net info.
//!
//! Without `--broadcast` the keys are imported, the addresses are checked,
//! and the unsigned create-validator tx is rewritten with that memo. Nothing
//! is signed onto the chain.

use anyhow::{Context, Result, bail};
use clap::Parser;
use dao_foundation_suite::ibc_ops::quorum::{
    CREATE_VALIDATOR_GAS, FOUNDATION_DELEGATOR, FOUNDATION_MNEMONIC_ENV, OPERATOR_MNEMONIC_ENV,
    TERPZ_ADVERTISED_P2P, TERPZ_CONSENSUS_PUBKEY, TERPZ_NODE_ID, assert_foundation_redelegations,
    assert_phase_signers, dotenv_assignments, dotenv_value, validator_advertised_memo,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const OPERATOR_KEY: &str = "operator";
const FOUNDATION_KEY: &str = "foundation";

#[derive(Parser, Debug)]
#[command(name = "quorum_broadcast")]
struct Args {
    /// Dotenv file with the two mnemonics. Values are never printed.
    #[arg(long, default_value = ".env")]
    env_file: PathBuf,
    /// Directory that already holds `01-create-validator.json` and `02-next-block-unsigned.json`.
    #[arg(long)]
    out_dir: PathBuf,
    #[arg(long, env = "TERPD", default_value = "terpd")]
    terpd: PathBuf,
    #[arg(long, default_value = "tcp://127.0.0.1:26657")]
    node: String,
    #[arg(long, env = "CHAIN_ID", default_value = "morocco-1")]
    chain_id: String,
    #[arg(long, default_value = "0.05uthiol")]
    gas_prices: String,
    /// Sign and broadcast. Omit this to stop after the address check.
    #[arg(long, default_value_t = false)]
    broadcast: bool,
    /// Sign `02-next-block-unsigned.json` with the foundation key only.
    /// Skips create-validator and does not read the operator mnemonic.
    /// Pass `--node tcp://192.168.10.101:26657` for the LAN sentry.
    #[arg(long, default_value_t = false)]
    foundation_only: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.foundation_only {
        return broadcast_foundation_rebalance(&args);
    }
    let (operator_mnemonic, foundation_mnemonic) = load_mnemonics(&args.env_file)?;
    let keyring = args.out_dir.join("keyring");
    fs::create_dir_all(&keyring).with_context(|| format!("create {}", keyring.display()))?;
    fs::set_permissions(&keyring, fs::Permissions::from_mode(0o700))?;

    import_key(&args, &keyring, OPERATOR_KEY, &operator_mnemonic)?;
    import_key(&args, &keyring, FOUNDATION_KEY, &foundation_mnemonic)?;
    drop(operator_mnemonic);
    drop(foundation_mnemonic);

    let operator_account = key_address(&args, &keyring, OPERATOR_KEY, false)?;
    let operator_valoper = key_address(&args, &keyring, OPERATOR_KEY, true)?;
    let foundation = key_address(&args, &keyring, FOUNDATION_KEY, false)?;
    if foundation != FOUNDATION_DELEGATOR {
        bail!("foundation key is {foundation}; this plan signs {FOUNDATION_DELEGATOR}");
    }

    let create_cli = args.out_dir.join("01-create-validator.json");
    let next_path = args.out_dir.join("02-next-block-unsigned.json");
    let next_unsigned: serde_json::Value = read_json(&next_path)?;
    let create_unsigned_path = args.out_dir.join("01-create-validator-unsigned.json");
    let create_unsigned = generate_create_validator(&args, &keyring, &create_cli)?;
    assert_phase_signers(
        &create_unsigned,
        &next_unsigned,
        &operator_valoper,
        &foundation,
    )
    .map_err(|err| anyhow::anyhow!(err))?;
    fs::write(
        &create_unsigned_path,
        serde_json::to_vec_pretty(&create_unsigned)?,
    )?;
    let pubkey = create_unsigned
        .pointer("/body/messages/0/pubkey/key")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if pubkey != TERPZ_CONSENSUS_PUBKEY {
        bail!("create-validator pubkey {pubkey} is not the running node's consensus key");
    }

    println!("operator account {operator_account}");
    println!("operator valoper {operator_valoper}");
    println!("foundation {foundation}");
    println!(
        "create-validator memo {}",
        validator_advertised_memo(TERPZ_NODE_ID)
    );
    println!("advertised p2p {TERPZ_ADVERTISED_P2P}");
    if !args.broadcast {
        println!("keys imported into {}", keyring.display());
        println!(
            "unsigned create-validator rewritten at {}",
            create_unsigned_path.display()
        );
        println!(
            "pass --broadcast to send create-validator, wait for the next block, then send the foundation tx"
        );
        return Ok(());
    }

    let create_signed = args.out_dir.join("01-create-validator-signed.json");
    sign(
        &args,
        &keyring,
        OPERATOR_KEY,
        &create_unsigned_path,
        &create_signed,
    )?;
    let create_hash = broadcast_file(&args, &create_signed)?;
    println!("create-validator submitted {create_hash}");
    let included = wait_included(&args, &create_hash, Duration::from_secs(180))?;
    println!("create-validator included in block {included}");
    wait_past_height(&args, included, Duration::from_secs(90))?;

    let next_signed = args.out_dir.join("02-next-block-signed.json");
    sign(&args, &keyring, FOUNDATION_KEY, &next_path, &next_signed)?;
    let next_hash = broadcast_file(&args, &next_signed)?;
    println!("next-block submitted {next_hash}");
    let next_height = wait_included(&args, &next_hash, Duration::from_secs(180))?;
    println!("next-block included in block {next_height}");
    Ok(())
}

fn broadcast_foundation_rebalance(args: &Args) -> Result<()> {
    let next_path = args.out_dir.join("02-next-block-unsigned.json");
    let next_unsigned: serde_json::Value = read_json(&next_path)?;
    if next_unsigned.get("skipped").and_then(|v| v.as_bool()) == Some(true) {
        let reason = next_unsigned
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("the unsigned tx was skipped");
        bail!("refusing to broadcast: {reason}");
    }
    assert_foundation_redelegations(&next_unsigned, FOUNDATION_DELEGATOR)
        .map_err(|err| anyhow::anyhow!(err))?;

    let foundation_mnemonic = foundation_mnemonic(args)?;
    let keyring = args.out_dir.join("keyring");
    fs::create_dir_all(&keyring).with_context(|| format!("create {}", keyring.display()))?;
    fs::set_permissions(&keyring, fs::Permissions::from_mode(0o700))?;
    import_key(args, &keyring, FOUNDATION_KEY, &foundation_mnemonic)?;
    drop(foundation_mnemonic);
    let foundation = key_address(args, &keyring, FOUNDATION_KEY, false)?;
    if foundation != FOUNDATION_DELEGATOR {
        bail!("foundation key is {foundation}; this plan signs {FOUNDATION_DELEGATOR}");
    }
    println!("foundation {foundation}");
    println!("node {}", args.node);
    if !args.broadcast {
        println!("dry run; nothing was signed or broadcast");
        println!("pass --broadcast to send {}", next_path.display());
        return Ok(());
    }
    let next_signed = args.out_dir.join("02-next-block-signed.json");
    sign(args, &keyring, FOUNDATION_KEY, &next_path, &next_signed)?;
    let next_hash = broadcast_file(args, &next_signed)?;
    println!("rebalance submitted {next_hash}");
    let next_height = wait_included(args, &next_hash, Duration::from_secs(180))?;
    println!("rebalance included in block {next_height}");
    Ok(())
}

fn load_mnemonics(path: &Path) -> Result<(String, String)> {
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "read {} and set {OPERATOR_MNEMONIC_ENV} and {FOUNDATION_MNEMONIC_ENV}",
            path.display()
        )
    })?;
    let pairs = dotenv_assignments(&text);
    let operator = mnemonic_from(&pairs, OPERATOR_MNEMONIC_ENV)?;
    let foundation = mnemonic_from(&pairs, FOUNDATION_MNEMONIC_ENV)?;
    if operator == foundation {
        bail!("{OPERATOR_MNEMONIC_ENV} and {FOUNDATION_MNEMONIC_ENV} are the same phrase");
    }
    Ok((operator, foundation))
}

fn foundation_mnemonic(args: &Args) -> Result<String> {
    if let Ok(value) = std::env::var(FOUNDATION_MNEMONIC_ENV) {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return mnemonic_from(
                &[(FOUNDATION_MNEMONIC_ENV.to_string(), value)],
                FOUNDATION_MNEMONIC_ENV,
            );
        }
    }
    let text = fs::read_to_string(&args.env_file).with_context(|| {
        format!(
            "read {} and set {FOUNDATION_MNEMONIC_ENV}",
            args.env_file.display()
        )
    })?;
    mnemonic_from(&dotenv_assignments(&text), FOUNDATION_MNEMONIC_ENV)
}

fn mnemonic_from(pairs: &[(String, String)], key: &str) -> Result<String> {
    let value = dotenv_value(pairs, key)
        .ok_or_else(|| anyhow::anyhow!("{key} is empty in the env file"))?;
    let words = value.split_whitespace().count();
    if words != 12 && words != 24 {
        bail!("{key} must be 12 or 24 words");
    }
    Ok(value.to_string())
}

struct SecretFile(PathBuf);

impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn import_key(args: &Args, keyring: &Path, name: &str, mnemonic: &str) -> Result<()> {
    if key_address(args, keyring, name, false).is_ok() {
        let _ = run(
            args,
            &[
                "keys",
                "delete",
                name,
                "--keyring-backend",
                "test",
                "--keyring-dir",
                &keyring.display().to_string(),
                "--yes",
                "--output",
                "json",
            ],
        );
    }
    let phrase_path = keyring.join(format!(".{name}.phrase"));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&phrase_path)
            .with_context(|| format!("create {}", phrase_path.display()))?;
        file.write_all(mnemonic.as_bytes())?;
    }
    let _secret = SecretFile(phrase_path.clone());
    let keyring_s = keyring.display().to_string();
    let source = phrase_path.display().to_string();
    let output = run(
        args,
        &[
            "keys",
            "add",
            name,
            "--recover",
            "--source",
            &source,
            "--keyring-backend",
            "test",
            "--keyring-dir",
            &keyring_s,
            "--output",
            "json",
        ],
    )?;
    if !output.status.success() {
        bail!("import of {name} failed");
    }
    Ok(())
}

fn key_address(args: &Args, keyring: &Path, name: &str, valoper: bool) -> Result<String> {
    let keyring_s = keyring.display().to_string();
    let mut argv = vec![
        "keys",
        "show",
        name,
        "-a",
        "--keyring-backend",
        "test",
        "--keyring-dir",
        &keyring_s,
    ];
    if valoper {
        argv.extend(["--bech", "val"]);
    }
    let output = run(args, &argv)?;
    if !output.status.success() {
        bail!("no {name} key in {}", keyring.display());
    }
    let address = String::from_utf8(output.stdout)?
        .lines()
        .find(|line| line.starts_with("terp"))
        .unwrap_or("")
        .trim()
        .to_string();
    if address.is_empty() {
        bail!("could not read the {name} address");
    }
    Ok(address)
}

fn generate_create_validator(
    args: &Args,
    keyring: &Path,
    cli_json: &Path,
) -> Result<serde_json::Value> {
    let keyring_s = keyring.display().to_string();
    let file = cli_json.display().to_string();
    let gas = CREATE_VALIDATOR_GAS.to_string();
    let memo = validator_advertised_memo(TERPZ_NODE_ID);
    let output = run(
        args,
        &[
            "tx",
            "staking",
            "create-validator",
            &file,
            "--from",
            OPERATOR_KEY,
            "--keyring-backend",
            "test",
            "--keyring-dir",
            &keyring_s,
            "--chain-id",
            &args.chain_id,
            "--node",
            &args.node,
            "--gas",
            &gas,
            "--gas-prices",
            &args.gas_prices,
            "--note",
            &memo,
            "--generate-only",
            "--output",
            "json",
        ],
    )?;
    if !output.status.success() {
        bail!("create-validator generate-only failed");
    }
    parse_json_stdout(&output.stdout).context("create-validator unsigned json")
}

fn sign(args: &Args, keyring: &Path, from: &str, unsigned: &Path, signed: &Path) -> Result<()> {
    let keyring_s = keyring.display().to_string();
    let input = unsigned.display().to_string();
    let output_document = signed.display().to_string();
    let output = run(
        args,
        &[
            "tx",
            "sign",
            &input,
            "--from",
            from,
            "--keyring-backend",
            "test",
            "--keyring-dir",
            &keyring_s,
            "--chain-id",
            &args.chain_id,
            "--node",
            &args.node,
            "--output-document",
            &output_document,
        ],
    )?;
    if !output.status.success() || !signed.is_file() {
        bail!("signing {from} failed");
    }
    Ok(())
}

fn broadcast_file(args: &Args, signed: &Path) -> Result<String> {
    let file = signed.display().to_string();
    let output = run(
        args,
        &[
            "tx",
            "broadcast",
            &file,
            "--node",
            &args.node,
            "--broadcast-mode",
            "sync",
            "--output",
            "json",
        ],
    )?;
    if !output.status.success() {
        bail!("broadcast failed");
    }
    let body = parse_json_stdout(&output.stdout).context("broadcast response")?;
    let code = body.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
    if code != 0 {
        let log = body.get("raw_log").and_then(|v| v.as_str()).unwrap_or("");
        bail!("chain rejected the tx with code {code}: {log}");
    }
    body.get("txhash")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .context("broadcast response has no txhash")
}

fn wait_included(args: &Args, hash: &str, timeout: Duration) -> Result<u64> {
    let start = Instant::now();
    loop {
        if start.elapsed() > timeout {
            bail!("tx {hash} was not included within {}s", timeout.as_secs());
        }
        let output = run(
            args,
            &[
                "query", "tx", hash, "--node", &args.node, "--output", "json",
            ],
        )?;
        if output.status.success() {
            let body = parse_json_stdout(&output.stdout)?;
            let code = body.get("code").and_then(|v| v.as_u64()).unwrap_or(1);
            if code != 0 {
                let log = body.get("raw_log").and_then(|v| v.as_str()).unwrap_or("");
                bail!("tx {hash} failed in block with code {code}: {log}");
            }
            let height = body
                .get("height")
                .and_then(|v| v.as_str())
                .context("included tx has no height")?
                .parse::<u64>()?;
            return Ok(height);
        }
        sleep(Duration::from_secs(2));
    }
}

fn wait_past_height(args: &Args, height: u64, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    let http = rpc_http(&args.node);
    loop {
        if start.elapsed() > timeout {
            bail!("chain did not pass block {height}");
        }
        if let Some(latest) = latest_height(&http)
            && latest > height
        {
            println!("chain is at block {latest}");
            return Ok(());
        }
        sleep(Duration::from_secs(2));
    }
}

fn latest_height(http: &str) -> Option<u64> {
    let output = Command::new("curl")
        .args(["-fsS", &format!("{http}/status")])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    body.pointer("/result/sync_info/latest_block_height")
        .and_then(|v| v.as_str())
        .and_then(|text| text.parse().ok())
}

fn rpc_http(node: &str) -> String {
    let rest = node
        .strip_prefix("tcp://")
        .or_else(|| node.strip_prefix("http://"))
        .unwrap_or(node);
    format!("http://{rest}")
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn run(args: &Args, argv: &[&str]) -> Result<Output> {
    let mut cmd = Command::new(&args.terpd);
    cmd.args(argv)
        .env_remove(OPERATOR_MNEMONIC_ENV)
        .env_remove(FOUNDATION_MNEMONIC_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.output().with_context(|| {
        format!(
            "run {} {}",
            args.terpd.display(),
            argv.first().copied().unwrap_or("")
        )
    })
}

fn parse_json_stdout(stdout: &[u8]) -> Result<serde_json::Value> {
    let text = String::from_utf8_lossy(stdout);
    let line = text
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .context("no json on stdout")?;
    Ok(serde_json::from_str(line.trim())?)
}
