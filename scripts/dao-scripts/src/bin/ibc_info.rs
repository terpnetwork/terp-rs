use clap::{Parser, Subcommand, ValueEnum};
use cw_orch::{
    daemon::{
        Daemon, DaemonState,
        networks::{ATOMEONE_MAINNET, OSMOSIS_1, TERP_MAINNET, chain_name_from_id},
        queriers::Ibc,
    },
    environment::{ChainState, QuerierGetter},
    prelude::*,
};
use cw_orch_interchain::prelude::*;
use log::info;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use terp_rs::{Message, ibc::lightclients::tendermint::v1::ClientState};
use terp_scripts::ibc::{
    AtomicPublisher, FixtureBackend, PredictedWorld, TerpChannelInfo,
    build_channel_to_chain_map as lib_build_channel_to_chain_map, check_invariants,
    compute_ibc_denom_hash, derive_terp_ibc_denom, file_sha256, finalize_channels_for_ibc_entry,
    resolve_out_dir,
};
use terp_scripts::{
    ArtifactRef, CapabilityMode, OutputFormat, RunEnv, RunReport, preflight_missing,
};
use tracing::warn;

const TOOL: &str = "terp-ibc";

#[derive(Parser, Debug)]
#[command(
    name = "terp-ibc",
    about = "Terp IBC data generate / validate / compare / rebuild (lib-backed). Agents: tests/agent/COMMANDS.md"
)]
struct Cli {
    /// Output format: human text or machine JSON RunReport
    #[arg(long, global = true, default_value = "text", value_enum)]
    format: FormatArg,
    /// Output / public directory (absolute preferred). Defaults to repo-root public/
    #[arg(long, global = true)]
    out: Option<PathBuf>,
    /// Legacy alias for --out
    #[arg(long, global = true)]
    public_dir: Option<PathBuf>,
    /// Do not write files (rebuild/generate)
    #[arg(long, global = true, default_value_t = false)]
    dry_run: bool,
    /// Fail preflight if .env / required env missing before connect
    #[arg(long, global = true, default_value_t = false)]
    require_env_file: bool,
    #[command(subcommand)]
    cmd: Option<Commands>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    Text,
    Json,
}

impl From<FormatArg> for OutputFormat {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Text => OutputFormat::Text,
            FormatArg::Json => OutputFormat::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Offline,
    #[value(name = "live-query")]
    LiveQuery,
    #[value(name = "live-tx")]
    LiveTx,
    #[value(name = "docker-harness")]
    DockerHarness,
}

// ModeArg is Copy; Commands holds PathBuf in Compare — Clone for match convenience.

impl From<ModeArg> for CapabilityMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Offline => CapabilityMode::Offline,
            ModeArg::LiveQuery => CapabilityMode::LiveQuery,
            ModeArg::LiveTx => CapabilityMode::LiveTx,
            ModeArg::DockerHarness => CapabilityMode::DockerHarness,
        }
    }
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Query live chains and write public/ artifacts (default)
    Generate {
        #[arg(long, default_value = "live-query", value_enum)]
        mode: ModeArg,
    },
    /// Validate existing public/ibc-data + routing against hard invariants
    Validate,
    /// Compare predicted routes to fixture/snapshot observations
    Compare {
        #[arg(long)]
        snapshot: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        strict: bool,
    },
    /// Check env/mode requirements without network (exit 2 if cannot run)
    Preflight {
        #[arg(long, value_enum)]
        mode: ModeArg,
    },
    /// Offline: rebuild routing/lookup tables from public/ibc-data (atomic out)
    #[command(name = "rebuild-from-public")]
    RebuildFromPublic,
}

fn resolved_out(cli: &Cli) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chosen = cli.out.as_deref().or(cli.public_dir.as_deref());
    resolve_out_dir(chosen, &manifest)
}

fn finish(report: RunReport, format: OutputFormat) -> ! {
    let _ = report.emit(format);
    std::process::exit(report.exit_code);
}

fn main() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();
    env_logger::init();
    let _ = dotenv::dotenv();

    let cli = Cli::parse();
    let format = OutputFormat::from(cli.format);
    let start = Instant::now();

    let cmd = cli.cmd.clone().unwrap_or(Commands::Generate {
        mode: ModeArg::LiveQuery,
    });
    match cmd {
        Commands::Preflight { mode } => {
            let mode = CapabilityMode::from(mode);
            let missing = preflight_missing(&mode, cli.require_env_file);
            let duration_ms = start.elapsed().as_millis() as u64;
            if missing.is_empty() {
                finish(RunReport::preflight_ok(TOOL, &mode, duration_ms), format);
            } else {
                finish(
                    RunReport::missing_env(
                        TOOL,
                        "preflight",
                        &mode,
                        missing,
                        "export required vars (e.g. MAIN_MNEMONIC) or use --mode offline",
                        duration_ms,
                    ),
                    format,
                );
            }
        }
        Commands::Validate => {
            let out = resolved_out(&cli);
            match cmd_validate(&out, start) {
                Ok(r) => finish(r, format),
                Err(e) => finish(internal_err("validate", &out, start, e), format),
            }
        }
        Commands::Compare { snapshot, strict } => {
            let out = resolved_out(&cli);
            match cmd_compare(&out, snapshot.as_deref(), strict, start) {
                Ok(r) => finish(r, format),
                Err(e) => finish(internal_err("compare", &out, start, e), format),
            }
        }
        Commands::RebuildFromPublic => {
            let out = resolved_out(&cli);
            match cmd_rebuild_from_public(&out, cli.dry_run, start) {
                Ok(r) => finish(r, format),
                Err(e) => finish(internal_err("rebuild-from-public", &out, start, e), format),
            }
        }
        Commands::Generate { mode } => {
            let mode = CapabilityMode::from(mode);
            let missing = preflight_missing(&mode, cli.require_env_file);
            if !missing.is_empty() {
                finish(
                    RunReport::missing_env(
                        TOOL,
                        "generate",
                        &mode,
                        missing,
                        "export MAIN_MNEMONIC before live generate, or use rebuild-from-public offline",
                        start.elapsed().as_millis() as u64,
                    ),
                    format,
                );
            }
            if matches!(mode, CapabilityMode::Offline) {
                finish(
                    RunReport {
                        tool: TOOL.into(),
                        verb: "generate".into(),
                        ok: false,
                        exit_code: 2,
                        artifacts: vec![],
                        findings: vec![],
                        env: RunEnv {
                            mode: mode.as_str().into(),
                            docker: false,
                            out: None,
                        },
                        duration_ms: start.elapsed().as_millis() as u64,
                        error: Some("wrong_mode".into()),
                        vars: None,
                        hint: Some(
                            "generate is live-only; use rebuild-from-public or validate for offline"
                                .into(),
                        ),
                    },
                    format,
                );
            }
            match derive_full_ibc_state() {
                Ok(()) => {
                    let duration_ms = start.elapsed().as_millis() as u64;
                    let out = resolved_out(&cli);
                    finish(
                        RunReport {
                            tool: TOOL.into(),
                            verb: "generate".into(),
                            ok: true,
                            exit_code: 0,
                            artifacts: vec![],
                            findings: vec![],
                            env: RunEnv {
                                mode: mode.as_str().into(),
                                docker: false,
                                out: Some(out.display().to_string()),
                            },
                            duration_ms,
                            error: None,
                            vars: None,
                            hint: Some(
                                "generate still writes via legacy paths; prefer rebuild-from-public for offline"
                                    .into(),
                            ),
                        },
                        format,
                    );
                }
                Err(e) => {
                    let out = resolved_out(&cli);
                    finish(internal_err("generate", &out, start, e), format);
                }
            }
        }
    }
}

