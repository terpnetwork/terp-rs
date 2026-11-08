
// inputs
// total tokens in headstash begin distributed
const BASE_ALLOCATION = 67000000;
// total supply of token
const TOTAL_SUPPLY = 420000000;

// pecentrages of genesis airdrop to gaia & bcna holders
const GAIA_PERC_SUPPLY = 0.061152;
const BCNA_PERC_SUPPLY = 0.01911;

// Default single URL (legacy). Prefer ETH_RPC_URLS / ETH_RPC_MODE in scrape-pubkeys.
const ETH_RPC_URL = process.env.ETH_RPC_URL || "http://127.0.0.1:8545";
// YAML files
const HEADSTASH_TOOL = "../../tools/headstash";
const HEADSTASH_YAML = `${HEADSTASH_TOOL}/headstash.yaml`;
const GENESIS_YAML_FILE = `${HEADSTASH_TOOL}/headstash/scripts-data/final_tally.csv`;

// Exported files
const NETWORK_GENESIS_FILE = `${HEADSTASH_TOOL}/data/genesis.json`
const LIVE_NETWORK_EXPORT_FILE = `${HEADSTASH_TOOL}/data/export.json`

// Genesis Relevant Files
const GENESIS_DISTRIBUTION_FILE = `${HEADSTASH_TOOL}/genesis/scripts-data/final-output.csv`;
const PATCHED_DISTRIBUTION_FILE = `${HEADSTASH_TOOL}/genesis/scripts-data/patched-distribution.csv`
const INACTIVE_ACCOUNT_FILE = `${HEADSTASH_TOOL}/genesis/scripts-data/accounts-inactive.json`
const ACTIVE_ACCOUNTS_FILE = `${HEADSTASH_TOOL}/genesis/scripts-data/accounts-active.json`
const TOKEN_DIFF_OUTPUT = `${HEADSTASH_TOOL}/genesis/scripts-data/token-differences.csv`
const SUMMARY_OUTPUT = `${HEADSTASH_TOOL}/genesis/scripts-data/summary.json`
const POINTS_SUMMARY_FILE = `${HEADSTASH_TOOL}/genesis/scripts-data/points-distribution.csv`;
const TOTAL_POINTS_FILE = `${HEADSTASH_TOOL}/genesis/scripts-data/total-points.csv`;
// Headstash Relevant Files
const SAC_JSON_PATH = `${HEADSTASH_TOOL}/headstash/communities/stoned-ape-club/sac.json`;
const SAC_ENCODED_FILE = `${HEADSTASH_TOOL}/headstash/communities/stoned-ape-club/stoned-ape-club.csv`;
const SCAVENGER_HUNT_FILE = `${HEADSTASH_TOOL}/genesis/scavenger_hunt.csv`;
const TERPOG_FILE = `${HEADSTASH_TOOL}/genesis/terp_og.csv`;
const BCNA_DELEGATORS = `${HEADSTASH_TOOL}/genesis/bcna_delegators.csv`;
const GAIA_DELEGATORS = `${HEADSTASH_TOOL}/genesis/gaia.csv`;
const HEADSTASH_FINAL_TALLY = `${HEADSTASH_TOOL}/headstash/scripts-data/final_tally.csv`;
const SINSEMILLA_JSON_FILE = `${HEADSTASH_TOOL}/zk-crates/data/genesis_sinsemilla.json`;
const OVERVIEW_README = `${HEADSTASH_TOOL}/README.md`;

export {
    TOTAL_SUPPLY,
    GENESIS_YAML_FILE,
    NETWORK_GENESIS_FILE,
    LIVE_NETWORK_EXPORT_FILE,
    SCAVENGER_HUNT_FILE,
    TERPOG_FILE,
    SAC_JSON_PATH,
    SAC_ENCODED_FILE,
    BCNA_DELEGATORS,
    GAIA_DELEGATORS,
    HEADSTASH_YAML,
    HEADSTASH_FINAL_TALLY,
    ACTIVE_ACCOUNTS_FILE,
    TOKEN_DIFF_OUTPUT,
    SUMMARY_OUTPUT,
    PATCHED_DISTRIBUTION_FILE,
    POINTS_SUMMARY_FILE,
    INACTIVE_ACCOUNT_FILE,
    GENESIS_DISTRIBUTION_FILE,
    BCNA_PERC_SUPPLY,
    GAIA_PERC_SUPPLY,
    TOTAL_POINTS_FILE,
    BASE_ALLOCATION,
    SINSEMILLA_JSON_FILE,
    OVERVIEW_README,
    ETH_RPC_URL,
};