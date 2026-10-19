use serde::Serialize;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

// ── Field Descriptor ────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug)]
pub struct Fd {
    pub category: &'static str,
    pub key: &'static str,
    pub env_var: &'static str,
    pub prompt: &'static str,
    pub default: &'static str,
    pub secret: bool,
}

// ── Phase Info ──────────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug)]
pub struct PhaseInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub file: &'static str,
    pub description: &'static str,
    /// Field descriptor categories relevant to this phase.
    pub categories: Vec<&'static str>,
}

// ── Template Substitution ───────────────────────────────────────────
// Exact port of o-line/src/config.rs::substitute_template_raw

fn substitute_template_raw(
    template: &str,
    variables: &HashMap<String, String>,
) -> Result<String, String> {
    let mut result = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            i += 2;
            let start = i;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            if i >= chars.len() {
                return Err("Unclosed ${...} placeholder in template".into());
            }
            let var_name: String = chars[start..i].iter().collect();
            match variables.get(&var_name) {
                Some(val) => result.push_str(val),
                None => return Err(format!("Variable '{}' has no value", var_name)),
            }
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    Ok(result)
}

/// Extract all ${VAR} references from a template string.
fn extract_vars(template: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            i += 2;
            let start = i;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            if i < chars.len() {
                let var_name: String = chars[start..i].iter().collect();
                if !vars.contains(&var_name) {
                    vars.push(var_name);
                }
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    vars
}

// ── Static Data ─────────────────────────────────────────────────────
// Mirrors o-line/src/lib.rs FIELD_DESCRIPTORS — same order, same defaults.

const ALL_FDS: &[Fd] = &[
    // default
    Fd {
        category: "default",
        key: "omnibus_image",
        env_var: "OMNIBUS_IMAGE",
        prompt: "Omnibus Image",
        default: "ghcr.io/akash-network/cosmos-omnibus:v1.2.38-generic",
        secret: false,
    },
    Fd {
        category: "default",
        key: "sdl_dir",
        env_var: "SDL_DIR",
        prompt: "SDL Templates folder",
        default: "~/.oline/templates/sdls/oline",
        secret: false,
    },
    Fd {
        category: "default",
        key: "binary",
        env_var: "OLINE_BINARY",
        prompt: "Cosmos daemon binary name",
        default: "terpd",
        secret: false,
    },
    // chain
    Fd {
        category: "chain",
        key: "chain_id",
        env_var: "OLINE_CHAIN_ID",
        prompt: "Chain ID",
        default: "morocco-1",
        secret: false,
    },
    Fd {
        category: "chain",
        key: "chain_json",
        env_var: "OLINE_CHAIN_JSON",
        prompt: "Chain JSON URL",
        default: "https://raw.githubusercontent.com/permissionlessweb/chain-registry/refs/heads/terpnetwork%40v5.0.4/akash/chain.json",
        secret: false,
    },
    Fd {
        category: "chain",
        key: "addrbook_url",
        env_var: "OLINE_ADDRBOOK_URL",
        prompt: "Address book URL",
        default: "https://raw.githubusercontent.com/111STAVR111/props/main/Terp/addrbook.json",
        secret: false,
    },
    Fd {
        category: "chain",
        key: "persistent_peers",
        env_var: "OLINE_PERSISTENT_PEERS",
        prompt: "Default persistent peers",
        default: "5bf887027701d3b8c4d95c0ba898cc8bf6d166ff@188.165.194.110:26676,a81dc3bf1bb1c3837b768eeb82659eecc971890b@terp-mainnet-peer.itrocket.net:13656",
        secret: false,
    },
    // network
    Fd {
        category: "network",
        key: "rpc_endpoint",
        env_var: "OLINE_RPC_ENDPOINT",
        prompt: "RPC endpoint",
        default: "https://rpc-akash.ecostake.com:443",
        secret: false,
    },
    Fd {
        category: "network",
        key: "grpc_endpoint",
        env_var: "OLINE_GRPC_ENDPOINT",
        prompt: "gRPC endpoint",
        default: "https://akash.lavenderfive.com:443",
        secret: false,
    },
    Fd {
        category: "network",
        key: "rest_endpoint",
        env_var: "OLINE_REST_ENDPOINT",
        prompt: "REST endpoint",
        default: "https://api.akashnet.net:443",
        secret: false,
    },
    // validator
    Fd {
        category: "validator",
        key: "peer_id",
        env_var: "OLINE_VALIDATOR_PEER_ID",
        prompt: "Private validator peer id",
        default: "",
        secret: false,
    },
    // cloudflare
    Fd {
        category: "cloudflare",
        key: "api_token",
        env_var: "OLINE_CF_API_TOKEN",
        prompt: "Cloudflare API token",
        default: "",
        secret: true,
    },
    Fd {
        category: "cloudflare",
        key: "zone_id",
        env_var: "OLINE_CF_ZONE_ID",
        prompt: "Cloudflare zone ID",
        default: "",
        secret: false,
    },
    Fd {
        category: "cloudflare",
        key: "tls_config_url",
        env_var: "TLS_CONFIG_URL",
        prompt: "TLS config URL",
        default: "https://raw.githubusercontent.com/permissionlessweb/o-line/refs/heads/master/plays/audible/tls-setup.sh",
        secret: false,
    },
    Fd {
        category: "cloudflare",
        key: "entrypoint_url",
        env_var: "ENTRYPOINT_URL",
        prompt: "Entrypoint URL",
        default: "https://raw.githubusercontent.com/permissionlessweb/o-line/refs/heads/master/plays/audible/oline-entrypoint.sh",
        secret: false,
    },
    Fd {
        category: "network",
        key: "ssh_port",
        env_var: "SSH_PORT",
        prompt: "SSH/SFTP port",
        default: "22",
        secret: false,
    },
    // snapshot
    Fd {
        category: "snapshot",
        key: "path",
        env_var: "OLINE_SNAPSHOT_PATH",
        prompt: "S3 snapshot path",
        default: "snapshots/terpnetwork",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "time",
        env_var: "OLINE_SNAPSHOT_TIME",
        prompt: "Snapshot schedule time",
        default: "00:00:00",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "state_url",
        env_var: "OLINE_SNAPSHOT_STATE_URL",
        prompt: "Snapshot state url",
        default: "https://server-4.itrocket.net/mainnet/terp/.current_state.json",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "base_url",
        env_var: "OLINE_SNAPSHOT_BASE_URL",
        prompt: "Public snapshot URL",
        default: "https://server-4.itrocket.net/mainnet/terp/",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "save_format",
        env_var: "OLINE_SNAPSHOT_SAVE_FORMAT",
        prompt: "Snapshot save format",
        default: "tar.gz",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "retain",
        env_var: "OLINE_SNAPSHOT_RETAIN",
        prompt: "Snapshot retention period",
        default: "2 days",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "keep_last",
        env_var: "OLINE_SNAPSHOT_KEEP_LAST",
        prompt: "Minimum snapshots to keep",
        default: "2",
        secret: false,
    },
    Fd {
        category: "snapshot",
        key: "download_domain",
        env_var: "OLINE_SNAPSHOT_DOWNLOAD_DOMAIN",
        prompt: "Private snapshot domain",
        default: "",
        secret: false,
    },
    // minio
    Fd {
        category: "minio",
        key: "image",
        env_var: "MINIO_IPFS_IMAGE",
        prompt: "MinIO-IPFS image",
        default: "minio/minio:latest",
        secret: false,
    },
    Fd {
        category: "minio",
        key: "autopin_interval",
        env_var: "OLINE_AUTOPIN_INTERVAL",
        prompt: "IPFS auto-pin interval",
        default: "300",
        secret: false,
    },
    // special_teams — snapshot
    Fd {
        category: "special_teams",
        key: "snapshot_rpc_domain",
        env_var: "RPC_DOMAIN_SNAPSHOT",
        prompt: "Snapshot RPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_rpc_port",
        env_var: "RPC_PORT_SNAPSHOT",
        prompt: "Snapshot RPC port",
        default: "26657",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_p2p_domain",
        env_var: "P2P_DOMAIN_SNAPSHOT",
        prompt: "Snapshot P2P domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_p2p_port",
        env_var: "P2P_PORT_SNAPSHOT",
        prompt: "Snapshot P2P port",
        default: "26656",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_api_domain",
        env_var: "API_DOMAIN_SNAPSHOT",
        prompt: "Snapshot API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_api_port",
        env_var: "API_PORT_SNAPSHOT",
        prompt: "Snapshot API port",
        default: "1317",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_grpc_domain",
        env_var: "GRPC_DOMAIN_SNAPSHOT",
        prompt: "Snapshot gRPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "snapshot_grpc_port",
        env_var: "GRPC_PORT_SNAPSHOT",
        prompt: "Snapshot gRPC port",
        default: "9090",
        secret: false,
    },
    // special_teams — seed
    Fd {
        category: "special_teams",
        key: "seed_rpc_domain",
        env_var: "RPC_DOMAIN_SEED",
        prompt: "Seed RPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_rpc_port",
        env_var: "RPC_PORT_SEED",
        prompt: "Seed RPC port",
        default: "26657",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_p2p_domain",
        env_var: "P2P_DOMAIN_SEED",
        prompt: "Seed P2P domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_p2p_port",
        env_var: "P2P_PORT_SEED",
        prompt: "Seed P2P port",
        default: "26656",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_api_domain",
        env_var: "API_DOMAIN_SEED",
        prompt: "Seed API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_api_port",
        env_var: "API_PORT_SEED",
        prompt: "Seed API port",
        default: "1317",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_grpc_domain",
        env_var: "GRPC_DOMAIN_SEED",
        prompt: "Seed gRPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "special_teams",
        key: "seed_grpc_port",
        env_var: "GRPC_PORT_SEED",
        prompt: "Seed gRPC port",
        default: "9090",
        secret: false,
    },
    // tackles — left
    Fd {
        category: "tackles",
        key: "left_p2p_port",
        env_var: "P2P_PORT_TACKLE_L",
        prompt: "Left tackle P2P port",
        default: "26656",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_rpc_port",
        env_var: "RPC_PORT_TACKLE_L",
        prompt: "Left tackle RPC port",
        default: "26657",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_api_port",
        env_var: "API_PORT_TACKLE_L",
        prompt: "Left tackle API port",
        default: "1317",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_grpc_port",
        env_var: "GRPC_PORT_TACKLE_L",
        prompt: "Left tackle gRPC port",
        default: "9090",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_rpc_domain",
        env_var: "RPC_DOMAIN_TACKLE_L",
        prompt: "Left tackle RPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_api_domain",
        env_var: "API_DOMAIN_TACKLE_L",
        prompt: "Left tackle API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_p2p_domain",
        env_var: "P2P_DOMAIN_TACKLE_L",
        prompt: "Left tackle P2P domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "left_grpc_domain",
        env_var: "GRPC_DOMAIN_TACKLE_L",
        prompt: "Left tackle gRPC domain",
        default: "",
        secret: false,
    },
    // tackles — right
    Fd {
        category: "tackles",
        key: "right_p2p_port",
        env_var: "P2P_PORT_TACKLE_R",
        prompt: "Right tackle P2P port",
        default: "26656",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_rpc_port",
        env_var: "RPC_PORT_TACKLE_R",
        prompt: "Right tackle RPC port",
        default: "26657",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_api_port",
        env_var: "API_PORT_TACKLE_R",
        prompt: "Right tackle API port",
        default: "1317",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_grpc_port",
        env_var: "GRPC_PORT_TACKLE_R",
        prompt: "Right tackle gRPC port",
        default: "9090",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_rpc_domain",
        env_var: "RPC_DOMAIN_TACKLE_R",
        prompt: "Right tackle RPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_api_domain",
        env_var: "API_DOMAIN_TACKLE_R",
        prompt: "Right tackle API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_p2p_domain",
        env_var: "P2P_DOMAIN_TACKLE_R",
        prompt: "Right tackle P2P domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "tackles",
        key: "right_grpc_domain",
        env_var: "GRPC_DOMAIN_TACKLE_R",
        prompt: "Right tackle gRPC domain",
        default: "",
        secret: false,
    },
    // forwards — left
    Fd {
        category: "forwards",
        key: "left_p2p_port",
        env_var: "P2P_PORT_FORWARD_L",
        prompt: "Left forward P2P port",
        default: "26656",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_rpc_port",
        env_var: "RPC_PORT_FORWARD_L",
        prompt: "Left forward RPC port",
        default: "26657",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_api_port",
        env_var: "API_PORT_FORWARD_L",
        prompt: "Left forward API port",
        default: "1317",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_grpc_port",
        env_var: "GRPC_PORT_FORWARD_L",
        prompt: "Left forward gRPC port",
        default: "9090",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_rpc_domain",
        env_var: "RPC_DOMAIN_FORWARD_L",
        prompt: "Left forward RPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_api_domain",
        env_var: "API_DOMAIN_FORWARD_L",
        prompt: "Left forward API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_p2p_domain",
        env_var: "P2P_DOMAIN_FORWARD_L",
        prompt: "Left forward P2P domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "left_grpc_domain",
        env_var: "GRPC_DOMAIN_FORWARD_L",
        prompt: "Left forward gRPC domain",
        default: "",
        secret: false,
    },
    // forwards — right
    Fd {
        category: "forwards",
        key: "right_p2p_port",
        env_var: "P2P_PORT_FORWARD_R",
        prompt: "Right forward P2P port",
        default: "26656",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_rpc_port",
        env_var: "RPC_PORT_FORWARD_R",
        prompt: "Right forward RPC port",
        default: "26657",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_api_port",
        env_var: "API_PORT_FORWARD_R",
        prompt: "Right forward API port",
        default: "1317",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_grpc_port",
        env_var: "GRPC_PORT_FORWARD_R",
        prompt: "Right forward gRPC port",
        default: "9090",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_rpc_domain",
        env_var: "RPC_DOMAIN_FORWARD_R",
        prompt: "Right forward RPC domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_api_domain",
        env_var: "API_DOMAIN_FORWARD_R",
        prompt: "Right forward API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_p2p_domain",
        env_var: "P2P_DOMAIN_FORWARD_R",
        prompt: "Right forward P2P domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "forwards",
        key: "right_grpc_domain",
        env_var: "GRPC_DOMAIN_FORWARD_R",
        prompt: "Right forward gRPC domain",
        default: "",
        secret: false,
    },
    // relayer
    Fd {
        category: "relayer",
        key: "image",
        env_var: "RLY_IMAGE",
        prompt: "Relayer Docker image",
        default: "ghcr.io/permissionlessweb/rly-docker:latest",
        secret: false,
    },
    Fd {
        category: "relayer",
        key: "key_name",
        env_var: "RLY_KEY_NAME",
        prompt: "Relayer key name",
        default: "relayer_key",
        secret: false,
    },
    Fd {
        category: "relayer",
        key: "remote_chain_id",
        env_var: "RLY_REMOTE_CHAIN_ID",
        prompt: "Remote chain ID",
        default: "",
        secret: false,
    },
    Fd {
        category: "relayer",
        key: "api_domain",
        env_var: "RLY_API_DOMAIN",
        prompt: "Relayer API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "relayer",
        key: "key_terp",
        env_var: "RLY_KEY_TERP",
        prompt: "Terp relayer mnemonic",
        default: "",
        secret: true,
    },
    Fd {
        category: "relayer",
        key: "key_remote",
        env_var: "RLY_KEY_REMOTE",
        prompt: "Remote relayer mnemonic",
        default: "",
        secret: true,
    },
    Fd {
        category: "relayer",
        key: "entrypoint_url",
        env_var: "RELAYER_ENTRYPOINT",
        prompt: "Relayer entrypoint URL",
        default: "",
        secret: false,
    },
    // argus
    Fd {
        category: "argus",
        key: "node_moniker",
        env_var: "ARGUS_NODE_MONIKER",
        prompt: "Argus node moniker",
        default: "",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "node_seeds",
        env_var: "ARGUS_NODE_SEEDS",
        prompt: "Argus node seeds",
        default: "",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "node_peers",
        env_var: "ARGUS_NODE_PERSISTENT_PEERS",
        prompt: "Argus node peers",
        default: "",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "image",
        env_var: "ARGUS_IMAGE",
        prompt: "Argus Docker image",
        default: "ghcr.io/permissionlessweb/argus:latest",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "entrypoint_url",
        env_var: "ARGUS_ENTRYPOINT_URL",
        prompt: "Argus entrypoint URL",
        default: "",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "api_domain",
        env_var: "ARGUS_API_DOMAIN",
        prompt: "Argus API domain",
        default: "",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "bech32_prefix",
        env_var: "ARGUS_BECH32_PREFIX",
        prompt: "Chain bech32 prefix",
        default: "terp",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "db_user",
        env_var: "ARGUS_DB_USER",
        prompt: "PostgreSQL username",
        default: "argus",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "db_password",
        env_var: "ARGUS_DB_PASSWORD",
        prompt: "PostgreSQL password",
        default: "",
        secret: true,
    },
    Fd {
        category: "argus",
        key: "db_data_name",
        env_var: "ARGUS_DB_DATA_NAME",
        prompt: "PostgreSQL data DB",
        default: "argus_data",
        secret: false,
    },
    Fd {
        category: "argus",
        key: "db_accounts_name",
        env_var: "ARGUS_DB_ACCOUNTS_NAME",
        prompt: "PostgreSQL accounts DB",
        default: "argus_accounts",
        secret: false,
    },
];

// PhaseInfo can't use vec![] in const context, so we build them at runtime.
fn phase_infos() -> Vec<PhaseInfo> {
    vec![
        PhaseInfo {
            id: "a",
            label: "Phase A: Special Teams",
            file: "a.yml",
            description: "Snapshot + Seed + MinIO nodes",
            categories: vec![
                "default",
                "chain",
                "network",
                "cloudflare",
                "snapshot",
                "minio",
                "special_teams",
            ],
        },
        PhaseInfo {
            id: "b",
            label: "Phase B: Tackles",
            file: "b.yml",
            description: "Left & Right Tackle nodes",
            categories: vec!["default", "chain", "network", "cloudflare", "tackles"],
        },
        PhaseInfo {
            id: "c",
            label: "Phase C: Forwards",
            file: "c.yml",
            description: "Left & Right Forward nodes",
            categories: vec!["default", "chain", "network", "cloudflare", "forwards"],
        },
        PhaseInfo {
            id: "e",
            label: "Phase E: Relayer",
            file: "e.yml",
            description: "IBC Relayer service",
            categories: vec!["relayer"],
        },
        PhaseInfo {
            id: "f",
            label: "Phase F: Indexer",
            file: "f.yml",
            description: "Argus Indexer + PostgreSQL",
            categories: vec!["default", "chain", "network", "argus"],
        },
    ]
}

// ── WASM Exports ────────────────────────────────────────────────────

/// Return all field descriptors as a JSON array.
/// Each entry: { category, key, env_var, prompt, default, secret }
#[wasm_bindgen]
pub fn get_field_descriptors() -> Result<JsValue, JsError> {
    serde_wasm_bindgen::to_value(ALL_FDS).map_err(|e| JsError::new(&e.to_string()))
}

/// Return phase definitions as a JSON array.
#[wasm_bindgen]
pub fn get_phases() -> Result<JsValue, JsError> {
    let phases = phase_infos();
    serde_wasm_bindgen::to_value(&phases).map_err(|e| JsError::new(&e.to_string()))
}

/// Substitute ${VAR} placeholders in a template using the provided variables.
/// `vars_json` is a JSON object: { "VAR_NAME": "value", ... }
///
/// This is the exact algorithm used by the o-line CLI (`oline sdl`).
#[wasm_bindgen]
pub fn substitute_template(template: &str, vars_json: &str) -> Result<String, JsError> {
    let vars: HashMap<String, String> = serde_json::from_str(vars_json)
        .map_err(|e| JsError::new(&format!("Invalid vars JSON: {}", e)))?;
    substitute_template_raw(template, &vars).map_err(|e| JsError::new(&e))
}

/// Extract ${VAR} names from a template and report which are missing from vars.
/// Returns JSON: { required: [...], missing: [...], provided: [...] }
#[wasm_bindgen]
pub fn analyze_template(template: &str, vars_json: &str) -> Result<JsValue, JsError> {
    let vars: HashMap<String, String> = serde_json::from_str(vars_json)
        .map_err(|e| JsError::new(&format!("Invalid vars JSON: {}", e)))?;
    let required = extract_vars(template);
    let mut missing = Vec::new();
    let mut provided = Vec::new();
    for v in &required {
        if vars.contains_key(v) {
            provided.push(v.clone());
        } else {
            missing.push(v.clone());
        }
    }
    #[derive(Serialize)]
    struct Analysis {
        required: Vec<String>,
        missing: Vec<String>,
        provided: Vec<String>,
    }
    let result = Analysis {
        required,
        missing,
        provided,
    };
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// Build a default variable map from all field descriptors (env_var → default value).
/// Returns JSON object. Secret fields are included as empty strings.
#[wasm_bindgen]
pub fn get_default_vars() -> Result<JsValue, JsError> {
    let mut map = HashMap::new();
    for fd in ALL_FDS {
        if fd.secret {
            map.insert(fd.env_var.to_string(), String::new());
        } else {
            map.insert(fd.env_var.to_string(), fd.default.to_string());
        }
    }
    serde_wasm_bindgen::to_value(&map).map_err(|e| JsError::new(&e.to_string()))
}

/// Validate that a JSON config object has all required fields populated.
/// Returns JSON: { valid: bool, empty_fields: [...] }
#[wasm_bindgen]
pub fn validate_config(vars_json: &str) -> Result<JsValue, JsError> {
    let vars: HashMap<String, String> = serde_json::from_str(vars_json)
        .map_err(|e| JsError::new(&format!("Invalid JSON: {}", e)))?;
    let empty: Vec<&str> = ALL_FDS
        .iter()
        .filter(|fd| !fd.secret)
        .filter(|fd| vars.get(fd.env_var).map(|v| v.is_empty()).unwrap_or(true))
        .filter(|fd| fd.default.is_empty()) // only flag fields that have no default
        .map(|fd| fd.env_var)
        .collect();
    #[derive(Serialize)]
    struct Validation {
        valid: bool,
        empty_fields: Vec<String>,
    }
    let result = Validation {
        valid: empty.is_empty(),
        empty_fields: empty.iter().map(|s| s.to_string()).collect(),
    };
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_simple() {
        let mut vars = HashMap::new();
        vars.insert("NAME".into(), "terpd".into());
        vars.insert("PORT".into(), "26657".into());
        let tpl = "binary: ${NAME} on port ${PORT}";
        let result = substitute_template_raw(tpl, &vars).unwrap();
        assert_eq!(result, "binary: terpd on port 26657");
    }

    #[test]
    fn substitute_missing_var() {
        let vars = HashMap::new();
        let tpl = "image: ${MISSING}";
        let err = substitute_template_raw(tpl, &vars).unwrap_err();
        assert!(err.contains("MISSING"));
    }

    #[test]
    fn substitute_unclosed() {
        let vars = HashMap::new();
        let tpl = "image: ${UNCLOSED";
        let err = substitute_template_raw(tpl, &vars).unwrap_err();
        assert!(err.contains("Unclosed"));
    }

    #[test]
    fn extract_vars_deduplicates() {
        let tpl = "${A} ${B} ${A} ${C}";
        let vars = extract_vars(tpl);
        assert_eq!(vars, vec!["A", "B", "C"]);
    }

    #[test]
    fn field_descriptors_not_empty() {
        assert!(!ALL_FDS.is_empty());
        // Check a known field exists.
        assert!(ALL_FDS.iter().any(|fd| fd.env_var == "OLINE_CHAIN_ID"));
    }

    #[test]
    fn phase_infos_complete() {
        let phases = phase_infos();
        assert_eq!(phases.len(), 5);
        assert!(phases.iter().any(|p| p.id == "a"));
        assert!(phases.iter().any(|p| p.id == "b"));
    }
}