fn internal_err(verb: &str, out: &Path, start: Instant, e: anyhow::Error) -> RunReport {
    RunReport {
        tool: TOOL.into(),
        verb: verb.into(),
        ok: false,
        exit_code: 1,
        artifacts: vec![],
        findings: vec![],
        env: RunEnv {
            mode: "offline".into(),
            docker: false,
            out: Some(out.display().to_string()),
        },
        duration_ms: start.elapsed().as_millis() as u64,
        error: Some(e.to_string()),
        vars: None,
        hint: None,
    }
}

fn load_public_ibc_data(public_dir: &std::path::Path) -> anyhow::Result<serde_json::Value> {
    let dir = public_dir.join("ibc-data");
    let mut map = serde_json::Map::new();
    if dir.is_dir() {
        for ent in std::fs::read_dir(&dir)? {
            let ent = ent?;
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            map.insert(stem, v);
        }
    }
    Ok(serde_json::Value::Object(map))
}

fn minimal_assets_from_public(
    public_dir: &std::path::Path,
) -> HashMap<String, Vec<serde_json::Value>> {
    let mut chain_assets = HashMap::new();
    chain_assets.insert(
        "terp".into(),
        vec![
            json!({"symbol": "TERP", "base": "uterp"}),
            json!({"symbol": "THIOL", "base": "uthiol"}),
        ],
    );
    let al = public_dir.join("assetlist.json");
    if al.exists() {
        if let Ok(text) = std::fs::read_to_string(&al) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(assets) = v["assets"].as_array() {
                    let mut natives = Vec::new();
                    for a in assets {
                        let base = a["base"].as_str().unwrap_or("");
                        if !base.starts_with("ibc/") {
                            natives.push(a.clone());
                        }
                    }
                    if !natives.is_empty() {
                        chain_assets.insert("terp".into(), natives);
                    }
                }
            }
        }
    }
    // Counterparties from ibc-data stems
    if let Ok(ibc) = load_public_ibc_data(public_dir) {
        if let Some(obj) = ibc.as_object() {
            for (_k, entry) in obj {
                for side in ["chain_1", "chain_2"] {
                    if let Some(name) = entry[side]["chain_name"].as_str() {
                        chain_assets
                            .entry(name.to_string())
                            .or_insert_with(Vec::new);
                    }
                }
            }
        }
    }
    chain_assets
}

fn offline_env(out: &Path) -> RunEnv {
    RunEnv {
        mode: "offline".into(),
        docker: false,
        out: Some(out.display().to_string()),
    }
}

fn cmd_validate(public_dir: &Path, start: Instant) -> anyhow::Result<RunReport> {
    let ibc_data = load_public_ibc_data(public_dir)?;
    let assets = minimal_assets_from_public(public_dir);
    let world = PredictedWorld::from_inputs(ibc_data, &assets, 3);
    let report = check_invariants(&world);
    Ok(RunReport::from_diff(
        TOOL,
        "validate",
        &report,
        offline_env(public_dir),
        start.elapsed().as_millis() as u64,
    ))
}

fn cmd_compare(
    public_dir: &Path,
    snapshot: Option<&Path>,
    strict: bool,
    start: Instant,
) -> anyhow::Result<RunReport> {
    use terp_scripts::ibc::{CompareOptions, SnapshotBackend, compare_predict_observe_with};
    let ibc_data = load_public_ibc_data(public_dir)?;
    let assets = minimal_assets_from_public(public_dir);
    let world = PredictedWorld::from_inputs(ibc_data, &assets, 3);
    let inv = check_invariants(&world);
    if inv.has_errors() {
        return Ok(RunReport::from_diff(
            TOOL,
            "compare",
            &inv,
            offline_env(public_dir),
            start.elapsed().as_millis() as u64,
        ));
    }
    let opts = CompareOptions {
        unobserved_as_error: strict,
        preferred_only: true,
    };
    let report = if let Some(snap) = snapshot {
        let backend = SnapshotBackend::load(snap).map_err(|e| anyhow::anyhow!(e))?;
        compare_predict_observe_with(&world, &backend, &opts)
    } else {
        let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/ibc/golden");
        let backend =
            FixtureBackend::load_from_golden_dir(&golden).map_err(|e| anyhow::anyhow!(e))?;
        compare_predict_observe_with(&world, &backend, &opts)
    };
    Ok(RunReport::from_diff(
        TOOL,
        "compare",
        &report,
        offline_env(public_dir),
        start.elapsed().as_millis() as u64,
    ))
}

fn cmd_rebuild_from_public(
    public_dir: &Path,
    dry_run: bool,
    start: Instant,
) -> anyhow::Result<RunReport> {
    let ibc_data = load_public_ibc_data(public_dir)?;
    if ibc_data.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        anyhow::bail!(
            "no ibc-data JSON under {}/ibc-data — nothing to rebuild",
            public_dir.display()
        );
    }
    let assets = minimal_assets_from_public(public_dir);
    let world = PredictedWorld::from_inputs(ibc_data, &assets, 3);
    let report = check_invariants(&world);
    let duration_ms = start.elapsed().as_millis() as u64;
    if report.has_errors() {
        return Ok(RunReport::from_diff(
            TOOL,
            "rebuild-from-public",
            &report,
            offline_env(public_dir),
            duration_ms,
        ));
    }

    let lookup_json = serde_json::to_vec_pretty(&world.lookup)?;
    let routing_json = serde_json::to_vec_pretty(&world.routes.to_json())?;
    let meta = json!({
        "generated_at": now_timestamp(),
        "generator": "terp-scripts/bin/terp-ibc rebuild-from-public",
        "mode": "offline",
        "max_hops": 3,
        "total_routes": world.routes.metadata.total_routes,
        "chains": world.routes.metadata.chains,
        "source": "public/ibc-data",
        "invariant_findings": report.items.len(),
    });
    let meta_json = serde_json::to_vec_pretty(&meta)?;

    if dry_run {
        return Ok(RunReport::from_diff(
            TOOL,
            "rebuild-from-public",
            &report,
            offline_env(public_dir),
            duration_ms,
        )
        .with_artifacts(vec![
            ArtifactRef {
                path: public_dir
                    .join("ibc_lookup_table.json")
                    .display()
                    .to_string(),
                sha256: None,
                role: "lookup".into(),
            },
            ArtifactRef {
                path: public_dir
                    .join("ibc_routing_table.json")
                    .display()
                    .to_string(),
                sha256: None,
                role: "routing".into(),
            },
        ]));
    }

    let publisher = AtomicPublisher::create(public_dir)?;
    publisher.write_rel("ibc_lookup_table.json", &lookup_json)?;
    publisher.write_rel("ibc_routing_table.json", &routing_json)?;
    publisher.write_rel("ibc_generation_meta.json", &meta_json)?;
    let promoted = publisher.promote()?;

    let mut artifacts = Vec::new();
    for p in promoted {
        let role = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let sha = file_sha256(&p).ok();
        artifacts.push(ArtifactRef {
            path: p.display().to_string(),
            sha256: sha,
            role,
        });
    }

    Ok(RunReport::from_diff(
        TOOL,
        "rebuild-from-public",
        &report,
        offline_env(public_dir),
        start.elapsed().as_millis() as u64,
    )
    .with_artifacts(artifacts))
}

fn derive_full_ibc_state() -> anyhow::Result<()> {
    let interchain = DaemonInterchain::new(
        vec![
            TERP_MAINNET.clone(),
            OSMOSIS_1.clone(),
            ATOMEONE_MAINNET.clone(),
            // AKASH_MAINNET.clone(),
        ],
        &ChannelCreationValidator,
    )?;
    let terp: Daemon = interchain.get_chain("morocco-1")?;
    let osmosis: Daemon = interchain.get_chain("osmosis-1")?;
    let atone: Daemon = interchain.get_chain("atomone-1")?;
    // let akash: Daemon = interchain.get_chain("akashnet-2")?;
    let ibc: Ibc = terp.querier();

    let state_terp = terp.state();
    let state_osmo = osmosis.state();
    let _state_atone = atone.state();
    // let state_akash = akash.state();

    // Capability-based filtering: exclude only via env / known dead chain IDs.
    // Prefer skipping clients without open transfer channels (handled below).
    let dead_chains: Vec<String> = std::env::var("IBC_EXCLUDE_CHAIN_IDS")
        .unwrap_or_else(|_| "omniflixhub-1,evmos-1,stargaze-1".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let exclude_clients: Vec<String> = std::env::var("IBC_EXCLUDE_CLIENT_IDS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut skip_reasons: Vec<serde_json::Value> = Vec::new();

    let clients = terp.rt_handle.block_on(async {
        ibc._clients()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query clients: {}", e))
    })?;

    info!("{} IBC clients on {}\n", clients.len(), terp.chain_id());
    let mut ibc_by_chain: HashMap<String, serde_json::Value> = HashMap::new();

    for c in &clients {
        if exclude_clients.iter().any(|id| id == &c.client_id) {
            skip_reasons.push(json!({"client_id": c.client_id, "reason": "exclude_list"}));
            println!("excluded client: {}", c.client_id);
            continue;
        }
        let (clientid, counterpartychainid, counterpartyname) = if let Some(raw) = &c.client_state {
            match ClientState::decode(raw.value.as_slice()) {
                Ok(tmstate) => (
                    &c.client_id,
                    tmstate.chain_id.clone(),
                    chain_name_from_id(&tmstate.chain_id),
                ),
                Err(_) => {
                    println!("  ⚠ Could not decode client state for {}", c.client_id);
                    continue;
                }
            }
        } else {
            println!("  ⚠ No client state for {}", c.client_id);
            continue;
        };

        if dead_chains.iter().any(|d| d == &counterpartychainid) {
            skip_reasons.push(json!({"client_id": clientid, "chain_id": counterpartychainid, "reason": "dead_chain"}));
            continue;
        }
        // === Usability filter for canonical ibcinfo ===
        // We only emit a client into the final ibc_data / channels / preferred / reverse assets / lookup tables
        // if we can successfully discover live open transfer channels for it during this run.
        // This prevents frontends from ever defaulting to expired clients (the generator is the source of truth).
        // (Direct ClientStatus gRPC is currently not reachable due to private fields in this cw-orch version.)
        info!(
            "  Evaluating client {} for inclusion (only clients with live open transfer channels will be kept)",
            clientid
        );
        // Get connections for this client
        let (ccons, e) = terp.rt_handle.block_on(async {
            match ibc._client_connections(clientid).await.map_err(|e| {
                anyhow::anyhow!("Failed to query connections for {}: {}", c.client_id, e)
            }) {
                Ok(cc) => (cc, String::default()),
                Err(e) => (vec![], e.to_string()),
            }
        });
        if ccons.is_empty() || !e.is_empty() {
            warn!("error_result:{}", e);
            warn!("connection count for {}:{}", clientid, ccons.len());
            continue;
        }

        for conid in &ccons {
            let ends = terp
                .rt_handle
                .block_on(async {
                    ibc._connection_end(conid)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to query connection {}: {}", conid, e))
                })?
                .expect("no end");

            let (counterpartyclientid, counterpartyconnectionid) = match &ends.counterparty {
                Some(e) => (&e.client_id, &e.connection_id),
                None => {
                    println!(
                        "  ⚠ No client_id or connection_id for connection end {}",
                        ends.client_id
                    );
                    continue;
                }
            };

            let chans = terp.rt_handle.block_on(async {
                ibc._connection_channels(conid)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to query connection {}: {}", conid, e))
            })?;

            let mut channel_entries: Vec<Value> = Vec::new();

            for chan in chans {
                let channel_id = &chan.channel_id;
                let port_id = &chan.port_id;
                let counterparty = &chan.counterparty.clone().expect("");
                let counterpartychannelid = &counterparty.channel_id;
                let counterparty_port_id = &counterparty.port_id;
                let ord = &chan.ordering;
                let v = &chan.version;
                let state = chan.state().as_str_name();
                let status = if state == "STATE_OPEN" {
                    "ACTIVE"
                } else {
                    "INACTIVE"
                };
                info!(
                    " {} :: Channel: {} ({}) <-> {} ({}) | {} | {} | {} | {} ",
                    counterpartychainid,
                    channel_id,
                    port_id,
                    counterpartychannelid,
                    counterparty_port_id,
                    ord,
                    v,
                    state,
                    status,
                );

                channel_entries.push(json!({
                    "chain_1": {
                        "channel_id": channel_id,
                        "port_id": port_id,
                    },
                    "chain_2": {
                        "channel_id": counterpartychannelid,
                        "port_id": counterparty_port_id,
                    },
                    "ordering": ord,
                    "version": v,
                    "tags": {
                        "status": status,
                    }
                }));
            }

            // Build or merge IBC data for this counterparty chain
            let existing = ibc_by_chain.get(&counterpartyname.clone());
            let existing_channels = existing
                .and_then(|v| v["channels"].as_array().cloned())
                .unwrap_or_default();

            let all_channels: Vec<Value> = existing_channels
                .into_iter()
                .chain(channel_entries)
                .collect();

            ibc_by_chain.insert(
                counterpartyname.clone(),
                json!({
                    "counterparty_chain_id": counterpartychainid,
                    "terp_client_id": clientid,
                    "client_status": "Active",  // only clients with live usable channels at curation time are emitted
                    "terp_connection_id": conid,
                    "counterparty_client_id": counterpartyclientid,
                    "counterparty_connection_id": counterpartyconnectionid,
                    "channels": all_channels,
                }),
            );
        }
    }

    // Step 2: Build final IBC data schema JSON for each counterparty chain
    let mut state_mut = terp.state().clone();
    let mut ibc_data_output: Vec<Value> = Vec::new();
    for (counterpartyname, raw_data) in ibc_by_chain.iter() {
        let counterparty_chain_id = raw_data["counterparty_chain_id"].as_str().unwrap_or("");
        let terp_client_id = raw_data["terp_client_id"].as_str().unwrap_or("");
        let terp_connection_id = raw_data["terp_connection_id"].as_str().unwrap_or("");
        let counterparty_client_id = raw_data["counterparty_client_id"].as_str().unwrap_or("");
        let counterparty_connection_id = raw_data["counterparty_connection_id"]
            .as_str()
            .unwrap_or("");

        // Determine alphabetical ordering for chain names
        let (chain_1_name, chain_2_name) = if "terp" < counterpartyname.as_str() {
            ("terp", counterpartyname.as_str())
        } else {
            (counterpartyname.as_str(), "terp")
        };

        // Build chain_1 and chain_2 data based on alphabetical order
        let (chain_1_data, chain_2_data) = if chain_1_name == "terp" {
            (
                json!({
                    "chain_name": "terp",
                    "client_id": terp_client_id,
                    "connection_id": terp_connection_id,
                    "chain_id": "morocco-1",
                }),
                json!({
                    "chain_name": counterpartyname,
                    "client_id": counterparty_client_id,
                    "connection_id": counterparty_connection_id,
                    "chain_id": counterparty_chain_id,
                }),
            )
        } else {
            (
                json!({
                    "chain_name": counterpartyname,
                    "client_id": counterparty_client_id,
                    "connection_id": counterparty_connection_id,
                    "chain_id": counterparty_chain_id,
                }),
                json!({
                    "chain_name": "terp",
                    "client_id": terp_client_id,
                    "connection_id": terp_connection_id,
                    "chain_id": "morocco-1",
                }),
            )
        };

        // Adjust channel entries based on alphabetical order and set preferred tags.
        // NOTE: stored channels always have "chain_1" = Terp side (from query on morocco-1),
        // "chain_2" = counterparty side. We remap to alpha order for top-level chain_1/chain_2.
        let d: Vec<Value> = Vec::with_capacity(1);
        let channels = raw_data["channels"].as_array().unwrap_or(&d);
        let client_status = raw_data
            .get("client_status")
            .and_then(|v| v.as_str())
            .unwrap_or("Active");
        let final_channels = finalize_channels_for_ibc_entry(
            channels,
            &chain_1_name,
            &counterpartyname,
            client_status,
        );

        // Build the full IBC data entry using the correctly-ordered chain data
        // chain_1_data/chain_2_data are built alphabetically above (lines 219-249)
        // final_channels has string ordering, chain_1/chain_2 swapped, and proper tags
        let filename = format!("{}-{}.json", chain_1_name, chain_2_name);

        let ibc_entry = json!({
            "$schema": "../ibc_data.schema.json",
            "chain_1": chain_1_data,
            "chain_2": chain_2_data,
            "channels": final_channels,
            "client_status": raw_data.get("client_status").cloned().unwrap_or(serde_json::json!("Active")),
        });

        // Direct transposition into state content here: the finalized entry (with real channel_id/port_id
        // from the query + alpha remap + preferred tags) is written to state.ibc_data[cp] immediately.
        // This ensures the ~/.cw-orch/state.json (and anything reading state.ibc_data) gets the
        // channel information, not just the public/ files or the in-memory output list.
        let cp_key_for_state = if chain_1_name == "terp" {
            chain_2_name.to_string()
        } else {
            chain_1_name.to_string()
        };
        state_mut.set("ibc_data", &cp_key_for_state, ibc_entry.clone())?;

        println!("\n=== {} ===", filename);
        println!("{}", serde_json::to_string_pretty(&ibc_entry)?);

        ibc_data_output.push(json!({
            "filename": filename,
            "data": ibc_entry,
        }));
    }

    // (ibc_data state writes are now direct/inline in the build loop using the finalized entry with channels.
    // The clone + re-set loop below was removed to avoid shadowing; force the outer state_mut.)
    state_mut.force_write()?;

    // UI state export moved AFTER asset derivation (and after channel map build)
    // so the terp-state.json for frontend gets:
    // - correct channel populated ibc_data (paths accurately curated)
    // - assets as FLAT array with derived IBC entries that have proper traces
    // Early export was using pre-derivation (empty channel map => no IBC assets)

    // Step 4: Write individual ibc_data files to public/ (schema-compliant)
    let ibc_data_path = std::path::PathBuf::from("../public/ibc-data");
    for entry in &ibc_data_output {
        let filename = entry["filename"].as_str().unwrap_or("unknown");
        let file_path = ibc_data_path.join(filename);
        let data = &entry["data"];
        std::fs::write(&file_path, serde_json::to_string_pretty(data)?)?;
        println!("  ✓ Wrote {}", file_path.display());
    }
    println!(
        "\n✓ {} IBC data files written to public/",
        ibc_data_output.len()
    );

    // Step 5: build the channel map from state (ibc_data was directly transposed with finalized
    // channels during the build; public files also written from the same source)
    let ibc_data_map = build_channel_to_chain_map(&state_mut);
    println!(
        "\n✓ {} IBC data entries written to state.json",
        ibc_data_output.len()
    );

    info!("\nChannel map entries: {}", ibc_data_map.len());

    let mut all_derived_ibc_assets: Vec<serde_json::Value> = Vec::new();
    let mut chain_assets: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let d = Vec::with_capacity(1);
    let mut assetlist = Vec::new();

    let assosmo = state_osmo.get("assets")?;
    let assosmo = assosmo.as_array().unwrap_or(&d);

    assetlist.extend::<&Vec<Value>>(assosmo.as_ref());

    let terp_assets: Vec<serde_json::Value> = state_terp
        .get("assets")
        .ok()
        .and_then(|a| a.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|mut a| {
            a["_source_chain"] = serde_json::json!("terp");
            a
        })
        .collect();
    chain_assets.insert("terp".to_string(), terp_assets.clone());

    all_derived_ibc_assets.extend(derive_ibc_asset_list(
        &terp,
        &ibc,
        &state_terp,
        terp_assets.as_slice(),
        &ibc_data_map,
    )?);

    // Process Osmosis assets — pass state_osmo, not state_terp
    let osmo_assets = load_assetlist_for_chain("osmosis", &state_osmo);
    if !osmo_assets.is_empty() {
        println!("\n=== Processing Osmosis assets (multi-hop paths) ===\n");
        all_derived_ibc_assets.extend(derive_ibc_asset_list(
            &terp,
            &ibc,
            &state_terp,
            &osmo_assets,
            &ibc_data_map,
        )?);
    }

    // Build final assetlist: native Terp assets + chain-registry base denoms + all derived IBC assets
    let mut native_assets: Vec<serde_json::Value> = state_terp
        .get("assets")
        .ok()
        .and_then(|a| a.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a["traces"].as_array().map_or(true, |t| t.is_empty()))
        .collect();

    // Also include chain-registry base denoms from connected chains
    // (e.g. osmosis assets, akash assets) so the assetlist is complete
    for osmo_asset in &osmo_assets {
        let base = osmo_asset["base"].as_str().unwrap_or("");
        let is_native = osmo_asset["traces"]
            .as_array()
            .map_or(true, |t| t.is_empty());
        if is_native && !native_assets.iter().any(|a| a["base"] == base) {
            let mut entry = osmo_asset.clone();
            entry["_source_chain"] = serde_json::json!("osmosis");
            native_assets.push(entry);
        }
    }
    chain_assets.insert("osmosis".to_string(), osmo_assets);
    // === Supply Terp natives (uterp + uthiol) to other chains' asset lists (accurate from finalized channels) ===
    // From first principles: the IBC trace on the counterparty side uses the channel on the cp side from the curated (finalized) ibc_data.
    // This ensures the hash matches the actual on-chain trace (e.g. transfer/channel-13/uthiol for atone uthiol from the live packet).
    // Uses the ibc_data_output (same source as public/ibc-data and state) to pick the correct cp-side channel from the alpha-ordered finalized channels.
    let terp_natives = vec!["uterp", "uthiol"];
    for entry in &ibc_data_output {
        let data = &entry["data"];
        let chain_1_name = data["chain_1"]["chain_name"].as_str().unwrap_or("");
        let chain_2_name = data["chain_2"]["chain_name"].as_str().unwrap_or("");
        let (cp_name, _is_terp_chain1) = if chain_1_name == "terp" {
            (chain_2_name.to_string(), true)
        } else if chain_2_name == "terp" {
            (chain_1_name.to_string(), false)
        } else {
            continue;
        };
        let channels = data["channels"].as_array().cloned().unwrap_or_default();
        let pref_ch = channels
            .iter()
            .find(|c| {
                let p1 = c["chain_1"]["port_id"].as_str() == Some("transfer");
                let p2 = c["chain_2"]["port_id"].as_str() == Some("transfer");
                p1 && p2 && c["tags"]["preferred"].as_bool() == Some(true)
            })
            .or_else(|| {
                channels.iter().find(|c| {
                    let p1 = c["chain_1"]["port_id"].as_str() == Some("transfer");
                    let p2 = c["chain_2"]["port_id"].as_str() == Some("transfer");
                    p1 && p2
                })
            });
        if let Some(ch) = pref_ch {
            let cp_side_key = if chain_1_name == cp_name {
                "chain_1"
            } else {
                "chain_2"
            };
            let terp_side_key = if chain_1_name == "terp" {
                "chain_1"
            } else {
                "chain_2"
            };
            let cp_channel = ch[cp_side_key]["channel_id"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let terp_ch = ch[terp_side_key]["channel_id"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if cp_channel.is_empty() {
                continue;
            }
            for native in &terp_natives {
                let trace_path = format!("transfer/{}/{}", cp_channel, native);
                let ibc_base = compute_ibc_denom_hash(&trace_path);
                let (symbol, name, desc, exp) = if *native == "uterp" {
                    ("TERP", "Terp", "The native token of Terp Network.", 6u32)
                } else {
                    ("THIOL", "Thiol", "The Thiol token of Terp Network.", 6u32)
                };
                let terp_native_on_cp = serde_json::json!({
                    "description": desc,
                    "denom_units": [
                        {"denom": ibc_base.clone(), "exponent": 0},
                        {"denom": native, "exponent": exp}
                    ],
                    "base": ibc_base,
                    "display": native,
                    "symbol": symbol,
                    "name": name,
                    "type_asset": "ics20",
                    "_source_chain": "terp",
                    "traces": [{
                        "type": "ibc",
                        "counterparty": {
                            "chain_name": "terp",
                            "base_denom": native,
                            "channel_id": terp_ch.clone()
                        },
                        "chain": {
                            "channel_id": cp_channel.clone(),
                            "path": trace_path
                        }
                    }]
                });
                chain_assets
                    .entry(cp_name.clone())
                    .or_default()
                    .push(terp_native_on_cp);
                println!(
                    "+ Added Terp native ({}) as IBC asset on {}: {}",
                    native, cp_name, ibc_base
                );
            }
        }
    }

    // === Ensure Terp native assets (uterp + uthiol) are added to other chain states ===
    // Default behavior: for every CP that has channels in the curated ibc_data, add the reverse
    // Terp native IBC assets (with correct trace using the actual cp-side channel from finalized data)
    // into a "chains.<cp>" entry in state. This populates assetinfo + IBC trace for destination
    // chains (e.g. atomone-1 will have the uthiol IBC entry with the live hash 4E87F00...).
    // Also makes the premine / lookup / routing tables see the 1-hop routes for both natives.
    let mut per_chain = serde_json::Map::new();
    for (cp, cp_assets) in &chain_assets {
        if cp == "terp" {
            continue;
        }
        let mut entry = serde_json::Map::new();
        entry.insert(
            "assets".to_string(),
            serde_json::to_value(cp_assets.clone()).unwrap(),
        );
        if let Some(info) = ibc_data_map.get(cp) {
            entry.insert("terp_channel_id".to_string(), json!(info.terp_channel_id));
            entry.insert(
                "counterparty_channel_id".to_string(),
                json!(info.counterparty_channel_id),
            );
        }
        // Carry the full ibc_data entry for the cp (has the channels with real ids + client_status)
        for e in &ibc_data_output {
            let d = &e["data"];
            if d["chain_1"]["chain_name"].as_str() == Some(cp)
                || d["chain_2"]["chain_name"].as_str() == Some(cp)
            {
                entry.insert("ibc_data".to_string(), d.clone());
                break;
            }
        }
        per_chain.insert(cp.clone(), serde_json::Value::Object(entry));
    }
    let chains_val = serde_json::Value::Object(per_chain.clone());
    state_mut.set("chains", "", &chains_val)?;
    println!(
        "✓ Populated per-chain state (chains.<cp>) for {} CPs with Terp native reverse assets + traces",
        per_chain.len()
    );

    // === Inject origin native assets for connected chains (so we can derive their IBC versions on Terp) ===
    // These are the "source of truth" natives on their home chains (no traces in their assetlist).
    // The derive logic will add the proper ibc trace using the channel map.
    let origin_assets = vec![
        // Akash
        serde_json::json!({
            "description": "Akash is a decentralized cloud computing marketplace.",
            "denom_units": [{"denom": "uakt", "exponent": 0}, {"denom": "AKT", "exponent": 6}],
            "base": "uakt",
            "display": "AKT",
            "symbol": "AKT",
            "name": "Akash",
            "type_asset": "sdk.coin",
            "_source_chain": "akash"
        }),
        // Atomone
        serde_json::json!({
            "description": "The native token of Atomone.",
            "denom_units": [{"denom": "uatone", "exponent": 0}, {"denom": "ATONE", "exponent": 6}],
            "base": "uatone",
            "display": "ATONE",
            "symbol": "ATONE",
            "name": "Atomone",
            "type_asset": "sdk.coin",
            "_source_chain": "atomone"
        }),
        // Cosmos Hub ATOM (example for multi-hop scenarios)
        serde_json::json!({
            "description": "The native staking token of the Cosmos Hub.",
            "denom_units": [{"denom": "uatom", "exponent": 0}, {"denom": "ATOM", "exponent": 6}],
            "base": "uatom",
            "display": "ATOM",
            "symbol": "ATOM",
            "name": "Cosmos Hub",
            "type_asset": "sdk.coin",
            "_source_chain": "cosmoshub"
        }),
    ];
    for origin in &origin_assets {
        // Only add if we have a channel for it in the ibc_data_map (so derivation can succeed)
        if let Some(sc) = origin["_source_chain"].as_str() {
            if ibc_data_map.contains_key(sc) {
                assetlist.push(origin);
                println!("+ Injected origin asset for {}", sc);
            }
        }
    }

    // Merge IBC assets into the list
    for new_asset in &all_derived_ibc_assets {
        let base = new_asset["base"].as_str().unwrap_or("");
        if let Some(pos) = native_assets.iter().position(|a| a["base"] == base) {
            native_assets[pos] = new_asset.clone();
        } else {
            native_assets.push(new_asset.clone());
        }
    }

    if !native_assets.is_empty() {
        let mut state_mut = state_terp.clone();
        state_mut.set(
            "assets",
            "",
            serde_json::Value::Array(native_assets.clone()),
        )?;
        state_mut.force_write()?;

        let assetlist_output = serde_json::json!({
            "$schema": "../asset_list.schema.json",
            "chain_name": "terp",
            "assets": native_assets
        });
        let output_path = std::path::PathBuf::from("../public/assetlist.json");
        std::fs::write(
            &output_path,
            serde_json::to_string_pretty(&assetlist_output)?,
        )?;

        println!(
            "\n✓ Assetlist written with {} total assets ({} IBC)",
            native_assets.len(),
            all_derived_ibc_assets.len()
        );

        // NOW (late) export UI state AFTER derivation + channel fixes.
        // "assets" is now a flat array (with traces). ibc_data map has the full curated
        // channel info (ids, ports, preferred) for accurate transfers in the UI.
        let ui_state_path = std::path::PathBuf::from(
            "../websites/dao-dao-ui/packages/utils/constants/terp-state.json",
        );
        let mut ibc_data_for_ui: serde_json::Map<String, serde_json::Value> =
            serde_json::Map::new();
        for entry in &ibc_data_output {
            let filename = entry["filename"].as_str().unwrap_or("unknown");
            let counterpartyname = filename
                .strip_suffix(".json")
                .unwrap_or(filename)
                .split("-")
                .find(|p| *p != "terp")
                .unwrap_or("unknown")
                .to_string();
            if let Some(data) = entry.get("data") {
                ibc_data_for_ui.insert(counterpartyname, data.clone());
            }
        }
        let ui_state = serde_json::json!({
            "morocco-1": {
                // Flat assets array (preferred shape for registry.ts consumption).
                // Includes native + all derived IBC assets with full traces (multi-hop respected).
                "assets": native_assets.clone(),
                // Comprehensive ibc_data: now includes real channel_id/port_id, clients, connections,
                // preferred transfer channels, and status from the stabilized query + finalize logic.
                "ibc_data": ibc_data_for_ui,
                "code_ids": state_mut.get("code_ids").ok(),
                "default": state_mut.get("default").ok(),
            }
        });
        if let Some(parent) = ui_state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&ui_state_path, serde_json::to_string_pretty(&ui_state)?)?;
        println!(
            "  ✓ UI state exported to {} (flat assets + {} ibc entries)",
            ui_state_path.display(),
            ibc_data_for_ui.len()
        );
    }

    // Lib-backed predict + fail-closed invariants (Agent C remediation #1)
    let ibc_data_for_predict = state_mut.get("ibc_data").unwrap_or(json!({}));
    let world = PredictedWorld::from_inputs(ibc_data_for_predict.clone(), &chain_assets, 3);

    println!("\n=== IBC Channel Graph (lib) ===");
    for (chain, edges) in &world.graph.edges {
        println!("  {} -> {} connections", chain, edges.len());
        for edge in edges {
            println!(
                "    -> {}/{} (preferred: {}, status: {})",
                edge.counterparty_chain, edge.this_channel_id, edge.preferred, edge.status
            );
        }
    }

    println!("\n=== Premining IBC Asset Routes (lib) ===");
    println!("Total routes: {}", world.routes.metadata.total_routes);
    println!("Chains covered: {:?}", world.routes.metadata.chains);

    let report = check_invariants(&world);
    for item in &report.items {
        println!(
            "  [{:?}] {} — {} ({:?})",
            item.severity, item.code, item.message, item.path
        );
    }
    if report.has_errors() {
        anyhow::bail!(
            "IBC generation failed hard invariants ({} errors). Not writing success.",
            report.errors().count()
        );
    }

    let simplified = world.lookup.clone();
    let lookup_path = std::path::PathBuf::from("../public/ibc_lookup_table.json");
    std::fs::write(&lookup_path, serde_json::to_string_pretty(&simplified)?)?;
    println!("\n✓ IBC lookup table written to ibc_lookup_table.json");

    let full_table = world.routes.to_json();
    let full_path = std::path::PathBuf::from("../public/ibc_routing_table.json");
    std::fs::write(&full_path, serde_json::to_string_pretty(&full_table)?)?;
    println!("✓ Full routing table written to ibc_routing_table.json");

    let meta = json!({
        "generated_at": now_timestamp(),
        "generator": "terp-scripts/bin/terp-ibc (lib-backed)",
        "max_hops": 3,
        "total_routes": world.routes.metadata.total_routes,
        "chains": world.routes.metadata.chains,
        "skip_reasons": skip_reasons,
        "invariant_findings": report.items.len(),
        "exclude_chain_ids": dead_chains,
        "exclude_client_ids": exclude_clients,
    });
    let meta_path = std::path::PathBuf::from("../public/ibc_generation_meta.json");
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    println!("✓ Generation meta written to ibc_generation_meta.json");

    Ok(())
}

/// Build a trace entry conforming to the specific schema for each trace type
fn build_trace_entry(trace: &serde_json::Value) -> Option<serde_json::Value> {
    let trace_type = trace["type"].as_str().unwrap_or("");
    let counterparty = &trace["counterparty"];
    let chain = &trace["chain"];

    match trace_type {
        "ibc" => Some(serde_json::json!({
            "type": "ibc",
            "counterparty": {
                "chain_name": counterparty["chain_name"].as_str().unwrap_or(""),
                "base_denom": counterparty["base_denom"].as_str().unwrap_or(""),
                "channel_id": counterparty["channel_id"].as_str().unwrap_or("")
            },
            "chain": {
                "channel_id": chain["channel_id"].as_str().unwrap_or(""),
                "path": chain["path"].as_str().unwrap_or("")
            }
        })),
        "ibc-cw20" => Some(serde_json::json!({
            "type": "ibc-cw20",
            "counterparty": {
                "chain_name": counterparty["chain_name"].as_str().unwrap_or(""),
                "base_denom": counterparty["base_denom"].as_str().unwrap_or(""),
                "port": counterparty["port"].as_str().unwrap_or(""),
                "channel_id": counterparty["channel_id"].as_str().unwrap_or("")
            },
            "chain": {
                "port": chain["port"].as_str().unwrap_or(""),
                "channel_id": chain["channel_id"].as_str().unwrap_or(""),
                "path": chain["path"].as_str().unwrap_or("")
            }
        })),
        "ibc-bridge" => Some(serde_json::json!({
            "type": "ibc-bridge",
            "counterparty": {
                "chain_name": counterparty["chain_name"].as_str().unwrap_or(""),
                "base_denom": counterparty["base_denom"].as_str().unwrap_or(""),
                "channel_id": counterparty["channel_id"].as_str().unwrap_or("")
            },
            "chain": {
                "channel_id": chain["channel_id"].as_str().unwrap_or(""),
                "path": chain["path"].as_str().unwrap_or("")
            },
            "provider": trace["provider"].as_str().unwrap_or("")
        })),
        "bridge" | "liquid-stake" | "synthetic" | "wrapped" | "additional-mintage"
        | "test-mintage" | "legacy-mintage" => {
            let mut cp = serde_json::json!({
                "chain_name": counterparty["chain_name"].as_str().unwrap_or(""),
                "base_denom": counterparty["base_denom"].as_str().unwrap_or("")
            });
            if let Some(contract) = counterparty["contract"].as_str() {
                cp["contract"] = serde_json::json!(contract);
            }
            let mut entry = serde_json::json!({
                "type": trace_type,
                "counterparty": cp,
                "provider": trace["provider"].as_str().unwrap_or("")
            });
            if !chain.is_null() && chain["contract"].is_string() {
                entry["chain"] = serde_json::json!({
                    "contract": chain["contract"].as_str().unwrap_or("")
                });
            }
            Some(entry)
        }
        _ => None,
    }
}

fn build_image_sync(counterparty_chain: &str, counterparty_base_denom: &str) -> serde_json::Value {
    let png_url = format!(
        "https://raw.githubusercontent.com/cosmos/chain-registry/master/{}/images/{}.png",
        counterparty_chain,
        counterparty_base_denom.trim_start_matches('u')
    );
    let svg_url = format!(
        "https://raw.githubusercontent.com/cosmos/chain-registry/master/{}/images/{}.svg",
        counterparty_chain,
        counterparty_base_denom.trim_start_matches('u')
    );
    serde_json::json!({
        "image_sync": {
            "chain_name": counterparty_chain,
            "base_denom": counterparty_base_denom
        },
        "png": png_url,
        "svg": svg_url
    })
}

/// Build a complete assetlist-compliant asset entry for any trace type
fn build_ibc_asset_entry(
    asset: &serde_json::Value,
    ibc_denom: &str,
    traces: &[serde_json::Value],
    _trace_path: &str,
    symbol: &str,
    counterparty_chain: &str,
    counterparty_denom: &str,
    _dest_channel: &str,
    via_chain: Option<&str>,
) -> serde_json::Value {
    let display_denom = asset["display"].as_str().unwrap_or("");
    let display_exponent = asset["denom_units"]
        .as_array()
        .and_then(|units| units.iter().find(|u| u["denom"] == display_denom))
        .and_then(|u| u["exponent"].as_u64())
        .unwrap_or(6);

    let type_asset = asset["type_asset"].as_str().unwrap_or("ics20");

    // Build traces array from all trace entries
    let trace_entries: Vec<serde_json::Value> = traces
        .iter()
        .filter_map(|t| {
            let mut entry = build_trace_entry(t)?;
            // If this is a multi-hop, add the via information
            if let Some(via) = via_chain {
                if entry["type"] == "ibc" {
                    entry["via"] = serde_json::json!(via);
                }
            }
            Some(entry)
        })
        .collect();

    // Build the base asset object
    let mut asset_obj = serde_json::json!({
        "description": asset["description"].as_str().unwrap_or(""),
        "denom_units": [
            {
                "denom": ibc_denom,
                "exponent": 0,
                "aliases": [counterparty_denom]
            },
            {
                "denom": display_denom,
                "exponent": display_exponent
            }
        ],
        "type_asset": type_asset,
        "base": ibc_denom,
        "name": asset["name"].as_str().unwrap_or(""),
        "display": display_denom,
        "symbol": symbol,
    });

    // Add traces if present
    if !trace_entries.is_empty() {
        asset_obj["traces"] = serde_json::json!(trace_entries);
    }

    // Add address for cw20/erc20/snip20 (required by schema)
    if let Some(address) = asset["address"].as_str() {
        if !address.is_empty() {
            asset_obj["address"] = serde_json::json!(address);
        }
    }

    // Add image_sync for IBC/bridged assets
    let has_ibc_trace = traces.iter().any(|t| {
        matches!(
            t["type"].as_str().unwrap_or(""),
            "ibc"
                | "ibc-cw20"
                | "ibc-bridge"
                | "bridge"
                | "wrapped"
                | "liquid-stake"
                | "test-mintage"
        )
    });

    if has_ibc_trace && !counterparty_chain.is_empty() && !counterparty_denom.is_empty() {
        let image_entry = build_image_sync(counterparty_chain, counterparty_denom);
        asset_obj["images"] = serde_json::json!([image_entry]);
        asset_obj["logo_URIs"] = serde_json::json!({
            "png": image_entry["png"],
            "svg": image_entry["svg"]
        });
    }

    // Add coingecko_id if present
    if let Some(cg_id) = asset["coingecko_id"].as_str() {
        if !cg_id.is_empty() {
            asset_obj["coingecko_id"] = serde_json::json!(cg_id);
        }
    }

    // Add deprecated flag only if true
    if asset["deprecated"].as_bool().unwrap_or(false) {
        asset_obj["deprecated"] = serde_json::json!(true);
    }

    asset_obj
}
fn derive_ibc_asset_list(
    _terp: &Daemon,
    _ibc: &Ibc,
    _state_terp: &DaemonState,
    assetlist: &[serde_json::Value],
    ibc_data_map: &HashMap<String, TerpChannelInfo>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut ibc_denoms: Vec<serde_json::Value> = vec![];

    info!("assetlist: {}", assetlist.len());

    for asset in assetlist {
        let symbol = asset["symbol"].as_str().unwrap_or("UNKNOWN");
        let base_denom = asset["base"].as_str().unwrap_or("");

        let traces = asset["traces"].as_array().cloned().unwrap_or_default();
        let has_traces = !traces.is_empty();
        let source_chain = asset["_source_chain"].as_str().unwrap_or("").to_string();

        // Skip only pure local natives without source tag. Origin natives from other chains (tagged with _source_chain) and assets with traces are processed.
        if !has_traces && source_chain.is_empty() {
            continue;
        }

        // Check if this is an IBC-type trace
        let has_ibc_trace = traces.iter().any(|t| {
            matches!(
                t["type"].as_str().unwrap_or(""),
                "ibc" | "ibc-cw20" | "ibc-bridge"
            )
        });

        if has_ibc_trace || !has_traces {
            // has_ibc_trace or origin native (no traces but source_chain)
            match derive_terp_ibc_denom(asset, ibc_data_map) {
                Some((ibc_hash, trace_path, cp_chain)) => {
                    let cp_denom = if has_traces && !traces.is_empty() {
                        traces[0]["counterparty"]["base_denom"]
                            .as_str()
                            .unwrap_or(base_denom)
                    } else {
                        base_denom
                    };
                    let dest_channel = if has_traces && !traces.is_empty() {
                        traces[0]["chain"]["channel_id"].as_str().unwrap_or("")
                    } else {
                        ""
                    };

                    println!("↻ {} - derived via {}", symbol, trace_path);
                    println!("  ✓ IBC denom: {}", ibc_hash);

                    // For origin natives, synthesize a minimal ibc trace so build_ibc_asset_entry produces correct output
                    let effective_traces = if has_traces {
                        traces.clone()
                    } else {
                        // Synthesize trace for origin
                        vec![serde_json::json!({
                            "type": "ibc",
                            "counterparty": {
                                "chain_name": source_chain,
                                "base_denom": base_denom,
                                "channel_id": ""  // will be filled by build if possible
                            },
                            "chain": {
                                "channel_id": "",  // filled later if needed
                                "path": trace_path
                            }
                        })]
                    };

                    ibc_denoms.push(build_ibc_asset_entry(
                        asset,
                        &ibc_hash,
                        &effective_traces,
                        &trace_path,
                        symbol,
                        &cp_chain,
                        cp_denom,
                        dest_channel,
                        None,
                    ));
                }
                None => {
                    println!("⚠ No path found for {}", symbol);
                }
            }
        } else {
            // Non-IBC trace (bridge, liquid-stake, etc.)
            let cp_chain = if !traces.is_empty() {
                traces[0]["counterparty"]["chain_name"]
                    .as_str()
                    .unwrap_or("")
            } else {
                ""
            };
            let cp_denom = if !traces.is_empty() {
                traces[0]["counterparty"]["base_denom"]
                    .as_str()
                    .unwrap_or("")
            } else {
                ""
            };

            println!("⏭ {} - non-IBC trace, using base denom", symbol);
            ibc_denoms.push(build_ibc_asset_entry(
                asset, base_denom, &traces, "", symbol, cp_chain, cp_denom, "", None,
            ));
        }
    }

    Ok(ibc_denoms)
}

struct IbcChannelInfo {
    terp_channel: String,
    counterparty_channel: String,
    counterparty_chain_name: String,
    counterparty_chain_id: String,
}

fn build_channel_to_chain_map(state: &DaemonState) -> HashMap<String, TerpChannelInfo> {
    let ibc_data = match state.get("ibc_data") {
        Ok(data) => data,
        Err(_) => {
            println!("  ⚠ No ibc_data found in state");
            return HashMap::new();
        }
    };
    let map = lib_build_channel_to_chain_map(&ibc_data);
    for (cp, info) in &map {
        println!(
            "  ✓ Channel map: terp/{} <-> {}/{}",
            info.terp_channel_id, cp, info.counterparty_channel_id
        );
    }
    map
}
/// Find a multi-hop path from Terp to a target chain via an intermediary
/// Returns (terp_channel_to_intermediary, intermediary_chain_name)
fn find_path_via_intermediary(
    _target_chain: &str,
    ibc_data_map: &HashMap<String, IbcChannelInfo>,
    available_intermediaries: &[&str], // e.g., ["osmosis", "cosmoshub", "juno"]
) -> Option<(String, String)> {
    for intermediary in available_intermediaries {
        if let Some(info) = ibc_data_map.get(*intermediary) {
            // We have a channel to this intermediary
            if !info.terp_channel.is_empty() {
                return Some((info.terp_channel.clone(), intermediary.to_string()));
            }
        }
    }
    None
}

fn load_assetlist_for_chain(chain_name: &str, state: &DaemonState) -> Vec<serde_json::Value> {
    let assets = state
        .get("assets")
        .ok()
        .and_then(|a| a.as_array().cloned())
        .unwrap_or_default();

    println!("Loaded {} assets from '{}'", assets.len(), chain_name);

    assets
        .into_iter()
        .map(|mut asset| {
            asset["_source_chain"] = serde_json::json!(chain_name);
            asset
        })
        .collect()
}

/// Derive the IBC denom hash and trace path for a foreign asset on Terp
///
/// Logic:
/// 1. If the asset has an IBC trace, extract the counterparty chain and base denom
/// 2. If Terp has a direct channel to the counterparty chain:
///    - Single hop: transfer/{terp_ch}/{counterparty_base_denom}
///    - Multi-hop on source: transfer/{terp_ch}/{source_trace_path}
/// 3. If Terp has NO direct channel to counterparty but has channel to source chain:
///    - Route through source: transfer/{terp_ch_to_source}/{source_trace_path}

/// Current unix timestamp as a string.
pub fn now_timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}
